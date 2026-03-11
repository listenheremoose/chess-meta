use iced::widget::{button, container, row, text, text_input};
use iced::{Element, Length};

use crate::app::Message;
use crate::coordinator::SearchSnapshot;

/// Top bar: position input, start/pause/reset, live stats.
pub fn view<'a>(
    move_input: &str,
    running: bool,
    snapshot: Option<&SearchSnapshot>,
) -> Element<'a, Message> {
    let input = text_input("Enter moves (e.g. e2e4 e7e5 g1f3)...", move_input)
        .on_input(Message::MoveInputChanged)
        .on_submit(Message::StartSearch)
        .width(Length::FillPortion(3));

    let start_btn = if running {
        button(text("Pause")).on_press(Message::PauseSearch)
    } else {
        button(text("Start")).on_press(Message::StartSearch)
    };

    let reset_btn = button(text("Reset")).on_press(Message::ResetSearch);

    let buttons = row![start_btn, reset_btn].spacing(8);

    let stats = match snapshot {
        Some(snap) => {
            let best = snap
                .best_move
                .as_deref()
                .unwrap_or("-");
            let iter_text = format!(
                "Iter: {} | Nodes: {} | {:.0} it/s | Best: {}",
                snap.iteration,
                snap.node_count,
                snap.iterations_per_sec,
                best,
            );
            text(iter_text).size(14)
        }
        None => text("Ready").size(14),
    };

    let elapsed = snapshot
        .map(|s| format!("{:.1}s", s.elapsed_secs))
        .unwrap_or_default();
    let elapsed_text = text(elapsed).size(14);

    let top_row = row![input, buttons, stats, elapsed_text]
        .spacing(16)
        .align_y(iced::Alignment::Center);

    container(top_row)
        .padding(12)
        .width(Length::Fill)
        .into()
}
