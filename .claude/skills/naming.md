---
name: Naming Conventions
description: Variable, function, and type naming style
globs: src/**/*.rs
---

# Naming Conventions

Be verbose and descriptive. Readability over brevity.

## Variables

Always use full descriptive names:

```rust
let selected_piece = board.get(position);
let legal_moves = generate_legal_moves(&board, selected_piece);
```

## Iterator Variables

Descriptive even in closures:

```rust
pieces.iter().filter(|piece| piece.color == Color::White)
squares.iter().map(|square| square.piece())
```

## Index Variables

Full descriptive names, no abbreviations:

```rust
board[row][column]
for (row_index, column_index) in positions { ... }
```

## Booleans

Read as natural English, no `is_`/`has_` prefix required:

```rust
let in_checkmate = check_for_checkmate(&board);
let castling_available = king.can_castle();
```

## No Abbreviations

Never abbreviate. Spell out fully:

```rust
let position = (row, column);
let destination = calculate_destination(piece, direction);

// Not: pos, col, src, dst
```

## Function Names

Descriptive and natural, not forced into verb-first. The name should clearly convey what the function does or returns:

```rust
fn legal_moves_for_piece(board: &Board, piece: &Piece) -> Vec<Move>
fn board_after_move(board: &Board, chess_move: &Move) -> Board
fn pieces_attacking_square(board: &Board, square: Position) -> Vec<Piece>
```

## Type / Struct Names

Concise, with context implied by the module:

```rust
struct Board { ... }        // in chess module, obviously a chess board
enum Color { White, Black }
struct History { ... }
```
