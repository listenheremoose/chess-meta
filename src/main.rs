mod app;
mod cache;
mod config;
mod coordinator;
mod engine;
mod maia;
mod position;
mod search;
mod ui;

use iced::Size;

fn main() -> iced::Result {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Info, simplelog::Config::default())
        .ok();

    iced::application(app::App::new, app::App::update, app::App::view)
        .title("chess-meta")
        .subscription(app::App::subscription)
        .theme(app::App::theme)
        .window_size(Size::new(1400.0, 900.0))
        .run()
}
