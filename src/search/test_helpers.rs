use super::{NodeId, NodeType, SearchTree};

pub struct TreeBuilder {
    tree: SearchTree,
}

impl TreeBuilder {
    pub fn new() -> Self {
        Self {
            tree: SearchTree::new("startpos".to_string(), String::new(), NodeType::Max),
        }
    }

    pub fn with_root(move_seq: &str, node_type: NodeType) -> Self {
        Self {
            tree: SearchTree::new("pos".to_string(), move_seq.to_string(), node_type),
        }
    }

    /// Add a child to the given parent with preset visits and Q value.
    pub fn with_child(
        mut self,
        parent_id: NodeId,
        move_uci: &str,
        node_type: NodeType,
        prior: f64,
        visits: u64,
        q_white: f64,
    ) -> Self {
        let id = self.tree.add_child(
            parent_id,
            move_uci.to_string(),
            node_type,
            format!("pos_after_{move_uci}"),
            if parent_id == NodeId(0) {
                move_uci.to_string()
            } else {
                format!("e2e4 {move_uci}")
            },
            prior,
        );
        let node = self.tree.get_mut(id).unwrap();
        node.visit_count = visits;
        node.total_value = q_white * visits as f64;
        self
    }

    /// Mark a node as expanded.
    pub fn expanded(mut self, node_id: NodeId) -> Self {
        self.tree.get_mut(node_id).unwrap().expanded = true;
        self
    }

    pub fn build(self) -> SearchTree {
        self.tree
    }
}
