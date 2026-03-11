---
name: Code Organization
description: Function length, file length, comments, and iteration
globs: src/**/*.rs
---

# Code Organization

## Function Length

Soft limit of ~40 lines (one screen). If it scrolls past one screen, split it.

Split when:
- A function does more than one logical step
- You'd want to add a comment explaining a block — make it a named function instead

## File Length

Keep files under ~200 lines. When a file grows past this, split into a directory module:

```
// Before: src/search.rs (getting too long)

// After:
// src/search/mod.rs          — public API, re-exports
// src/search/selection.rs    — PUCT selection logic
// src/search/expansion.rs    — node expansion and evaluation
// src/search/backprop.rs     — backpropagation
```

## Comments

### When to Comment

Comment any non-obvious logic. Good names and small functions come first, but don't shy away from comments when they add clarity.

### Doc Comments (`///`)

All public items get doc comments. Skip restating what the name already says — focus on behavior, invariants, and edge cases:

```rust
// Redundant — skip this
/// Returns the visit count
fn visit_count(&self) -> u32

// Useful — clarifies non-obvious behavior
/// Selects a child using PUCT, converting stored White-perspective Q values
/// to side-to-move perspective for comparison
fn select_child_puct(node: &TreeNode, config: &SearchConfig) -> NodeId

/// Value from White's perspective in [0, 1]. Draws are scored
/// using the contempt parameter (0.5 = neutral, 0.6 = slightly favor us)
fn value_for_backprop(wdl: (u32, u32, u32), contempt: f64) -> f64
```

### TODO/FIXME

Must reference an issue number:

```rust
// TODO(#12): add periodic ucinewgame to prevent memory leaks
```

### Section Comments

No section comments — if you need them, the file should be split into a directory module instead.

### Domain Knowledge

Assume the reader knows chess and basic MCTS concepts. No comments explaining UCI protocol basics or standard tree search terminology.

## Iteration

Prefer iterating over collections/slices rather than index ranges:

```rust
// Yes
node.children().filter(|child| child.visit_count > 0)

// Avoid
(0..node.child_count()).filter(|index| node.child(index).visit_count > 0)
```
