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
eprintln!("[ERROR] Failed to load game: {err}");
eprintln!("[WARN] Move took 2.3s to compute");
println!("[INFO] Game started");
println!("[DEBUG] Evaluating position: material={material}");
```

## What to Log

### Non-performance-critical code (log heavily)

Log everything useful — no restraint needed:

- **Errors** — anything that fails or is unexpected
- **Warnings** — slow operations, fallback behavior
- **State transitions** — game start/end, turn changes, check/checkmate, captures, castling, promotion
- **Analysis events** — position loaded, analysis started/completed, results summary
- **All move candidates considered** — with evaluation scores
- **Configuration** — startup settings, window size, loaded options

### Performance-critical code (behind feature flag)

Search, evaluation, and move generation are hot paths. Log nothing by default. Use a compile-time feature flag for optional tracing:

```rust
#[cfg(feature = "search-trace")]
println!("[TRACE] depth={depth} nodes={nodes} score={score} pv={pv}");
```

In `Cargo.toml`:

```toml
[features]
search-trace = []
```

Enable with `cargo build --features search-trace`. When disabled, these lines are compiled out entirely — zero overhead.

Hot paths that must be log-free by default:
- Move generation
- Position evaluation
- Search tree traversal
- Board state updates during search

Log before and after the search in normal builds:

```rust
println!("[INFO] Search started depth={max_depth} position={fen}");
// ... search runs with no logging (unless search-trace enabled) ...
println!("[INFO] Search complete depth={depth} nodes={nodes} best={best_move} score={score}");
```

## Log Format

Structured with context — include relevant data inline:

```rust
println!("[INFO] Move applied piece=Knight from=G1 to=F3");
println!("[DEBUG] Legal moves count=23 for=White");
eprintln!("[ERROR] Invalid square index={index}");
```

## Repeated Events

Log first occurrence, then summarize:

```rust
// Instead of logging "Invalid input" 50 times:
eprintln!("[WARN] Invalid input repeated count=12");
```

## Output Destination

Log to both stderr and a file. Errors and warnings go to stderr, everything goes to the log file. Store log files in `./logs/`.

## Log Rotation

Rotate log files at 1MB. Keep the last 10 files, delete older ones.

## Logging in Tests

Tests should always produce log output to help debug failures. Don't suppress output in tests.
