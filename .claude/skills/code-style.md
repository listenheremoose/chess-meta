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
use PieceKind::{King, Queen, Rook, Bishop, Knight, Pawn};
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

## Method Chaining Newlines

First method on a new line:

```rust
pieces
    .iter()
    .filter(Piece::is_white)
    .count()
```

## Return Expressions

Implicit return — no `return` keyword, no trailing semicolon:

```rust
fn piece_value(kind: PieceKind) -> i32 {
    match kind {
        Pawn => 100,
        Knight => 300,
        Bishop => 300,
        Rook => 500,
        Queen => 900,
        King => 0,
    }
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
let action = state.menu.update(message);
handle_menu_action(action, &mut state.game);
```
