use iced::widget::{container, row, scrollable, text, Column};
use iced::{Element, Length};

use crate::app::Message;
use crate::search::RootMoveInfo;
use crate::ui::colors;

/// Left panel: move comparison table with candidate moves.
pub fn view<'a>(
    moves: &'a [RootMoveInfo],
    selected_move: Option<&str>,
) -> Element<'a, Message> {
    let header = row![
        text("Move").width(Length::FillPortion(2)).size(13),
        text("Policy").width(Length::FillPortion(2)).size(13),
        text("Pract Q").width(Length::FillPortion(2)).size(13),
        text("Delta").width(Length::FillPortion(2)).size(13),
        text("Visits").width(Length::FillPortion(2)).size(13),
    ]
    .spacing(4)
    .padding(4);

    let mut rows: Vec<Element<'a, Message>> = vec![header.into()];

    moves.iter().for_each(|info| {
        let is_selected = selected_move == Some(&info.uci_move);

        let policy_str = match info.engine_policy {
            Some(p) => format!("{:.1}%", p),
            None => "-".to_string(),
        };

        let delta_str = match info.delta {
            Some(d) => format!("{:+.3}", d),
            None => "-".to_string(),
        };

        let delta_color = match info.delta {
            Some(d) if d > 0.01 => colors::GREEN,
            Some(d) if d < -0.01 => colors::RED,
            Some(_) => colors::TEXT_DIM,
            None => colors::TEXT,
        };

        let move_row = row![
            text(&info.uci_move).width(Length::FillPortion(2)).size(13),
            text(policy_str).width(Length::FillPortion(2)).size(13),
            text(format!("{:.3}", info.practical_q))
                .width(Length::FillPortion(2))
                .size(13),
            text(delta_str)
                .width(Length::FillPortion(2))
                .size(13)
                .color(delta_color),
            text(format!("{}", info.visits))
                .width(Length::FillPortion(2))
                .size(13),
        ]
        .spacing(4)
        .padding(4);

        rows.push(
            container(move_row)
                .style(move |_theme: &iced::Theme| {
                    if is_selected {
                        container::Style {
                            background: Some(iced::Background::Color(colors::SURFACE)),
                            ..Default::default()
                        }
                    } else {
                        container::Style::default()
                    }
                })
                .into(),
        );

        // Detail view for selected move
        if is_selected {
            match move_detail(info) {
                Some(detail) => rows.push(detail),
                None => {}
            }
        }
    });

    let content = Column::with_children(rows).spacing(2);

    container(scrollable(content))
        .padding(8)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Detail panel for a selected move.
fn move_detail<'a>(info: &RootMoveInfo) -> Option<Element<'a, Message>> {
    let mut details = Vec::new();

    details.push(
        text(format!("Q (White): {:.4}", info.q_white))
            .size(12)
            .into(),
    );
    details.push(
        text(format!("Worst-case: {:.4}", info.worst_case))
            .size(12)
            .into(),
    );

    match info.wdl {
        Some((w, d, l)) => {
            let total = (w + d + l) as f64;
            if total > 0.0 {
                let wdl_str = format!(
                    "WDL: {:.1}% / {:.1}% / {:.1}%",
                    w as f64 / total * 100.0,
                    d as f64 / total * 100.0,
                    l as f64 / total * 100.0,
                );
                details.push(text(wdl_str).size(12).into());
            }
        }
        None => {}
    }

    let detail_col = Column::with_children(details)
        .spacing(2)
        .padding(4);

    Some(
        container(detail_col)
            .style(|_theme: &iced::Theme| container::Style {
                background: Some(iced::Background::Color(colors::SURFACE)),
                ..Default::default()
            })
            .into(),
    )
}
