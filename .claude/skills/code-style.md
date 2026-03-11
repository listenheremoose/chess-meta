---
name: Code Style
description: Formatting and syntax conventions
globs: src/**/*.rs
---

# Code Style

## Imports

Use granular imports — import specific items, no wildcards:

```rust
use iced::widget::{button, column, row, text};
```

## Enum Variant Imports

Import specific variants, no wildcards:

```rust
use NodeType::{Max, Chance};
use SearchStatus::{Idle, Running, Paused};
```

## String Formatting

Prefer inline format strings with `format!()` and `text()`:

```rust
text(format!("Iterations: {iteration_count}"))
```

## Match Arms

Keep arms single-line. Factor multi-line logic into separate functions (see pattern-matching skill):

```rust
match message {
    Msg::StartSearch => start_search(state),
    Msg::PauseSearch => pause_search(state),
}
```

## Builder Chains

One method per line:

```rust
column![controls, main_panels, progress]
    .spacing(10)
    .padding(20)
    .align_x(Center)
```

## Method Chaining Newlines

First method on a new line:

```rust
children
    .iter()
    .filter(TreeNode::is_expanded)
    .count()
```

## Return Expressions

Implicit return — no `return` keyword, no trailing semicolon:

```rust
fn value_for_backprop(wdl: (u32, u32, u32), contempt: f64) -> f64 {
    let (win, draw, loss) = wdl;
    win as f64 / 1000.0 + contempt * draw as f64 / 1000.0
}
```

## Trailing Commas

Always use trailing commas in multi-line constructs.

## Blank Lines

Blank line between every function/impl item.

## Generics

Always use where clauses on separate lines:

```rust
fn process<T>(item: T) -> Result<T>
where
    T: Clone + Debug,
{
    // ...
}
```

## Nested Function Calls

Extract to variables when nesting more than 2 levels:

```rust
let action = state.controls.update(message);
handle_controls_action(action, &mut state.search);
```
