---
name: Error Handling
description: Error types, propagation, and panic policy
globs: src/**/*.rs
---

# Error Handling

## Error Propagation

Use `Result<T, E>` everywhere. Propagate with `?`:

```rust
fn parse_fen(fen: &str) -> Result<Board, ParseError> {
    let parts = split_fen(fen)?;
    let pieces = parse_pieces(parts.pieces)?;
    let side = parse_side(parts.side)?;
    Ok(Board::new(pieces, side))
}
```

## Error Types

Define custom error enums per module:

```rust
// board/error.rs
enum BoardError {
    InvalidSquare { index: u8 },
    NoPieceAt { square: Square },
    OccupiedSquare { square: Square, existing: Piece },
}

// parse/error.rs
enum ParseError {
    InvalidFen { fen: String, reason: String },
    InvalidPiece { char: char, rank: u8 },
    InvalidSquare { notation: String },
}
```

## Error Context

Include relevant data in error variants, and chain context when errors cross boundaries:

```rust
enum ParseError {
    InvalidPiece { char: char, rank: u8 },
}

enum LoadError {
    Parse { source: ParseError, fen: String },
    Io { source: std::io::Error, path: PathBuf },
}
```

## Panic Policy

No `panic!` in production code. Always return `Result`.

## Unwrap/Expect

Never use `.unwrap()` or `.expect()`. Always handle the error explicitly:

```rust
// Yes
let piece = board.piece_at(square).ok_or(BoardError::NoPieceAt { square })?;

// Never
let piece = board.piece_at(square).unwrap();
let piece = board.piece_at(square).expect("should have piece");
```
