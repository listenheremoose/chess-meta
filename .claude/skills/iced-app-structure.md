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
        .window_size((800.0, 600.0))
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
    game: Game,        // shared domain data
    board_ui: BoardUi, // UI state (selection, highlights, etc.)
    menu: Menu,        // UI state
    engine: Engine,    // engine state
}
```

Each component struct lives in its own module file (`board_ui.rs`, `menu.rs`, etc.).

## Messages

Use nested message enums. The top-level `Message` wraps per-component message types:

```rust
#[derive(Debug, Clone)]
enum Message {
    Board(BoardMessage),
    Menu(MenuMessage),
    Engine(EngineMessage),
}
```

Each component defines its own message enum:

```rust
#[derive(Debug, Clone)]
pub enum BoardMessage {
    SquareClicked(usize, usize),
}
```

## Update

Route messages to components. Components receive `&mut` to shared state as needed. Use returned actions for cross-component side effects:

```rust
fn update(state: &mut ChessMeta, message: Message) {
    match message {
        Message::Board(message) => state.board_ui.update(message, &mut state.game),
        Message::Menu(message) => handle_menu_update(message, &mut state.menu, &mut state.game),
        Message::Engine(message) => state.engine.update(message),
    }
}

fn handle_menu_update(message: MenuMessage, menu: &mut Menu, game: &mut Game) {
    let action = menu.update(message);
    match action {
        Some(MenuAction::NewGame) => game.reset(),
        None => {}
    }
}
```

## View

Each component has a `view()` method returning `Element<'_, ComponentMessage>`. Components receive `&` to shared state as needed. The top-level `view` maps them to the parent `Message`:

```rust
fn view(state: &ChessMeta) -> Element<'_, Message> {
    let board = state.board_ui.view(&state.game).map(Message::Board);
    let menu = state.menu.view().map(Message::Menu);
    row![board, menu].into()
}
```

## Styling

Use the built-in `Theme::Dark` theme. No custom stylesheet traits.

## Error Handling

Use `thiserror` for typed error enums. Surface errors in the UI via messages (e.g., a status bar or dialog), not panics.
