---
name: Logging
description: Logging conventions and output strategy
globs: src/**/*.rs
---

# Logging

## Crate

No logging dependencies — use `println!` and `eprintln!` only.

## Log Levels

Use `eprintln!` for errors and warnings, `println!` for informational and debug output. Distinguish severity with prefixes:

```rust
eprintln!("[ERROR] lc0 process crashed: {err}");
eprintln!("[WARN] lc0 memory exceeds threshold, restarting");
println!("[INFO] Search started position={position_key} max_iterations={max}");
println!("[DEBUG] Node expanded epd={epd} value={value}");
```

## What to Log

### Non-performance-critical code (log heavily)

Log everything useful — no restraint needed:

- **Errors** — lc0 crashes, UCI parse failures, SQLite errors
- **Warnings** — lc0 memory growth, slow evaluations, engine restarts
- **State transitions** — search started/paused/resumed/completed, engine process spawned/killed
- **Search milestones** — every 100 iterations: best move, Q-value, visit distribution
- **Configuration** — startup settings, engine paths, search parameters

### Performance-critical code (behind feature flag)

The MCTS inner loop (selection, expansion, backprop) is the hot path. Log nothing by default. Use a compile-time feature flag for optional tracing:

```rust
#[cfg(feature = "search-trace")]
println!("[TRACE] iteration={iteration} selected={node_id} depth={depth}");
```

In `Cargo.toml`:

```toml
[features]
search-trace = []
```

Enable with `cargo build --features search-trace`. When disabled, these lines are compiled out entirely — zero overhead.

Hot paths that must be log-free by default:
- PUCT selection traversal
- Maia sampling
- Backpropagation
- Tree node creation

Log before and after the search in normal builds:

```rust
println!("[INFO] Search started iterations={max} position={position_key}");
// ... MCTS runs with no logging (unless search-trace enabled) ...
println!("[INFO] Search complete iterations={count} best={best_move} practical_q={q}");
```

## Log Format

Structured with context — include relevant data inline:

```rust
println!("[INFO] Engine initialized path={path} weights={weights}");
println!("[DEBUG] Cache hit epd={epd} nodes={nodes_searched}");
eprintln!("[ERROR] UCI parse failed line={line}");
```

## Repeated Events

Log first occurrence, then summarize:

```rust
// Instead of logging "Cache hit" 5000 times:
println!("[INFO] Search complete cache_hits={hits} cache_misses={misses}");
```

## Output Destination

Log to stderr only (`eprintln!` for errors/warnings, `println!` for info/debug goes to stdout).
Redirect to a file at the shell level if persistent logs are needed: `chess-meta 2>&1 | tee logs/run.log`

## Logging in Tests

Tests should always produce log output to help debug failures. Don't suppress output in tests.
