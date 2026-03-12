use crate::config::Config;
use crate::engine::lookup_castling_aware;

use super::{NodeId, SearchTree};

/// Information about a root candidate move, for UI display.
#[derive(Debug, Clone)]
pub struct RootMoveInfo {
    pub uci_move: String,
    #[allow(dead_code)] // Read by integration tests (separate crate, invisible to lint)
    pub node_id: NodeId,
    pub visits: u64,
    /// Engine policy percentage for this move (0-100 from NN).
    pub engine_policy: Option<f64>,
    pub practical_q: f64,
    /// Difference between practical Q and position's engine eval.
    pub delta: Option<f64>,
    pub worst_case: f64,
    pub wdl: Option<(u32, u32, u32)>,
}

/// Get all root move infos sorted by practical Q (descending).
pub fn root_move_infos(tree: &SearchTree, config: &Config) -> Vec<RootMoveInfo> {
    let root = tree.root();
    let is_white = super::is_white_to_move_from_node(root);

    let engine_position_value = root.wdl.map(|(wins, draws, _)| {
        let v_white = wins as f64 / 1000.0 + config.contempt * draws as f64 / 1000.0;
        if is_white { v_white } else { 1.0 - v_white }
    });

    let mut move_infos: Vec<RootMoveInfo> = root
        .children
        .iter()
        .filter_map(|&child_id| build_root_move_info(tree, child_id, is_white, engine_position_value, config))
        .collect();

    move_infos.sort_by(|first, second| match second.practical_q.partial_cmp(&first.practical_q) {
        Some(ord) => ord,
        None => std::cmp::Ordering::Equal,
    });

    move_infos
}

fn build_root_move_info(
    tree: &SearchTree,
    child_id: NodeId,
    is_white: bool,
    engine_position_value: Option<f64>,
    config: &Config,
) -> Option<RootMoveInfo> {
    let root = tree.root();
    let child = tree.get(child_id)?;
    let uci_move = child.move_uci.as_ref()?.clone();
    let visits = child.visit_count;
    let q_stm = {
        let q_white = child.q_value();
        if is_white { q_white } else { 1.0 - q_white }
    };

    let engine_pol = match root.engine_policy.as_ref() {
        Some(engine_policy) => match lookup_castling_aware(&uci_move, engine_policy) {
            Some(policy_value) => Some(policy_value as f64),
            None => None,
        },
        None => None,
    };

    let worst_case = if visits > 0 {
        worst_case_value(tree, child_id, is_white)
    } else {
        q_stm
    };
    let practical_q = (1.0 - config.safety) * q_stm + config.safety * worst_case;

    let delta = engine_position_value.map(|engine_value| practical_q - engine_value);

    Some(RootMoveInfo {
        uci_move,
        node_id: child_id,
        visits,
        engine_policy: engine_pol,
        practical_q,
        delta,
        worst_case,
        wdl: child.wdl,
    })
}

/// Select the best move at the root for final recommendation.
/// Uses safety parameter to blend expected score with worst-case.
#[allow(dead_code)] // Used by integration tests (separate crate, invisible to lint)
pub fn best_root_move(tree: &SearchTree, config: &Config) -> Option<RootMoveInfo> {
    let mut infos = root_move_infos(tree, config);
    infos.retain(|move_info| move_info.visits > 0);
    infos.into_iter().next()
}

