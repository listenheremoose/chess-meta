use iced::widget::{column, container, row};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::config::Config;
use crate::coordinator::Coordinator;
use crate::ui::{controls, move_table, progress, tree_view};

/// Messages for the iced application.
#[derive(Debug, Clone)]
pub enum Message {
    MoveInputChanged(String),
    StartSearch,
    PauseSearch,
    ResetSearch,
    Tick,
}

/// Main application state.
pub struct App {
    config: Config,
    coordinator: Coordinator,
    move_input: String,
    selected_move: Option<String>,
    tree_view_state: tree_view::TreeViewState,
    progress_state: progress::ProgressState,
}

impl App {
    pub fn new() -> (Self, Task<Message>) {
        let config = Config::load();
        let mut coordinator = Coordinator::new();
        // Load any persisted search results for the default (starting) position
        coordinator.load_persisted("", &config);
        (
            Self {
                config,
                coordinator,
                move_input: String::new(),
                selected_move: None,
                tree_view_state: tree_view::TreeViewState::default(),
                progress_state: progress::ProgressState::default(),
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub fn subscription(&self) -> Subscription<Message> {
        if self.coordinator.running {
            iced::time::every(std::time::Duration::from_millis(100)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MoveInputChanged(input) => self.move_input = input,
            Message::StartSearch => return self.handle_start_search(),
            Message::PauseSearch => {
                log::info!("Search paused");
                self.coordinator.stop();
            }
            Message::ResetSearch => self.handle_reset_search(),
            Message::Tick => self.handle_tick(),
        }
        Task::none()
    }

    fn handle_start_search(&mut self) -> Task<Message> {
        if !self.config.engine_paths_configured() {
            log::warn!("Engine paths not configured");
            return Task::none();
        }
        log::info!("Search started from UI position={}", if self.move_input.is_empty() { "startpos" } else { &self.move_input });
        self.coordinator
            .start(self.move_input.clone(), self.config.clone());
        self.selected_move = None;
        self.tree_view_state.clear_cache();
        self.progress_state.clear_cache();
        Task::none()
    }

    fn handle_reset_search(&mut self) {
        log::info!("Search reset position={}", if self.move_input.is_empty() { "startpos" } else { &self.move_input });
        self.coordinator.stop();
        self.coordinator.clear_session(&self.move_input);
        self.coordinator.latest_snapshot = None;
        self.selected_move = None;
        self.tree_view_state.clear_cache();
        self.progress_state.clear_cache();
    }

    fn handle_tick(&mut self) {
        if self.coordinator.poll() {
            self.tree_view_state.clear_cache();
            self.progress_state.clear_cache();

            // Auto-select best move if none selected
            if self.selected_move.is_none() {
                if let Some(snap) = &self.coordinator.latest_snapshot {
                    self.selected_move = snap.best_move.clone();
                }
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let snapshot = self.coordinator.latest_snapshot.as_ref();

        // Top bar
        let top = controls::view(&self.move_input, self.coordinator.running, snapshot);

        // Root moves for the table
        let root_moves = match snapshot {
            Some(s) => s.root_moves.as_slice(),
            None => &[],
        };

        // Left panel: move comparison table
        let left_panel = move_table::view(root_moves, self.selected_move.as_deref());

        // Right panel: tree visualization
        let tree_snap = match snapshot {
            Some(s) => s.tree_snapshot.as_ref(),
            None => None,
        };
        let right_panel = tree_view::view(tree_snap, &self.tree_view_state, 10, 8);

        let main_panels = row![
            container(left_panel)
                .width(Length::FillPortion(1))
                .height(Length::Fill),
            container(right_panel)
                .width(Length::FillPortion(1))
                .height(Length::Fill),
        ]
        .spacing(4);

        // Bottom strip: progress
        let bottom = progress::view(snapshot, &self.progress_state);

        let layout = column![top, main_panels, bottom].spacing(4);

        container(layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(4)
            .into()
    }
}
