---
name: Type Conventions
description: Type annotations, aliases, derives, and visibility
globs: src/**/*.rs
---

# Type Conventions

## Type Annotations — Local Variables

Annotate only when the type isn't obvious from the right-hand side:

```rust
let selected_node: Option<NodeId> = None;        // annotate — not obvious
let visit_count = node.visit_count();             // skip — clearly u32
```

## Type Annotations — Closures

Let the compiler infer:

```rust
let above_threshold = |node| node.visit_count > min_visits;
```

## Collect / Turbofish

Use turbofish on the method:

```rust
let candidates = filtered_moves.collect::<Vec<MoveInfo>>();
```

## Complex Types

Use named types / type aliases for complex types:

```rust
type MaiaDistribution = HashMap<String, f32>;
type NodeChildren = Vec<(String, Option<NodeId>)>;
```

## Struct Instantiation

Use `..Default::default()` for partial initialization:

```rust
let config = SearchConfig {
    max_iterations: 10_000,
    contempt: 0.6,
    ..Default::default()
};
```

## `Self` vs Type Name

Use the concrete type name, not `Self`:

```rust
impl TreeNode {
    fn new(node_type: NodeType) -> TreeNode { ... }
}
```

## String Types

`&str` for parameters, `String` for owned fields.

## Derive Order

By category — std traits first, then external:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
```

## Visibility

Minimal — only `pub` what's needed outside the module.

## Constants

Use `const` at module level, no magic numbers:

```rust
const CPUCT_INIT: f64 = 1.5;
const CPUCT_BASE: f64 = 19652.0;
const FPU_REDUCTION: f64 = 0.3;
const MAIA_MIN_PROBABILITY: f64 = 0.001;
