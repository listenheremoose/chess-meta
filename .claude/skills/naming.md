---
name: Naming Conventions
description: Variable, function, and type naming style
user_invocable: true
globs: src/**/*.rs
---

# Naming Conventions

Be verbose and descriptive. Readability over brevity.

## Variables

Always use full descriptive names:

```rust
let selected_node = tree.get(node_id);
let candidate_moves = generate_candidates(&position, &engine_policy, &maia_policy);
```

## Iterator Variables

Descriptive even in closures:

```rust
nodes.iter().filter(|node| node.visit_count > threshold)
children.iter().map(|child| child.q_value())
```

## Index Variables

Full descriptive names, no abbreviations:

```rust
for (move_index, candidate) in candidates.iter().enumerate() { ... }
```

## Booleans

Read as natural English, no `is_`/`has_` prefix required:

```rust
let converged = check_convergence(&root, &metrics);
let terminal = position.is_game_over();
```

## No Abbreviations

Never abbreviate. Spell out fully:

```rust
let position = apply_move(&current, candidate);
let iteration_count = coordinator.total_iterations();

// Not: pos, iter, eval, coord
```

## Function Names

Descriptive and natural, not forced into verb-first. The name should clearly convey what the function does or returns:

```rust
fn candidate_moves_for_position(position: &Chess, engine: &[MoveInfo], maia: &[MoveInfo]) -> Vec<UciMove>
fn practical_score_for_move(node: &TreeNode) -> f64
fn nodes_above_threshold(tree: &SearchTree, min_visits: u32) -> Vec<&TreeNode>
```

## Type / Struct Names

Concise, with context implied by the module:

```rust
struct TreeNode { ... }       // in search module, obviously an MCTS node
enum NodeType { Max, Chance }
struct SearchConfig { ... }
```
