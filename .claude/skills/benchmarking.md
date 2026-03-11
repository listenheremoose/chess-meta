---
name: Benchmarking
description: Benchmark conventions and regression tracking
globs: benches/**/*.rs
---

# Benchmarking

## Location

All benchmarks live in `benches/` (standard Rust convention). Use the `criterion` crate.

## What to Benchmark

Benchmark everything testable:

- Move generation
- Search
- Position evaluation
- Make/unmake move
- Attack detection
- Check testing
- Board setup
- FEN parsing
- Utility functions

## Granularity

Benchmark per function, per scenario, and per input size:

```rust
// Function
bench_generate_moves

// Function + scenario
bench_generate_moves_opening
bench_generate_moves_middlegame
bench_generate_moves_endgame

// Function + scenario + input size
bench_evaluate_few_pieces
bench_evaluate_many_pieces
```

## Benchmark Positions

Use a fixed set of representative positions covering:

- **Opening** — starting position and common opening lines
- **Middlegame** — active piece play, typical material
- **Endgame** — few pieces, king activity matters
- **Complex tactical** — multiple captures, pins, forks, discovered attacks

Define these as constants in a shared module within `benches/`:

```rust
const OPENING: &str = "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1";
const MIDDLEGAME: &str = "r1bq1rk1/pp2ppbp/2np1np1/8/2BNP3/2N1B3/PPP2PPP/R2Q1RK1 w - - 0 9";
const ENDGAME: &str = "8/5pk1/6p1/8/3K4/8/6PP/8 w - - 0 40";
const TACTICAL: &str = "r2q1rk1/ppp2ppp/2n2n2/3Np1B1/2B1P1b1/3P4/PPP2PPP/R2QK2R w KQ - 0 10";
```

## Regression Tracking

Run manually and save results to compare across commits. Use `criterion`'s built-in comparison — it reports percentage change against the previous run:

```
generate_moves_opening    time: [1.234 µs 1.256 µs 1.278 µs]
                          change: [-2.1% +0.3% +2.8%] (no change)
```

Save baseline before optimizing:

```sh
cargo bench -- --save-baseline before
# ... make changes ...
cargo bench -- --baseline before
```
