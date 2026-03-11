---
name: Pattern Matching
description: Pattern matching conventions and preferences
globs: src/**/*.rs
---

# Pattern Matching

Pattern matching is the preferred control flow style in this project. Use it aggressively.

## Keep Match Arms Concise

Factor multi-line logic into separate functions so match statements stay readable at a glance:

```rust
// Yes — each arm is a single call, control flow is clear
match message {
    Msg::Click(position) => handle_click(position, &mut board),
    Msg::Promote(piece) => handle_promotion(piece, &mut board),
    Msg::Castle(side) => handle_castle(side, &mut board),
}

// No — inline multi-line logic obscures the match structure
match message {
    Msg::Click(position) => {
        let piece = board.get(position);
        if piece.color == turn {
            selected = Some(position);
            highlights = legal_moves(position, &board);
        }
    }
    Msg::Promote(piece) => {
        board.set(position, piece);
        turn = turn.opposite();
        check_game_over(&board);
    }
    // ...
}
```

## Destructuring

Destructure as much as possible:

```rust
let Piece { color, kind, has_moved } = piece;
```

## Nested Patterns

Flatten with nested patterns rather than matching then destructuring:

```rust
match state {
    Game { board, turn: Color::White, .. } => handle_white(board),
    Game { board, turn: Color::Black, .. } => handle_black(board),
}
```

## Guards

Prefer match guards over nested matches:

```rust
match piece {
    Some(piece) if piece.color == turn => select(piece),
    Some(_) => show_error(),
    None => deselect(),
}
```

## Always Use `match`

Prefer `match` over `if let`, `let-else`, and `matches!`. Always use full `match` even for single-variant checks or boolean pattern tests:

```rust
// Yes — full match
match selected {
    Some(piece) => move_piece(piece),
    None => {}
}

// No — if let
if let Some(piece) = selected {
    move_piece(piece);
}

// Yes — full match for early return
match selected {
    Some(piece) => piece,
    None => return,
}

// No — let-else
let Some(piece) = selected else { return };

// Yes — full match for boolean checks
let major_piece = match piece.kind {
    Rook | Queen => true,
    _ => false,
};

// No — matches! macro
let major_piece = matches!(piece.kind, Rook | Queen);
```

## Or-Patterns

Group related variants freely:

```rust
match piece.kind {
    Rook | Queen => can_move_straight(from, to),
    Bishop | Queen => can_move_diagonal(from, to),
    _ => false,
}
```

## Tuple and Slice Patterns

Use freely and prefer them when possible for coordinates, state combos, and sequences:

```rust
// Coordinate matching
match (row, column) {
    (0, 0) | (0, 7) | (7, 0) | (7, 7) => corner_square(),
    (0, _) | (7, _) => edge_row(),
    (_, 0) | (_, 7) => edge_column(),
    _ => inner_square(),
}

// State combinations
match (selected, clicked) {
    (None, Some(piece)) => select(piece),
    (Some(from), Some(to)) => try_move(from, to),
    (Some(_), None) => deselect(),
    (None, None) => {}
}

// Binding within tuples
match (piece.kind, to_row) {
    (Pawn, 0 | 7) => promote(piece),
    (Pawn, row) if (row as i8 - from_row as i8).abs() == 2 => en_passant_possible(),
    _ => normal_move(),
}

// Slice patterns
match moves.as_slice() {
    [] => text("No moves yet"),
    [only] => text!("1. {only}"),
    [.., last] => text!("Last move: {last}"),
}
```
