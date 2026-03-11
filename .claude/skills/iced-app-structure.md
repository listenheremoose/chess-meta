---
name: Iced App Structure
description: Entry point, window config, state management, messaging, and styling patterns for the Iced 0.14 application
globs: src/**/*.rs
---

# Iced App Structure

## Entry Point

Use `iced::application()` with the boot/update/view free-function pattern. The first argument is a `BootFn` — a closure or function returning `(State, Task<Message>)` or just `State`:

```rust
fn main() -> iced::Result {
    iced::application(App::new, App::update, App::view)
        .title("chess-meta")
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(1400.0, 900.0))
        .run()
}
```

Note: `.title()` accepts `&'static str` (rendered as-is) or `Fn(&State) -> String`. The `.centered()` method does not exist in iced 0.14 — use `.window_position()` if needed.

## Window

- Set a default size with `.window_size()`
- The window is resizable by default

## State

Single `App` struct holds all state. UI-specific state (canvas caches) lives alongside domain state:

```rust
pub struct App {
    config: Config,
    coordinator: Coordinator,
    move_input: String,
    selected_move: Option<String>,
    tree_view_state: tree_view::TreeViewState,
    progress_state: progress::ProgressState,
}
```

## Messages

Use a flat message enum. Nested per-component message enums add complexity without benefit for this app's scale:

```rust
#[derive(Debug, Clone)]
pub enum Message {
    MoveInputChanged(String),
    StartSearch,
    PauseSearch,
    ResetSearch,
    Tick,
    SelectMove(String),
}
```

## Update

Methods on `App`. Return `Task<Message>` (use `Task::none()` when no async work needed):

```rust
pub fn update(&mut self, message: Message) -> Task<Message> {
    match message {
        Message::StartSearch => { /* ... */ }
        Message::Tick => { /* poll coordinator */ }
        // ...
    }
    Task::none()
}
```

## View

Method on `App`. UI panels are free functions in `ui/` submodules that accept data slices and return `Element`:

```rust
pub fn view(&self) -> Element<'_, Message> {
    let top = controls::view(&self.move_input, self.coordinator.running, snapshot);
    let left = move_table::view(root_moves, self.selected_move.as_deref());
    let right = tree_view::view(tree_snap, &self.tree_view_state, 10, 8);
    let bottom = progress::view(snapshot, &self.progress_state);

    column![top, row![left, right], bottom].into()
}
```

## Subscriptions

Use `iced::time::every` for periodic polling (requires the `tokio` feature on the `iced` crate):

```rust
pub fn subscription(&self) -> Subscription<Message> {
    if self.coordinator.running {
        iced::time::every(Duration::from_millis(100)).map(|_| Message::Tick)
    } else {
        Subscription::none()
    }
}
```

## Styling

Use the built-in `Theme::Dark` theme. Custom colors via module-level constants in `ui/mod.rs`:

```rust
pub mod colors {
    pub const BACKGROUND: Color = Color::from_rgb(0x1E as f32 / 255.0, ...);
    pub const GREEN: Color = ...;
}
```

## Padding

Iced 0.14's `Padding` accepts `u16`, `[u16; 2]`, `f32`, or `[f32; 2]`. Four-sided padding arrays (`[T; 4]`) are not supported — use a single value or construct `Padding` explicitly.

## Canvas

Use `iced::widget::canvas::Cache` for canvas-based panels (tree view, progress strip). Call `.clear()` when the underlying data changes to trigger a redraw.
