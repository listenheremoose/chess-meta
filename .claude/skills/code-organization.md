---
name: Code Organization
description: Function length, file length, comments, and iteration
globs: src/**/*.rs
---

# Code Organization

## Function Length

Soft limit of ~40 lines (one screen). If it scrolls past one screen, split it.

Split when:
- A function does more than one logical step
- You'd want to add a comment explaining a block — make it a named function instead

## File Length

Keep files under ~200 lines. When a file grows past this, split into a directory module:

```
// Before: src/board_ui.rs (getting too long)

// After:
// src/board_ui/mod.rs        — public API, re-exports
// src/board_ui/moves.rs      — move logic
// src/board_ui/rendering.rs  — view helpers
```

## Comments

### When to Comment

Comment any non-obvious logic. Good names and small functions come first, but don't shy away from comments when they add clarity.

### Doc Comments (`///`)

All public items get doc comments. Skip restating what the name already says — focus on behavior, invariants, and edge cases:

```rust
// Redundant — skip this
/// Returns the color of the piece
fn color(&self) -> Color

// Useful — clarifies non-obvious behavior
/// Returns valid destinations, excluding moves that would
/// leave the king in check
fn legal_moves_for_piece(board: &Board, piece: &Piece) -> Vec<Move>

/// Scores favor white (positive) over black (negative),
/// measured in centipawns
fn material_balance(board: &Board) -> i32
```

### TODO/FIXME

Must reference an issue number:

```rust
// TODO(#42): handle en passant edge case
```

### Section Comments

No section comments — if you need them, the file should be split into a directory module instead.

### Chess Domain

Assume the reader knows chess. No comments explaining standard rules.

## Iteration

Prefer iterating over collections/slices rather than index ranges:

```rust
// Yes
board.rows().map(|row| ...)

// Avoid
(0..BOARD_SIZE).map(|row| ...)
```
