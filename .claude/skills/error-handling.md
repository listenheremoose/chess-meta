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

## Error Context

Include relevant data in error variants, and chain context when errors cross boundaries:

```rust
enum EngineError {
    UciProtocolError { expected: String, got: String },
}

enum SearchError {
    Engine { source: EngineError, position: String },
    Cache { source: CacheError, epd: String },
}
```

## Panic Policy

No `panic!` in production code. Always return `Result`.

## Unwrap/Expect

Never use `.unwrap()` or `.expect()`. Always handle the error explicitly:

```rust
// Yes
let node = tree.get(node_id).ok_or(SearchError::NodeNotFound { node_id })?;

// Never
let node = tree.get(node_id).unwrap();
let node = tree.get(node_id).expect("node should exist");
```
