---
name: Functional Patterns
description: Functional programming conventions and iterator usage
user_invocable: true
globs: src/**/*.rs
---

# Functional Patterns

Prefer functional style as much as possible.

## Philosophy

Transform data functionally, but make decisions visibly. Use iterator chains for data transformation, but use `match` for control flow rather than hiding branching logic inside Option/Result combinators.

## Iterators Over Loops

Always use iterator chains, never `for` loops:

```rust
// Yes
let high_visit_nodes = tree
    .children(root_id)
    .filter(|node| node.visit_count > threshold)
    .collect::<Vec<_>>();

// No
let mut high_visit_nodes = Vec::new();
for node in tree.children(root_id) {
    if node.visit_count > threshold {
        high_visit_nodes.push(node);
    }
}
```

## Option/Result Handling

Use `match` over combinators like `map`, `and_then`, `unwrap_or` (consistent with pattern matching preference):

```rust
// Yes
let eval_value = match engine_cache.get(&epd) {
    Some(result) if result.nodes_searched >= min_nodes => result.q_value,
    _ => evaluate_position(&position),
};

// No
let eval_value = engine_cache.get(&epd)
    .filter(|result| result.nodes_searched >= min_nodes)
    .map(|result| result.q_value)
    .unwrap_or_else(|| evaluate_position(&position));
```

## Function Composition

Prefer point-free style where possible. When closures are needed, use descriptive variable names (see naming skill):

```rust
// Best — point-free method reference
children
    .iter()
    .filter(TreeNode::is_expanded)
    .map(TreeNode::q_value)

// OK — closure with descriptive name when logic requires it
children
    .iter()
    .filter(|child| child.visit_count > min_visits)
```

## Fold and Accumulation

Use specialized methods (`sum`, `count`, `any`, `all`) when available. Use `fold` for custom accumulation:

```rust
// Specialized
let total_visits: u32 = children
    .iter()
    .map(TreeNode::visit_count)
    .sum();

let has_converged = children
    .iter()
    .all(|child| child.q_stable());

// Custom accumulation with fold
let weighted_value = maia_moves
    .iter()
    .fold(0.0, |accumulator, candidate| {
        accumulator + candidate.maia_probability * candidate.child_q_value
    });
```

## Immutability

Default immutable, only `mut` when required:

```rust
let config = SearchConfig::default();
let mut iteration_count = 0;  // only when mutation is needed
```

## Chain Length

Break into named intermediate variables after 3-4 steps:

```rust
let mut by_engine = moves.to_vec();
by_engine.sort_by(|a, b| b.q_value.partial_cmp(&a.q_value).unwrap_or(Ordering::Equal));
let engine_candidates = &by_engine[..3.min(by_engine.len())];

let mut by_maia = moves.to_vec();
by_maia.sort_by(|a, b| b.maia_policy.partial_cmp(&a.maia_policy).unwrap_or(Ordering::Equal));
let maia_candidates = &by_maia[..5.min(by_maia.len())];

let mut all_candidates = engine_candidates.to_vec();
all_candidates.extend(
    maia_candidates
        .iter()
        .filter(|candidate| !all_candidates.iter().any(|existing| existing.uci_move == candidate.uci_move)),
);
```
