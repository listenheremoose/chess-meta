use iced::widget::{column, container, row};
use iced::{Element, Length, Subscription, Task, Theme};

use crate::config::Config;
use crate::coordinator::Coordinator;
use crate::ui::{controls, move_table, progress, tree_view};

/// Messages for the iced application.
#[derive(Debug, Clone)]
pub enum Message {
    MoveInputChanged(String),
    MoveInputSettled,
    StartSearch,
    PauseSearch,
    ResetSearch,
    Tick,
    SelectMove(String),
}

/// Main application state.
pub struct App {
    config: Config,
    coordinator: Coordinator,
    move_input: String,
    selected_move: Option<String>,
    tree_view_state: tree_view::TreeViewState,
    progress_state: progress::ProgressState,
    /// Incremented on each keystroke; used to debounce DB lookups.
    move_input_generation: u64,
    /// Generation at which we last ran load_persisted for the input field.
    last_settled_generation: u64,
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
                move_input_generation: 0,
                last_settled_generation: 0,
            },
            Task::none(),
        )
    }

    pub fn theme(&self) -> Theme {
        Theme::KanagawaDragon
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let tick = if self.coordinator.running {
            iced::time::every(std::time::Duration::from_millis(100)).map(|_| Message::Tick)
        } else {
            Subscription::none()
        };

        // Fire a debounce message 300 ms after the last keystroke while idle.
        let debounce = if !self.coordinator.running
            && self.move_input_generation != self.last_settled_generation
        {
            iced::time::every(std::time::Duration::from_millis(300))
                .map(|_| Message::MoveInputSettled)
        } else {
            Subscription::none()
        };

        Subscription::batch([tick, debounce])
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::MoveInputChanged(input) => {
                self.move_input = input;
                self.move_input_generation += 1;
            }
            Message::MoveInputSettled => {
                self.last_settled_generation = self.move_input_generation;
                self.coordinator.latest_snapshot = None;
                self.coordinator.load_persisted(&self.move_input, &self.config);
                self.selected_move = None;
                self.tree_view_state.clear_cache();
                self.progress_state.clear_cache();
            }
            Message::StartSearch => return self.handle_start_search(),
            Message::PauseSearch => self.handle_pause_search(),
            Message::ResetSearch => self.handle_reset_search(),
            Message::Tick => self.handle_tick(),
            Message::SelectMove(mv) => self.selected_move = Some(mv),
        }
        Task::none()
    }

    fn handle_pause_search(&mut self) {
        log::info!("Search paused");
        self.coordinator.stop();
    }

    fn handle_start_search(&mut self) -> Task<Message> {
        if !self.config.engine_paths_configured() {
            log::warn!("Engine paths not configured");
            return Task::none();
        }
        log::info!("Search started from UI position={}", if self.move_input.is_empty() { "startpos" } else { &self.move_input });
        // Mark input as settled so the debounce doesn't re-trigger after the search ends.
        self.last_settled_generation = self.move_input_generation;
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
            match &self.selected_move {
                None => match &self.coordinator.latest_snapshot {
                    Some(snap) => self.selected_move = snap.best_move.clone(),
                    None => {}
                },
                Some(_) => {}
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let snapshot = self.coordinator.latest_snapshot.as_ref();

        let top = controls::view(&self.move_input, self.coordinator.running, snapshot);

        let root_moves = match snapshot {
            Some(s) => s.root_moves.as_slice(),
            None => &[],
        };

        let left_panel = move_table::view(root_moves, self.selected_move.as_deref());

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

        let bottom = progress::view(snapshot, &self.progress_state);

        let layout = column![top, main_panels, bottom].spacing(4);

        container(layout)
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(4)
            .into()
    }
}
