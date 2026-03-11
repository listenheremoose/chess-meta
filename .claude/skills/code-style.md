---
name: Code Style
description: Rust code style conventions for the project
globs: src/**/*.rs
---

# Code Style

## Imports

Use granular imports — import specific items, no wildcards:

```rust
use iced::widget::{button, column, row, text};
```

## String Formatting

Prefer inline format strings and `text!` macro:

```rust
text!("Turn: {turn}")
```

## Match Arms

Keep arms single-line. Factor multi-line logic into separate functions (see pattern-matching skill):

```rust
match message {
    Msg::Click => handle_click(state),
    Msg::Reset => reset_game(state),
}
```

## Builder Chains

One method per line:

```rust
column![board, menu]
    .spacing(10)
    .padding(20)
    .align_x(Center)
```

## Type Annotations

### Local Variables

Annotate only when the type isn't obvious from the right-hand side:

```rust
let selected: Option<(usize, usize)> = None;  // annotate — not obvious
let count = pieces.len();                       // skip — clearly usize
```

### Closures

Let the compiler infer:

```rust
let white_piece = |piece| piece.color == Color::White;
```

### Collect / Turbofish

Use turbofish on the method:

```rust
let moves = legal_moves.collect::<Vec<Move>>();
```

### Complex Types

Use named types / type aliases for complex types:

```rust
type Board = [[Option<Piece>; 8]; 8];
```
