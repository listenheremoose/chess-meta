---
name: Iced App Structure
description: Entry point, window config, state management, messaging, and styling patterns for the Iced application
globs: src/**/*.rs
---

# Iced App Structure

## Entry Point

Use `iced::application()` builder with free functions (not trait impl):

```rust
fn main() -> iced::Result {
    iced::application(boot, update, view)
        .title("Chess Meta")
        .theme(theme)
        .window_size((1200.0, 800.0))
        .resizable(true)
        .centered()
        .run()
}

fn boot() -> (ChessMeta, Task<Message>) {
    (ChessMeta::default(), Task::none())
}

fn theme(_state: &ChessMeta) -> Theme {
    Theme::Dark
}
```

## Window

- Set a minimum size but allow free resizing (`.window_size()` + `.resizable(true)`)
- Center on launch (`.centered()`)

## State

Shared domain state lives at the top level. Components only own UI-specific state:

```rust
#[derive(Default)]
struct ChessMeta {
    search: SearchState,        // shared MCTS data (tree, metrics, config)
    controls: Controls,         // top bar UI state
    move_table: MoveTable,      // left panel UI state
    tree_view: TreeView,        // right panel UI state
    progress: SearchProgress,   // bottom strip UI state
}
```

Each component struct lives in its own module file under `src/ui/`.

## Messages

Use nested message enums. The top-level `Message` wraps per-component message types:

```rust
#[derive(Debug, Clone)]
enum Message {
    Controls(ControlsMessage),
    MoveTable(MoveTableMessage),
    TreeView(TreeViewMessage),
    Progress(ProgressMessage),
    SearchTick,  // periodic poll for MCTS updates
}
```

Each component defines its own message enum:

```rust
#[derive(Debug, Clone)]
pub enum ControlsMessage {
    PositionChanged(String),
    StartSearch,
    PauseSearch,
    ResetSearch,
}
```

## Update

Route messages to components. Components receive `&mut` to shared state as needed. Use returned actions for cross-component side effects:

```rust
fn update(state: &mut ChessMeta, message: Message) {
    match message {
        Message::Controls(message) => handle_controls_update(message, &mut state.controls, &mut state.search),
        Message::MoveTable(message) => state.move_table.update(message),
        Message::TreeView(message) => state.tree_view.update(message),
        Message::SearchTick => poll_search_state(&mut state.search),
        Message::Progress(message) => state.progress.update(message),
    }
}

fn handle_controls_update(message: ControlsMessage, controls: &mut Controls, search: &mut SearchState) {
    let action = controls.update(message);
    match action {
        Some(ControlsAction::StartSearch(position)) => search.start(position),
        Some(ControlsAction::Pause) => search.pause(),
        None => {}
    }
}
```

## View

Each component has a `view()` method returning `Element<'_, ComponentMessage>`. Components receive `&` to shared state as needed. The top-level `view` maps them to the parent `Message`:

```rust
fn view(state: &ChessMeta) -> Element<'_, Message> {
    let controls = state.controls.view(&state.search).map(Message::Controls);
    let move_table = state.move_table.view(&state.search).map(Message::MoveTable);
    let tree_view = state.tree_view.view(&state.search).map(Message::TreeView);
    let progress = state.progress.view(&state.search).map(Message::Progress);

    let main_panels = row![move_table, tree_view];
    column![controls, main_panels, progress].into()
}
```

## Styling

Use the built-in `Theme::Dark` theme. No custom stylesheet traits.

## Error Handling

Use custom error enums per module (see error-handling skill). Surface errors in the UI via messages (e.g., a status bar or dialog), not panics.
