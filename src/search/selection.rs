use crate::config::Config;

use super::{NodeId, NodeType, SearchState, SearchTree};

/// Select a leaf node from the tree using PUCT at MAX nodes and
/// probability-weighted sampling at CHANCE nodes.
pub fn select(tree: &SearchTree, config: &Config, state: &mut SearchState) -> NodeId {
    let mut current = tree.root_id;
    #[cfg(feature = "search-trace")]
    let mut depth = 0u32;

    loop {
        let node = &tree.nodes[current.index()];

        if !node.expanded || node.children.is_empty() {
            #[cfg(feature = "search-trace")]
            log::trace!("select leaf node_id={:?} depth={depth}", current);
            return current;
        }

        match node.node_type {
            NodeType::Max => {
                current = select_puct(tree, current, config);
            }
            NodeType::Chance => {
                current = select_chance(tree, current, config, state);
            }
        }
        #[cfg(feature = "search-trace")]
        { depth += 1; }
    }
}

/// PUCT selection at a MAX node. Picks the child with the highest UCB score,
/// converting stored White-perspective Q values to side-to-move perspective.
fn select_puct(tree: &SearchTree, node_id: NodeId, config: &Config) -> NodeId {
    let node = &tree.nodes[node_id.index()];
    let parent_visits = node.visit_count as f64;
    let is_white_turn = super::is_white_to_move_from_node(node);

    // Dynamic cpuct: C(s) = cpuct_init + cpuct_factor * ln((N(s) + cpuct_base) / cpuct_base)
    let cpuct = config.cpuct_init
        + config.cpuct_factor * ((parent_visits + config.cpuct_base) / config.cpuct_base).ln();

    let parent_q = node.q_value();

    let best_scored = node.children
        .iter()
        .map(|&child_id| {
            let child = &tree.nodes[child_id.index()];
            let child_visits = child.visit_count as f64;

            let q = if child.visit_count == 0 {
                // FPU: Q_parent_stm - fpu_reduction
                let parent_q_stm = if is_white_turn { parent_q } else { 1.0 - parent_q };
                parent_q_stm - config.fpu_reduction
            } else {
                let q_white = child.q_value();
                if is_white_turn { q_white } else { 1.0 - q_white }
            };

            // U = C(s) * P(s,a) * sqrt(N(s)) / (1 + N(s,a))
            let u = cpuct * child.prior * parent_visits.sqrt() / (1.0 + child_visits);

            let score = q + u;
            #[cfg(feature = "search-trace")]
            log::trace!("puct child={:?} q={q:.4} u={u:.4} score={score:.4} prior={:.4} visits={child_visits}", child_id, child.prior);
            (child_id, score)
        })
        .reduce(|(left_id, left_score), (right_id, right_score)| {
            if right_score > left_score { (right_id, right_score) } else { (left_id, left_score) }
        });

    let best_child = match best_scored {
        Some((id, _)) => id,
        None => node.children[0],
    };

    #[cfg(feature = "search-trace")]
    log::trace!("puct selected={:?}", best_child);

    best_child
}

/// Probability-weighted selection at a CHANCE node.
/// Samples proportional to Maia's distribution with temperature and floor.
/// Uses pre-allocated buffer from `SearchState` to avoid allocation.
fn select_chance(tree: &SearchTree, node_id: NodeId, config: &Config, state: &mut SearchState) -> NodeId {
    let node = &tree.nodes[node_id.index()];
    let children = &node.children;

    if children.is_empty() {
        return node_id;
    }

    state.chance_probs.clear();
    state.chance_probs.extend(
        children.iter().map(|&child_id| tree.nodes[child_id.index()].prior),
    );

    if (config.maia_temperature - 1.0).abs() > 1e-6 {
        let inv_t = 1.0 / config.maia_temperature;
        state.chance_probs.iter_mut().for_each(|probability| *probability = probability.powf(inv_t));
    }

    state.chance_probs.iter_mut().for_each(|probability| *probability = probability.max(config.maia_floor));

    let sum: f64 = state.chance_probs.iter().sum();
    if sum <= 0.0 {
        return children[0];
    }
    state.chance_probs.iter_mut().for_each(|probability| *probability /= sum);

    let random_value: f64 = rand::random();
    let found = state.chance_probs
        .iter()
        .scan(0.0, |cumulative, &prob| {
            *cumulative += prob;
            Some(*cumulative)
        })
        .enumerate()
        .find(|(_, cumulative_probability)| random_value < *cumulative_probability);

    match found {
        Some((index, _)) => children[index],
        // Fallback: floating-point rounding caused r >= cumulative sum; select last child.
        // children is guaranteed non-empty — select_chance is only called on expanded nodes.
        None => *children.last().unwrap(),
    }
}

#[cfg(test)]
mod tests {
    use crate::config::Config;
    use crate::search::{NodeId, NodeType, SearchState};
    use super::super::test_helpers::TreeBuilder;
    use super::{select, select_puct, select_chance};

    #[test]
    fn puct_with_all_unvisited_selects_highest_prior() {
        let tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.6, 0, 0.0)
            .with_child(NodeId(0), "d2d4", NodeType::Chance, 0.3, 0, 0.0)
            .with_child(NodeId(0), "g1f3", NodeType::Chance, 0.1, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let config = Config::default();
        let selected = select_puct(&tree, NodeId(0), &config);
        let selected_move = tree.get(selected).unwrap().move_uci.as_deref();
        assert_eq!(selected_move, Some("e2e4"));
    }

