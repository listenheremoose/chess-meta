---
name: Testing
description: Test conventions, structure, and coverage strategy
globs: tests/**/*.rs
---

# Testing

## Test Location

All tests live in the `tests/` directory — no inline `#[cfg(test)]` modules.

## Test Naming

Name tests as `<scenario>_<expected>`:

```rust
#[test]
fn pinned_piece_cannot_move() { ... }

#[test]
fn castling_clears_rook_square() { ... }
```

## Assertions

Standard library only — `assert!`, `assert_eq!`, `assert_ne!`. No assertion crates.

## Test Setup

Use the builder pattern for constructing test state:

```rust
let board = BoardBuilder::new()
    .with_piece(King, White, E1)
    .with_piece(Rook, Black, E8)
    .build();
```

## Board State

Express positions programmatically, not with FEN strings:

```rust
// Yes
board.place(King, E1);

// Avoid
Board::from_fen("8/8/8/8/8/8/8/4K3 w - - 0 1")
```

## Test Ordering

Group tests by scenario/feature. Within each group, failure cases first, then successes:

```rust
// -- Castling --

#[test]
fn castling_blocked_by_check_fails() { ... }

#[test]
fn castling_through_attacked_square_fails() { ... }

#[test]
fn castling_kingside_moves_both_pieces() { ... }

#[test]
fn castling_queenside_moves_both_pieces() { ... }
```

## Test Scope

Test at all levels:

- **Unit tests** — core logic: move generation, check/checkmate, game rules
- **Integration tests** — full game flow: multi-move sequences, complete games
- **Snapshot tests** — use `insta` to capture board rendering, move lists, and complex state; commit `.snap` files to version control

## Coverage

Maximize test coverage. When adding or modifying logic, add tests for every reachable code path — happy paths, edge cases, and error cases.
