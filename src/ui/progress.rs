use iced::mouse;
use iced::widget::canvas::{self, Canvas, Frame, Geometry, Path, Stroke, Text};
use iced::{Color, Element, Length, Point, Rectangle, Renderer, Size, Theme};

use crate::app::Message;
use crate::coordinator::SearchSnapshot;
use crate::ui::colors;

/// State for the progress strip canvas.
#[derive(Default)]
pub struct ProgressState {
    cache: canvas::Cache,
}

impl ProgressState {
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

/// Bottom strip: best move timeline, Q convergence sparkline, iter/sec.
pub fn view<'a>(
    snapshot: Option<&SearchSnapshot>,
    state: &'a ProgressState,
) -> Element<'a, Message> {
    let snap_clone = snapshot.cloned();
    Canvas::new(ProgressProgram {
        snapshot: snap_clone,
        cache: &state.cache,
    })
    .width(Length::Fill)
    .height(Length::Fixed(80.0))
    .into()
}

struct ProgressProgram<'a> {
    snapshot: Option<SearchSnapshot>,
    cache: &'a canvas::Cache,
}

impl<'a> canvas::Program<Message> for ProgressProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            frame.fill_rectangle(Point::ORIGIN, bounds.size(), colors::SURFACE);

            let snapshot = match &self.snapshot {
                Some(s) => s,
                None => return,
            };

            let half_w = bounds.width / 2.0;

            // Left half: Best move timeline
            draw_best_move_timeline(frame, snapshot, half_w, bounds.height);

            // Right half: Q convergence sparkline
            draw_q_sparkline(frame, snapshot, half_w, bounds.width, bounds.height);

            // Iterations/sec text
            let ips_text = Text {
                content: format!("{:.0} it/s", snapshot.iterations_per_sec),
                position: Point::new(bounds.width - 60.0, 4.0),
                color: colors::TEXT_DIM,
                size: iced::Pixels(11.0),
                ..Text::default()
            };
            frame.fill_text(ips_text);
        });

        vec![geometry]
    }
}

fn draw_best_move_timeline(
    frame: &mut Frame,
    snapshot: &SearchSnapshot,
    width: f32,
    height: f32,
) {
    let history = &snapshot.best_move_history;
    if history.is_empty() {
        return;
    }

    let label = Text {
        content: "Best move over time".to_string(),
        position: Point::new(4.0, 4.0),
        color: colors::TEXT_DIM,
        size: iced::Pixels(10.0),
        ..Text::default()
    };
    frame.fill_text(label);

    let max_iter = snapshot.iteration as f32;
    let band_y = 20.0;
    let band_h = height - 24.0;

    // Assign colors to moves
    let move_colors = [colors::ACCENT, colors::BLUE, colors::ORANGE, colors::GREEN, colors::RED];

    history.windows(2).enumerate().for_each(|(i, window)| {
        let start_x = window[0].0 as f32 / max_iter * width;
        let end_x = window[1].0 as f32 / max_iter * width;
        let color = move_colors[i % move_colors.len()];

        frame.fill_rectangle(
            Point::new(start_x, band_y),
            Size::new(end_x - start_x, band_h),
            color,
        );
    });

    // Last segment to current iteration
    match history.last() {
        Some(last) => {
            let start_x = last.0 as f32 / max_iter * width;
            let color = move_colors[(history.len() - 1) % move_colors.len()];
            frame.fill_rectangle(
                Point::new(start_x, band_y),
                Size::new(width - start_x, band_h),
                color,
            );

            // Label the current best move
            let label = Text {
                content: last.1.clone(),
                position: Point::new(start_x + 2.0, band_y + 2.0),
                color: Color::WHITE,
                size: iced::Pixels(11.0),
                ..Text::default()
            };
            frame.fill_text(label);
        }
        None => {}
    }
}

fn draw_q_sparkline(
    frame: &mut Frame,
    snapshot: &SearchSnapshot,
    start_x: f32,
    total_width: f32,
    height: f32,
) {
    let q_history = &snapshot.q_history;
    if q_history.len() < 2 {
        return;
    }

    let label = Text {
        content: "Q convergence".to_string(),
        position: Point::new(start_x + 4.0, 4.0),
        color: colors::TEXT_DIM,
        size: iced::Pixels(10.0),
        ..Text::default()
    };
    frame.fill_text(label);

    let spark_width = total_width - start_x - 70.0; // Leave room for iter/sec
    let spark_y = 20.0;
    let spark_h = height - 24.0;

    let min_q = q_history
        .iter()
        .map(|(_, q)| *q)
        .fold(f64::MAX, f64::min);
    let max_q = q_history
        .iter()
        .map(|(_, q)| *q)
        .fold(f64::MIN, f64::max);
    let q_range = (max_q - min_q).max(0.01);

    let max_iter = snapshot.iteration as f64;

    let points: Vec<Point> = q_history
        .iter()
        .map(|(iter, q)| {
            let x = start_x + (*iter as f64 / max_iter) as f32 * spark_width;
            let y = spark_y + spark_h - ((q - min_q) / q_range) as f32 * spark_h;
            Point::new(x, y)
        })
        .collect();

    // Draw line segments
    points.windows(2).for_each(|window| {
        let line = Path::line(window[0], window[1]);
        frame.stroke(
            &line,
            Stroke::default()
                .with_color(colors::ACCENT)
                .with_width(1.5),
        );
    });

    // Current Q value label
    match q_history.last() {
        Some(last) => {
            let q_label = Text {
                content: format!("{:.3}", last.1),
                position: Point::new(start_x + spark_width + 4.0, spark_y + spark_h / 2.0),
                color: colors::TEXT,
                size: iced::Pixels(11.0),
                ..Text::default()
            };
            frame.fill_text(q_label);
        }
        None => {}
    }
}
