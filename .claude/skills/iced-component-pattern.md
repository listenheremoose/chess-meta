---
name: Iced Component Pattern
description: Template and checklist for creating UI panels in Iced 0.14
user_invocable: true
globs: src/ui/**/*.rs
---

# Iced Component Pattern

UI panels are implemented as free functions in modules under `src/ui/`. Each module file corresponds to one panel.

## Template — Simple Panel (widgets only)

For panels that just render widgets (controls, move table):

```rust
// src/ui/foo.rs
use iced::widget::{column, text};
use iced::Element;

use crate::app::Message;

pub fn view<'a>(data: &'a SomeData, selected: Option<&str>) -> Element<'a, Message> {
    column![
        text("Header"),
        // ... build widgets from data ...
    ]
    .into()
}
```

## Template — Canvas Panel (custom drawing)

For panels that use custom canvas rendering (tree view, progress strip):

```rust
// src/ui/bar.rs
use iced::widget::canvas::{self, Canvas, Geometry, Path, Text};
use iced::{Element, Length, Point, Rectangle, Renderer, Theme};

use crate::app::Message;

#[derive(Default)]
pub struct BarState {
    cache: canvas::Cache,
}

impl BarState {
    pub fn clear_cache(&mut self) {
        self.cache.clear();
    }
}

pub fn view<'a>(
    data: Option<&SomeSnapshot>,
    state: &'a BarState,
) -> Element<'a, Message> {
    let data_clone = data.cloned();
    Canvas::new(BarProgram {
        data: data_clone,
        cache: &state.cache,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

struct BarProgram<'a> {
    data: Option<SomeSnapshot>,
    cache: &'a canvas::Cache,
}

impl<'a> canvas::Program<Message> for BarProgram<'a> {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let geometry = self.cache.draw(renderer, bounds.size(), |frame| {
            // ... custom drawing ...
        });
        vec![geometry]
    }
}
```

## Wiring into the app

1. Add `pub mod foo;` to `src/ui/mod.rs`
2. Call `foo::view(...)` in `App::view()`, passing data by reference
3. For canvas panels: add the state struct as a field on `App`, call `.clear_cache()` when data updates

## Key conventions

- View functions accept `&'a` references to data and return `Element<'a, Message>`
- All messages flow through the single top-level `Message` enum
- Canvas state structs own a `canvas::Cache` and expose `clear_cache()`
- Clone snapshot data into canvas programs (they can't borrow across the draw boundary)

## Rust 2024 edition note

Always use explicit `'_` lifetime in return types: `Element<'_, Message>` not `Element<Message>`.
