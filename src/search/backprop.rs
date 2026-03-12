use super::{NodeId, SearchTree};

/// Virtual loss value — pessimistic for White, makes path less attractive.
const VIRTUAL_LOSS: f64 = 0.0;

/// Apply virtual loss along a path to discourage re-selection during batched search.
/// Increments visit count and adds a pessimistic value at each node on the path.
pub fn apply_virtual_loss(tree: &mut SearchTree, path: &[NodeId]) {
    for &id in path {
        let node = &mut tree.nodes[id.index()];
        node.visit_count += 1;
        node.total_value += VIRTUAL_LOSS;
    }
}

/// Revert virtual loss along a path after batch evaluation completes.
pub fn revert_virtual_loss(tree: &mut SearchTree, path: &[NodeId]) {
    for &id in path {
        let node = &mut tree.nodes[id.index()];
        node.visit_count -= 1;
        node.total_value -= VIRTUAL_LOSS;
    }
}

/// Backpropagate a value (from White's perspective) up the tree.
pub fn backpropagate(tree: &mut SearchTree, leaf_id: NodeId, value_white: f64) {
    #[cfg(feature = "search-trace")]
    log::trace!("backprop leaf={:?} value={value_white:.4}", leaf_id);
    let mut current = Some(leaf_id);
    while let Some(id) = current {
        let node = &mut tree.nodes[id.index()];
        node.visit_count += 1;
        node.total_value += value_white;
        current = node.parent;
    }
}

#[cfg(test)]
mod tests {
    use crate::search::{NodeId, NodeType};
    use super::super::test_helpers::TreeBuilder;
    use super::backpropagate;

    #[test]
    fn backprop_updates_leaf_and_ancestors() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .expanded(NodeId(0))
            .build();
        let child_id = tree.root().children[0];

        backpropagate(&mut tree, child_id, 0.7);

        assert_eq!(tree.root().visit_count, 1);
        assert!((tree.root().q_value() - 0.7).abs() < 0.001);
        assert_eq!(tree.get(child_id).unwrap().visit_count, 1);
        assert!((tree.get(child_id).unwrap().q_value() - 0.7).abs() < 0.001);
    }

    #[test]
    fn backprop_accumulates_across_multiple_visits() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .expanded(NodeId(0))
            .build();
        let child_id = tree.root().children[0];

        backpropagate(&mut tree, child_id, 0.8);
        backpropagate(&mut tree, child_id, 0.4);

        assert_eq!(tree.root().visit_count, 2);
        assert!((tree.root().q_value() - 0.6).abs() < 0.001);
    }

    #[test]
    fn backprop_three_levels_deep() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .expanded(NodeId(0))
            .build();
        let child_id = tree.root().children[0];
        let grandchild_id = tree.add_child(
            child_id,
            "e7e5".to_string(),
            NodeType::Max,
            "pos3".to_string(),
            "e2e4 e7e5".to_string(),
            0.3,
        );

        backpropagate(&mut tree, grandchild_id, 0.6);

        assert_eq!(tree.root().visit_count, 1);
        assert_eq!(tree.get(child_id).unwrap().visit_count, 1);
        assert_eq!(tree.get(grandchild_id).unwrap().visit_count, 1);
        assert!((tree.root().q_value() - 0.6).abs() < 0.001);
    }
}
