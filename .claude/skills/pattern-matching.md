---
name: Pattern Matching
description: Pattern matching conventions and preferences
user_invocable: true
globs: src/**/*.rs
---

# Pattern Matching

Pattern matching is the preferred control flow style in this project. Use it aggressively.

## Keep Match Arms Concise

Factor multi-line logic into separate functions so match statements stay readable at a glance:

```rust
// Yes — each arm is a single call, control flow is clear
match message {
    Msg::StartSearch(position) => start_search(position, &mut coordinator),
    Msg::PauseSearch => pause_search(&mut coordinator),
    Msg::NodeClicked(node_id) => select_node(node_id, &mut tree_view),
}

// No — inline multi-line logic obscures the match structure
match message {
    Msg::StartSearch(position) => {
        let config = build_config(&settings);
        coordinator.reset();
        coordinator.start(position, config);
        status = SearchStatus::Running;
    }
    // ...
}
```

## Destructuring

Destructure as much as possible:

```rust
let TreeNode { visit_count, value_sum, node_type, .. } = node;
```

## Nested Patterns

Flatten with nested patterns rather than matching then destructuring:

```rust
match node {
    TreeNode { node_type: NodeType::Max, .. } => select_by_puct(node),
    TreeNode { node_type: NodeType::Chance, .. } => sample_by_maia(node),
}
```

## Guards

Prefer match guards over nested matches:

```rust
match child {
    Some(node) if node.visit_count > threshold => expand_further(node),
    Some(_) => skip_low_visit_node(),
    None => create_and_evaluate(),
}
```

## Always Use `match`

Prefer `match` over `if let`, `let-else`, and `matches!`. Always use full `match` even for single-variant checks or boolean pattern tests:

```rust
// Yes — full match
match cached_eval {
    Some(result) => use_cached(result),
    None => {}
}

// No — if let
if let Some(result) = cached_eval {
    use_cached(result);
}

// Yes — full match for early return
match cached_eval {
    Some(result) => result,
    None => return,
}

// No — let-else
let Some(result) = cached_eval else { return };

// Yes — full match for boolean checks
let high_confidence = match node.visit_count {
    count if count > 500 => true,
    _ => false,
};

// No — matches! macro
let high_confidence = node.visit_count > 500;
```

## Or-Patterns

Group related variants freely:

```rust
match node_type {
    NodeType::Max => select_by_puct(node),
    NodeType::Chance => sample_by_maia(node),
}
```

## Tuple and Slice Patterns

Use freely and prefer them when possible for state combos and sequences:

```rust
// State combinations
match (search_status, has_maia_data) {
    (SearchStatus::Running, true) => continue_with_maia(node),
    (SearchStatus::Running, false) => evaluate_without_maia(node),
    (SearchStatus::Paused, _) => show_partial_results(),
    (SearchStatus::Idle, _) => {}
}

// Binding within tuples
match (node.node_type, node.visit_count) {
    (NodeType::Max, 0) => use_fpu_reduction(node),
    (NodeType::Max, count) => select_puct(node, count),
    (NodeType::Chance, _) => sample_maia(node),
}

// Slice patterns
match candidates.as_slice() {
    [] => text("No candidates"),
    [only] => text!("Single candidate: {only}"),
    [best, ..] => text!("Best: {best} (+{} more)", candidates.len() - 1),
}
```
