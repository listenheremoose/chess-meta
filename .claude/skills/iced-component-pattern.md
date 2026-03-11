---
name: Iced Component Pattern
description: Template and checklist for creating new Iced UI components
globs: src/**/*.rs
---

# Iced Component Pattern

When creating a new component, follow this template. Each component lives in its own file under `src/`.

## Template

Components only hold UI-specific state. Shared domain state (like `Game`) is passed in via `update` and `view` parameters.

```rust
use iced::widget::text;
use iced::Element;

#[derive(Debug, Clone)]
pub enum FooMessage {
    // Component-specific messages
}

// Optional: define actions for cross-component side effects
pub enum FooAction {
    // Actions the parent should handle
}

#[derive(Default)]
pub struct Foo {
    // UI-specific state only (not domain data)
}

impl Foo {
    // Accept &mut Game (or other shared state) if the component needs to read/write it
    pub fn update(&mut self, message: FooMessage) -> Option<FooAction> {
        match message {
            // Handle messages, return Some(action) for cross-component effects
        }
        None
    }

    // Accept &Game (or other shared state) if the component needs to display it
    pub fn view(&self) -> Element<'_, FooMessage> {
        text("Foo").into()
    }
}
```

## Wiring into the app

1. Add `mod foo;` to `main.rs`
2. Add `use foo::{Foo, FooMessage};` import
3. Add `foo: Foo` field to `ChessMeta`
4. Add `Foo(FooMessage)` variant to `Message`
5. Add routing arm in `update()` — pass `&mut state.game` if needed, handle returned actions
6. Add `state.foo.view().map(Message::Foo)` in `view()` — pass `&state.game` if needed

## Rust 2024 edition note

Always use explicit `'_` lifetime in view return types: `Element<'_, FooMessage>` not `Element<FooMessage>`.
