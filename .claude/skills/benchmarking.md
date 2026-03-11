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

- PUCT selection (traversal to leaf)
- Maia distribution sampling
- Backpropagation (path update)
- Tree node creation and expansion
- UCI output parsing (engine + Maia)
- SQLite cache reads/writes
- Value conversion (WDL → backprop value)
- Candidate move filtering (top 3 engine + top 5 Maia)

## Granularity

Benchmark per function, per scenario, and per input size:

```rust
// Function
bench_puct_selection

// Function + scenario
bench_puct_selection_shallow_tree
bench_puct_selection_deep_tree

// Function + input size
bench_backprop_short_path
bench_backprop_long_path
```

## Test Trees

Use a fixed set of representative tree structures:

- **Shallow** — root with 8 children, each visited 100 times
- **Deep** — linear path of 20 nodes
- **Wide** — root with 30 children, varying visit counts
- **Realistic** — 1000-node tree from an actual search run

Define builders in a shared module within `benches/`:

```rust
fn shallow_tree() -> SearchTree {
    TreeBuilder::new()
        .with_root_children(8)
        .with_visits_per_child(100)
        .build()
}
```

## Regression Tracking

Run manually and save results to compare across commits. Use `criterion`'s built-in comparison — it reports percentage change against the previous run:

```
puct_selection_deep    time: [1.234 µs 1.256 µs 1.278 µs]
                       change: [-2.1% +0.3% +2.8%] (no change)
```

Save baseline before optimizing:

```sh
cargo bench -- --save-baseline before
# ... make changes ...
cargo bench -- --baseline before
```
