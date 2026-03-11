---
name: Performance
description: Performance conventions for chess analysis
globs: src/**/*.rs
---

# Performance

## Allocation Strategy

Avoid heap allocation in hot paths:

- Use stack-allocated fixed-size arrays for bounded data (e.g. `[Move; 256]` for move lists)
- Pre-allocate and reuse buffers for variable-size data
- Never allocate inside search, evaluation, or move generation loops

```rust
// Yes — stack array, no allocation
let mut moves = [Move::NONE; 256];
let count = generate_moves(&board, &mut moves);

// Avoid — allocates on every call
fn generate_moves(board: &Board) -> Vec<Move> { ... }
```

## Board Representation

Use bitboards — one `u64` per piece type/color. Use bitwise operations for attack/pin/check computation instead of looping over squares.

## Copy vs Reference

Derive `Copy` for small types (`Move`, `Square`, `Piece`, `Color`, `Bitboard`). Pass by value, not reference.

```rust
#[derive(Clone, Copy)]
struct Move { /* 2-4 bytes */ }

// Yes — by value
fn make_move(board: &mut Board, mv: Move) { ... }

// Avoid for tiny types
fn make_move(board: &mut Board, mv: &Move) { ... }
```

## Profiling

Use both approaches:

- **`criterion`** — benchmark critical paths (move generation, evaluation, search). Run regularly to catch regressions.
- **Flamegraphs** — for investigation when optimizing. Use `cargo flamegraph` or a system profiler.

Add benchmarks in `benches/` using `criterion`:

```rust
fn bench_move_gen(c: &mut Criterion) {
    let board = Board::starting_position();
    c.bench_function("generate_moves", |b| {
        b.iter(|| board.generate_moves())
    });
}
```

## Unsafe Code

No `unsafe` code. Use safe Rust only — rely on the compiler to optimize bounds checks away where it can.

## Inlining

Use `#[inline]` on small hot functions when profiling shows it helps. Don't add it speculatively:

```rust
#[inline]
fn rank(self) -> u8 {
    self.0 >> 3
}
```

## Parallelism

Single-threaded for now. Structure code to allow parallelism later:

- Keep mutable state contained and explicit
- Avoid global mutable state
- Separate search state from shared board data