    #[test]
    fn puct_balances_exploitation_and_exploration() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 500, 0.6)
            .with_child(NodeId(0), "d2d4", NodeType::Chance, 0.5, 1, 0.55)
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 501;
            root.total_value = 0.6 * 500.0 + 0.55;
        }

        let config = Config::default();
        let selected = select_puct(&tree, NodeId(0), &config);
        let selected_move = tree.get(selected).unwrap().move_uci.as_deref();
        assert_eq!(selected_move, Some("d2d4"));
    }

    #[test]
    fn puct_fpu_uses_parent_q_minus_reduction() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.6, 0, 0.0)
            .with_child(NodeId(0), "d2d4", NodeType::Chance, 0.4, 0, 0.0)
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 100;
            root.total_value = 70.0; // Q = 0.7 → FPU = 0.7 - 0.3 = 0.4
        }

        let config = Config::default();
        let selected = select_puct(&tree, NodeId(0), &config);
        assert_eq!(tree.get(selected).unwrap().move_uci.as_deref(), Some("e2e4"));
    }

    #[test]
    fn puct_perspective_converts_q_for_black() {
        let mut tree = TreeBuilder::with_root("e2e4", NodeType::Max)
            .with_child(NodeId(0), "e7e5", NodeType::Chance, 0.5, 50, 0.3)
            .with_child(NodeId(0), "d7d5", NodeType::Chance, 0.5, 50, 0.6)
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 100;
            root.total_value = 45.0;
        }

        let config = Config::default();
        let selected = select_puct(&tree, NodeId(0), &config);
        let selected_move = tree.get(selected).unwrap().move_uci.as_deref();
        assert_eq!(selected_move, Some("e7e5")); // Q_white=0.3 → Q_black=0.7
    }

    #[test]
    fn chance_samples_proportional_to_prior() {
        let tree = TreeBuilder::with_root("", NodeType::Chance)
            .with_child(NodeId(0), "e7e5", NodeType::Max, 0.99, 0, 0.0)
            .with_child(NodeId(0), "d7d5", NodeType::Max, 0.01, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let config = Config::default();
        let mut state = SearchState::new();
        let mut e5_count = 0;
        for _ in 0..1000 {
            let selected = select_chance(&tree, NodeId(0), &config, &mut state);
            if tree.get(selected).unwrap().move_uci.as_deref() == Some("e7e5") {
                e5_count += 1;
            }
        }
        assert!(e5_count > 950, "e5_count was {e5_count}, expected >950");
    }

    #[test]
    fn chance_floor_prevents_zero_probability() {
        let tree = TreeBuilder::with_root("", NodeType::Chance)
            .with_child(NodeId(0), "e7e5", NodeType::Max, 0.98, 0, 0.0)
            .with_child(NodeId(0), "d7d5", NodeType::Max, 0.001, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let config = Config::default(); // floor = 0.01
        let mut state = SearchState::new();
        let mut d5_count = 0;
        for _ in 0..10000 {
            let selected = select_chance(&tree, NodeId(0), &config, &mut state);
            if tree.get(selected).unwrap().move_uci.as_deref() == Some("d7d5") {
                d5_count += 1;
            }
        }
        assert!(d5_count > 30, "d5_count was {d5_count}, expected >30 (floor should help)");
    }

    #[test]
    fn chance_temperature_spreads_distribution() {
        let tree = TreeBuilder::with_root("", NodeType::Chance)
            .with_child(NodeId(0), "e7e5", NodeType::Max, 0.9, 0, 0.0)
            .with_child(NodeId(0), "d7d5", NodeType::Max, 0.1, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let mut config = Config::default();
        config.maia_temperature = 3.0;
        config.maia_floor = 0.0;

        let mut state = SearchState::new();
        let mut d5_count = 0;
        for _ in 0..10000 {
            let selected = select_chance(&tree, NodeId(0), &config, &mut state);
            if tree.get(selected).unwrap().move_uci.as_deref() == Some("d7d5") {
                d5_count += 1;
            }
        }
        assert!(d5_count > 2500, "d5_count was {d5_count}, expected >2500 with T=3.0");
    }

    #[test]
    fn chance_floor_applied_after_temperature() {
        let tree = TreeBuilder::with_root("", NodeType::Chance)
            .with_child(NodeId(0), "e7e5", NodeType::Max, 0.95, 0, 0.0)
            .with_child(NodeId(0), "d7d5", NodeType::Max, 0.05, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let mut config = Config::default();
        config.maia_temperature = 0.5;
        config.maia_floor = 0.01;

        let mut state = SearchState::new();
        let mut d5_count = 0;
        for _ in 0..10000 {
            let selected = select_chance(&tree, NodeId(0), &config, &mut state);
            if tree.get(selected).unwrap().move_uci.as_deref() == Some("d7d5") {
                d5_count += 1;
            }
        }
        assert!(d5_count > 50, "d5_count was {d5_count}, floor should guarantee >0.5% even with sharpening");
    }

    #[test]
    fn select_returns_unexpanded_leaf() {
        let tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let config = Config::default();
        let mut state = SearchState::new();
        let leaf = select(&tree, &config, &mut state);
        assert_ne!(leaf, NodeId(0));
        assert!(!tree.get(leaf).unwrap().expanded);
    }

    #[test]
    fn select_returns_root_when_unexpanded() {
        let tree = TreeBuilder::new().build();
        let config = Config::default();
        let mut state = SearchState::new();
        let leaf = select(&tree, &config, &mut state);
        assert_eq!(leaf, NodeId(0));
    }
}
