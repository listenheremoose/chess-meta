---
name: Functional Patterns
description: Functional programming conventions and iterator usage
globs: src/**/*.rs
---

# Functional Patterns

Prefer functional style as much as possible.

## Philosophy

Transform data functionally, but make decisions visibly. Use iterator chains for data transformation, but use `match` for control flow rather than hiding branching logic inside Option/Result combinators.

## Iterators Over Loops

Always use iterator chains, never `for` loops:

```rust
// Yes
let white_pieces = pieces.iter()
    .filter(|piece| piece.color == Color::White)
    .collect::<Vec<_>>();

// No
let mut white_pieces = Vec::new();
for piece in &pieces {
    if piece.color == Color::White {
        white_pieces.push(piece);
    }
}
```

## Option/Result Handling

Use `match` over combinators like `map`, `and_then`, `unwrap_or` (consistent with pattern matching preference):

```rust
// Yes
let piece_name = match board.get(row, column) {
    Some(piece) if piece.color == turn => piece.name(),
    _ => "empty",
};

// No
let piece_name = board.get(row, column)
    .filter(|piece| piece.color == turn)
    .map(|piece| piece.name())
    .unwrap_or("empty");
```

## Function Composition

Prefer point-free style where possible. When closures are needed, use descriptive variable names (see naming skill):

```rust
// Best — point-free method reference
pieces.iter().filter(Piece::is_white).map(Piece::name)

// OK — closure with descriptive name when logic requires it
pieces.iter().filter(|piece| piece.color == Color::White)
```

## Fold and Accumulation

Use specialized methods (`sum`, `count`, `any`, `all`) when available. Use `fold` for custom accumulation:

```rust
// Specialized
let score: i32 = pieces.iter()
    .map(Piece::value)
    .sum();

let king_present = pieces.iter()
    .any(|piece| piece.kind == King);

// Custom accumulation with fold
let material_balance = pieces.iter()
    .fold(0, |accumulator, piece| match piece.color {
        Color::White => accumulator + piece.value(),
        Color::Black => accumulator - piece.value(),
    });
```

## Immutability

Default immutable, only `mut` when required:

```rust
let board = Board::new();
let mut turn = Color::White;  // only when mutation is needed
```

## Chain Length

Break into named intermediate variables after 3-4 steps:

```rust
let my_pieces = board.squares()
    .filter(Square::has_piece)
    .filter(|square| square.piece().color == turn);

let result = my_pieces
    .flat_map(|square| square.piece().legal_moves(&board))
    .filter(|chess_move| !chess_move.leaves_king_in_check(&board))
    .collect::<Vec<_>>();
```
