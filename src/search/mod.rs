use std::collections::HashMap;

use serde::{Deserialize, Serialize};

/// Unique identifier for a node in the MCTS tree.
/// Indexes directly into the arena `Vec<Node>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[repr(transparent)]
pub struct NodeId(pub u32);

impl NodeId {
    #[inline]
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

/// Node type in the MCTS tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// Our turn — select via PUCT.
    Max,
    /// Opponent's turn — sample proportional to Maia distribution.
    Chance,
}

/// Pre-allocated buffers reused across MCTS iterations.
/// Avoids heap allocation in the hot select/backprop loop.
pub struct SearchState {
    pub(super) chance_probs: Vec<f64>,
}

impl SearchState {
    pub fn new() -> Self {
        Self {
            chance_probs: Vec::with_capacity(64),
        }
    }
}

/// A single node in the MCTS tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: NodeId,
    pub parent: Option<NodeId>,
    pub move_uci: Option<String>,
    pub node_type: NodeType,
    pub epd: String,
    pub move_sequence: String,

    pub visit_count: u64,
    pub total_value: f64,
    pub prior: f64,

    pub children: Vec<NodeId>,
    pub expanded: bool,

    /// Engine policy (percentage, 0-100) per move from this position.
    pub engine_policy: Option<HashMap<String, f32>>,
    /// Engine Q values per move from this position.
    pub engine_q_values: Option<HashMap<String, f32>>,
    /// Maia policy (percentage, 0-100) per move from this position.
    pub maia_policy: Option<HashMap<String, f32>>,
    /// WDL from engine eval.
    pub wdl: Option<(u32, u32, u32)>,
    /// Terminal value if game is over.
    pub terminal_value: Option<f64>,
}

impl Node {
    pub fn new(
        id: NodeId,
        parent: Option<NodeId>,
        move_uci: Option<String>,
        node_type: NodeType,
        epd: String,
        move_sequence: String,
    ) -> Self {
        Self {
            id,
            parent,
            move_uci,
            node_type,
            epd,
            move_sequence,
            visit_count: 0,
            total_value: 0.0,
            prior: 0.0,
            children: Vec::new(),
            expanded: false,
            engine_policy: None,
            engine_q_values: None,
            maia_policy: None,
            wdl: None,
            terminal_value: None,
        }
    }

    /// Average value from White's perspective.
    #[inline]
    pub fn q_value(&self) -> f64 {
        if self.visit_count == 0 {
            0.5
        } else {
            self.total_value / self.visit_count as f64
        }
    }
}

/// The full MCTS tree, stored as a flat Vec arena.
/// Nodes are indexed by `NodeId` which maps directly to the Vec index.
pub struct SearchTree {
    pub nodes: Vec<Node>,
    pub root_id: NodeId,
    next_id: u32,
}

impl SearchTree {
    pub fn new(root_epd: String, root_move_sequence: String, root_type: NodeType) -> Self {
        let root = Node::new(NodeId(0), None, None, root_type, root_epd, root_move_sequence);
        let nodes = vec![root];
        Self {
            nodes,
            root_id: NodeId(0),
            next_id: 1,
        }
    }

    /// Reconstruct a tree from a pre-built node Vec (e.g., loaded from DB).
    /// Nodes must be ordered by their ID (index 0 = NodeId(0), etc.).
    pub fn from_nodes(nodes: Vec<Node>, root_id: NodeId, next_id: u32) -> Self {
        Self {
            nodes,
            root_id,
            next_id,
        }
    }

    #[inline]
    pub fn root(&self) -> &Node {
        &self.nodes[self.root_id.index()]
    }

    #[allow(dead_code)] // Used by integration tests (separate crate, invisible to lint)
    pub fn root_mut(&mut self) -> &mut Node {
        let idx = self.root_id.index();
        &mut self.nodes[idx]
    }

    #[inline]
    pub fn get(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(id.index())
    }

    #[inline]
    pub fn get_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(id.index())
    }

    /// Add a child node and return its ID.
    pub fn add_child(
        &mut self,
        parent_id: NodeId,
        move_uci: String,
        node_type: NodeType,
        epd: String,
        move_sequence: String,
        prior: f64,
    ) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;

        let mut node = Node::new(
            id,
            Some(parent_id),
            Some(move_uci),
            node_type,
            epd,
            move_sequence,
        );
        node.prior = prior;

        self.nodes.push(node);
        self.nodes[parent_id.index()].children.push(id);
        id
    }

    #[inline]
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Heuristic: whether White is to move, based on move count parity.
#[inline]
pub(super) fn is_white_to_move_from_node(node: &Node) -> bool {
    if node.move_sequence.is_empty() {
        true
    } else {
        node.move_sequence.split_whitespace().count() % 2 == 0
    }
}

mod backprop;
mod candidates;
mod query;
mod selection;

pub use backprop::backpropagate;
pub use candidates::{candidate_moves_chance, candidate_moves_max};
#[allow(unused_imports)] // Used by integration tests (separate crate, invisible to lint)
pub use query::{best_root_move, root_move_infos, RootMoveInfo};
pub use selection::select;

#[cfg(test)]
mod test_helpers;

#[cfg(test)]
mod tests {
    use super::{Node, NodeId, NodeType, is_white_to_move_from_node};

    #[test]
    fn is_white_to_move_startpos_returns_true() {
        let node = Node::new(NodeId(0), None, None, NodeType::Max, "start".into(), "".into());
        assert!(is_white_to_move_from_node(&node));
    }

    #[test]
    fn is_white_to_move_after_one_move_returns_false() {
        let node = Node::new(NodeId(0), None, None, NodeType::Max, "pos".into(), "e2e4".into());
        assert!(!is_white_to_move_from_node(&node));
    }

    #[test]
    fn is_white_to_move_after_two_moves_returns_true() {
        let node = Node::new(NodeId(0), None, None, NodeType::Max, "pos".into(), "e2e4 e7e5".into());
        assert!(is_white_to_move_from_node(&node));
    }

    #[test]
    fn q_value_defaults_to_half_when_unvisited() {
        let node = Node::new(NodeId(0), None, None, NodeType::Max, "pos".into(), "".into());
        assert!((node.q_value() - 0.5).abs() < 0.001);
    }
}
