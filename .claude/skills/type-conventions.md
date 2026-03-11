---
name: Type Conventions
description: Type annotations, aliases, derives, and visibility
globs: src/**/*.rs
---

# Type Conventions

## Type Annotations — Local Variables

Annotate only when the type isn't obvious from the right-hand side:

```rust
let selected: Option<(usize, usize)> = None;  // annotate — not obvious
let count = pieces.len();                       // skip — clearly usize
```

## Type Annotations — Closures

Let the compiler infer:

```rust
let white_piece = |piece| piece.color == Color::White;
```

## Collect / Turbofish

Use turbofish on the method:

```rust
let moves = legal_moves.collect::<Vec<Move>>();
```

## Complex Types

Use named types / type aliases for complex types:

```rust
type Board = [[Option<Piece>; 8]; 8];
```

## Struct Instantiation

Use `..Default::default()` for partial initialization:

```rust
let game = Game {
    turn: Color::White,
    ..Default::default()
};
```

## `Self` vs Type Name

Use the concrete type name, not `Self`:

```rust
impl Board {
    fn new() -> Board { ... }
}
```

## String Types

`&str` for parameters, `String` for owned fields.

## Derive Order

By category — std traits first, then external:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
```

## Visibility

Minimal — only `pub` what's needed outside the module.

## Constants

Use `const` at module level, no magic numbers:

```rust
const BOARD_SIZE: usize = 8;
const STARTING_FEN: &str = "rnbqkbnr/...";
```
