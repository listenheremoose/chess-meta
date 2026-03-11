---
name: Testing
description: Test conventions, structure, and coverage strategy
globs: tests/**/*.rs
---

# Testing

## Test Location

All tests live in the `tests/` directory — no inline `#[cfg(test)]` modules.

## Test Naming

Name tests as `<scenario>_<expected>`:

```rust
#[test]
fn max_node_selects_highest_puct_child() { ... }

#[test]
fn chance_node_samples_from_maia_distribution() { ... }
```

## Assertions

Standard library only — `assert!`, `assert_eq!`, `assert_ne!`. No assertion crates.

## Test Setup

Use the builder pattern for constructing test state:

```rust
let tree = TreeBuilder::new()
    .with_root("e2e4 e7e5", NodeType::Max)
    .with_child("g1f3", 0.58, 1200)
    .with_child("d2d4", 0.61, 800)
    .build();
```

## Position Setup

Use move sequences for test positions (matching the app's internal representation):

```rust
// Yes
let position = position_from_moves(&["e2e4", "e7e5", "g1f3"]);

// Avoid raw FEN in tests unless testing FEN-specific functionality
```

## Test Ordering

Group tests by scenario/feature. Within each group, failure cases first, then successes:

```rust
// -- PUCT Selection --

#[test]
fn puct_with_zero_visits_uses_fpu_reduction() { ... }

#[test]
fn puct_with_no_children_returns_none() { ... }

#[test]
fn puct_selects_high_prior_when_all_unvisited() { ... }

#[test]
fn puct_balances_exploration_and_exploitation() { ... }
```

## Test Scope

Test at all levels:

- **Unit tests** — core logic: PUCT selection, Maia sampling, backpropagation, value conversion
- **Integration tests** — full MCTS iteration cycles, UCI parse → evaluate → backprop flows
- **Snapshot tests** — use `insta` to capture tree state, move rankings, and search progress; commit `.snap` files to version control

## Coverage

Maximize test coverage. When adding or modifying logic, add tests for every reachable code path — happy paths, edge cases, and error cases.
