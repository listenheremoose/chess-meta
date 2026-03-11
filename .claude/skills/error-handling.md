---
name: Error Handling
description: Error types, propagation, and panic policy
globs: src/**/*.rs
---

# Error Handling

## Error Propagation

Use `Result<T, E>` everywhere. Propagate with `?`:

```rust
fn query_engine(position_key: &str, epd: &str) -> Result<EngineResult, EngineError> {
    let engine = acquire_engine(&lc0_path, &weights_path)?;
    engine.send_position(position_key)?;
    let output = engine.go_nodes(1)?;
    parse_engine_output(&output)
}
```

## Error Types

Define custom error enums per module:

```rust
// engine/error.rs
enum EngineError {
    ProcessSpawnFailed { path: String, source: std::io::Error },
    UciProtocolError { expected: String, got: String },
    ProcessCrashed { stderr: String },
    Cancelled,
}

// cache/error.rs
enum CacheError {
    DatabaseOpen { path: PathBuf, source: rusqlite::Error },
    QueryFailed { query: String, source: rusqlite::Error },
    SerializationFailed { source: serde_json::Error },
}
```

Use `String` errors for early prototyping, but migrate to typed errors as modules stabilize.

## Error Context

Include relevant data in error variants, and chain context when errors cross boundaries:

```rust
enum SearchError {
    Engine { source: EngineError, position: String },
    Cache { source: CacheError, epd: String },
}
```

## Panic Policy

No `panic!` in production code. Always return `Result`.

## Unwrap/Expect

Avoid `.unwrap()` and `.expect()` at module boundaries and in code that handles external input (UCI parsing, file I/O, user input).

`.unwrap()` is acceptable for internal invariants where the value is structurally guaranteed to exist — e.g., accessing a tree node that was just inserted, or indexing a HashMap that was just populated. Prefer adding a comment explaining the invariant:

```rust
// Node was just added to the tree in the line above
let node = tree.get_mut(child_id).unwrap();
```

At module boundaries, always handle explicitly:

```rust
let node = tree.get(node_id).ok_or(SearchError::NodeNotFound { node_id })?;
```