/// Worst-case value against likely opponent responses (prior > 10%).
fn worst_case_value(tree: &SearchTree, node_id: NodeId, is_white: bool) -> f64 {
    let node = &tree.nodes[node_id.index()];
    if node.children.is_empty() {
        let q_value = node.q_value();
        return if is_white { q_value } else { 1.0 - q_value };
    }

    let qualifying = node.children
        .iter()
        .filter_map(|&child_id| {
            let child = &tree.nodes[child_id.index()];
            if child.visit_count > 0 && child.prior > 0.10 {
                let child_q_stm = if is_white { child.q_value() } else { 1.0 - child.q_value() };
                Some(child_q_stm)
            } else {
                None
            }
        });

    match qualifying.fold(None, |minimum_so_far: Option<f64>, candidate_value| Some(match minimum_so_far {
        Some(previous_minimum) => previous_minimum.min(candidate_value),
        None => candidate_value,
    })) {
        Some(worst) => worst,
        None => {
            let q_value = node.q_value();
            if is_white { q_value } else { 1.0 - q_value }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::search::{NodeId, NodeType};
    use super::super::test_helpers::TreeBuilder;
    use super::{root_move_infos, worst_case_value};

    #[test]
    fn worst_case_filters_low_prior_children() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 10, 0.5)
            .expanded(NodeId(0))
            .build();

        let chance_id = tree.root().children[0];
        tree.get_mut(chance_id).unwrap().expanded = true;

        let good_id = tree.add_child(
            chance_id, "e7e5".to_string(), NodeType::Max,
            "p1".to_string(), "e2e4 e7e5".to_string(), 0.8,
        );
        tree.get_mut(good_id).unwrap().visit_count = 5;
        tree.get_mut(good_id).unwrap().total_value = 3.0;

        let bad_id = tree.add_child(
            chance_id, "d7d5".to_string(), NodeType::Max,
            "p2".to_string(), "e2e4 d7d5".to_string(), 0.05,
        );
        tree.get_mut(bad_id).unwrap().visit_count = 3;
        tree.get_mut(bad_id).unwrap().total_value = 0.6;

        let worst = worst_case_value(&tree, chance_id, true);
        assert!((worst - 0.6).abs() < 0.001);
    }

    #[test]
    fn worst_case_returns_node_q_when_no_children_qualify() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 10, 0.55)
            .expanded(NodeId(0))
            .build();

        let chance_id = tree.root().children[0];
        tree.get_mut(chance_id).unwrap().expanded = true;

        let child_id = tree.add_child(
            chance_id, "e7e5".to_string(), NodeType::Max,
            "p".to_string(), "e2e4 e7e5".to_string(), 0.05,
        );
        tree.get_mut(child_id).unwrap().visit_count = 3;
        tree.get_mut(child_id).unwrap().total_value = 0.6;

        let worst = worst_case_value(&tree, chance_id, true);
        assert!((worst - 0.55).abs() < 0.001);
    }

    #[test]
    fn worst_case_returns_minimum_among_qualifying_children() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 10, 0.5)
            .expanded(NodeId(0))
            .build();

        let chance_id = tree.root().children[0];
        tree.get_mut(chance_id).unwrap().expanded = true;

        let c1 = tree.add_child(
            chance_id, "e7e5".to_string(), NodeType::Max,
            "p1".to_string(), "e2e4 e7e5".to_string(), 0.5,
        );
        tree.get_mut(c1).unwrap().visit_count = 5;
        tree.get_mut(c1).unwrap().total_value = 3.5;

        let c2 = tree.add_child(
            chance_id, "c7c5".to_string(), NodeType::Max,
            "p2".to_string(), "e2e4 c7c5".to_string(), 0.3,
        );
        tree.get_mut(c2).unwrap().visit_count = 5;
        tree.get_mut(c2).unwrap().total_value = 2.0;

        let worst = worst_case_value(&tree, chance_id, true);
        assert!((worst - 0.4).abs() < 0.001);
    }

    #[test]
    fn root_move_infos_sorted_by_practical_q_descending() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 100, 0.4)
            .with_child(NodeId(0), "d2d4", NodeType::Chance, 0.3, 100, 0.7)
            .with_child(NodeId(0), "g1f3", NodeType::Chance, 0.2, 100, 0.55)
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 300;
            root.total_value = 165.0;
        }

        let config = Config::default();
        let infos = root_move_infos(&tree, &config);

        assert_eq!(infos.len(), 3);
        assert!(infos[0].practical_q >= infos[1].practical_q);
        assert!(infos[1].practical_q >= infos[2].practical_q);
        assert_eq!(infos[0].uci_move, "d2d4");
    }

    #[test]
    fn root_move_infos_includes_unvisited_moves() {
        let tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .with_child(NodeId(0), "d2d4", NodeType::Chance, 0.5, 10, 0.6)
            .expanded(NodeId(0))
            .build();

        let config = Config::default();
        let infos = root_move_infos(&tree, &config);
        assert_eq!(infos.len(), 2);
    }

    #[test]
    fn root_move_infos_safety_blends_q_with_worst_case() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "risky", NodeType::Chance, 0.5, 50, 0.7)
            .with_child(NodeId(0), "safe", NodeType::Chance, 0.5, 50, 0.6)
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 100;
            root.total_value = 65.0;
        }

        let risky_id = tree.root().children[0];
        tree.get_mut(risky_id).unwrap().expanded = true;
        let r1 = tree.add_child(
            risky_id, "e7e5".to_string(), NodeType::Max,
            "p".to_string(), "risky e7e5".to_string(), 0.5,
        );
        tree.get_mut(r1).unwrap().visit_count = 10;
        tree.get_mut(r1).unwrap().total_value = 2.0;
        let r2 = tree.add_child(
            risky_id, "d7d5".to_string(), NodeType::Max,
            "p2".to_string(), "risky d7d5".to_string(), 0.5,
        );
        tree.get_mut(r2).unwrap().visit_count = 10;
        tree.get_mut(r2).unwrap().total_value = 9.0;

        let mut config = Config::default();
        config.safety = 0.8;

        let infos = root_move_infos(&tree, &config);
        let safe_info = infos.iter().find(|i| i.uci_move == "safe").unwrap();
        let risky_info = infos.iter().find(|i| i.uci_move == "risky").unwrap();
        assert!(safe_info.practical_q > risky_info.practical_q,
            "safe={:.3} should beat risky={:.3} with high safety",
            safe_info.practical_q, risky_info.practical_q);
    }
}
