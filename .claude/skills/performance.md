---
name: Performance
description: Performance conventions for MCTS analysis
user_invocable: true
globs: src/**/*.rs
---

# Performance

## Allocation Strategy

Avoid heap allocation in hot paths:

- Pre-allocate tree node storage (arena or Vec-based)
- Reuse buffers for UCI communication strings
- Never allocate inside the MCTS selection/backprop loop

```rust
// Yes — pre-allocated path buffer reused across iterations
let mut path = Vec::with_capacity(64);
path.clear();
select(&tree, root, &mut path);

// Avoid — allocates on every iteration
fn select(tree: &Tree, root: NodeId) -> Vec<NodeId> { ... }
```

## Position Representation

Use `shakmaty` for position logic (legal moves, game-over detection). Cache EPD strings and move paths on tree nodes to avoid recomputation.

## Copy vs Reference

Derive `Copy` for small types (`NodeId`, `NodeType`, `SearchStatus`). Pass by value, not reference.

```rust
#[derive(Clone, Copy)]
struct NodeId(u32);

// Yes — by value
fn get_node(tree: &Tree, node_id: NodeId) -> &TreeNode { ... }

// Avoid for tiny types
fn get_node(tree: &Tree, node_id: &NodeId) -> &TreeNode { ... }
```

## Profiling

Use both approaches:

- **`criterion`** — benchmark critical paths (PUCT selection, tree traversal, UCI parsing). Run regularly to catch regressions.
- **Flamegraphs** — for investigation when optimizing. Use `cargo flamegraph` or a system profiler.

Add benchmarks in `benches/` using `criterion`:

```rust
fn bench_puct_selection(c: &mut Criterion) {
    let tree = build_test_tree(1000);
    c.bench_function("puct_selection", |b| {
        b.iter(|| select_child_puct(&tree, root_id, &config))
    });
}
```

## Unsafe Code

No `unsafe` code. Use safe Rust only — rely on the compiler to optimize bounds checks away where it can.

## Inlining

Use `#[inline]` on small hot functions when profiling shows it helps. Don't add it speculatively:

```rust
#[inline]
fn q_from_white_perspective(node: &TreeNode) -> f64 {
    node.value_sum / node.visit_count as f64
}
```

## Parallelism

MCTS runs in a background thread, UI runs on the main thread. Communication via `Arc<Mutex<SearchSnapshot>>`.

- Keep mutable tree state owned by the search thread
- The UI thread reads periodic snapshots (every ~100 iterations)
- Engine/Maia processes are owned by the search thread (not shared)
