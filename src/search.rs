use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::config::Config;
use crate::engine::lookup_castling_aware;

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
    chance_probs: Vec<f64>,
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

/// Select a leaf node from the tree using PUCT at MAX nodes and
/// probability-weighted sampling at CHANCE nodes.
pub fn select(tree: &SearchTree, config: &Config, state: &mut SearchState) -> NodeId {
    let mut current = tree.root_id;
    #[cfg(feature = "search-trace")]
    let mut depth = 0u32;

    loop {
        let node = &tree.nodes[current.index()];

        // If not expanded or terminal, this is the leaf
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

/// PUCT selection at a MAX node. Picks the child with the highest UCB score.
fn select_puct(tree: &SearchTree, node_id: NodeId, config: &Config) -> NodeId {
    let node = &tree.nodes[node_id.index()];
    let parent_visits = node.visit_count as f64;
    let is_white_turn = is_white_to_move_from_node(node);

    // Dynamic cpuct: C(s) = cpuct_init + cpuct_factor * ln((N(s) + cpuct_base) / cpuct_base)
    let cpuct = config.cpuct_init
        + config.cpuct_factor * ((parent_visits + config.cpuct_base) / config.cpuct_base).ln();

    let parent_q = node.q_value();

    let best_scored = node.children
        .iter()
        .map(|&child_id| {
            let child = &tree.nodes[child_id.index()];
            let child_visits = child.visit_count as f64;

            // Q value from side-to-move perspective
            let q = if child.visit_count == 0 {
                // FPU: Q_fpu = Q_parent_stm - fpu_reduction
                // parent_q is White's perspective; convert to side-to-move first
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
        .reduce(|(id_a, score_a), (id_b, score_b)| {
            if score_b > score_a { (id_b, score_b) } else { (id_a, score_a) }
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

    // Reuse pre-allocated buffer
    state.chance_probs.clear();
    state.chance_probs.extend(
        children.iter().map(|&cid| tree.nodes[cid.index()].prior),
    );

    // Apply temperature (before floor, per docs)
    if (config.maia_temperature - 1.0).abs() > 1e-6 {
        let inv_t = 1.0 / config.maia_temperature;
        state.chance_probs.iter_mut().for_each(|p| *p = p.powf(inv_t));
    }

    // Apply exploration floor (after temperature, guarantees minimum probability)
    state.chance_probs.iter_mut().for_each(|p| *p = p.max(config.maia_floor));

    // Normalize
    let sum: f64 = state.chance_probs.iter().sum();
    if sum <= 0.0 {
        return children[0];
    }
    state.chance_probs.iter_mut().for_each(|p| *p /= sum);

    // Sample
    let r: f64 = rand::random();
    let found = state.chance_probs
        .iter()
        .scan(0.0, |cumulative, &prob| {
            *cumulative += prob;
            Some(*cumulative)
        })
        .enumerate()
        .find(|(_, cumulative)| r < *cumulative);

    match found {
        Some((i, _)) => children[i],
        None => *children.last().unwrap(),
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

/// Determine candidate moves at a MAX node.
/// Returns (uci_move, blended_prior) pairs.
pub fn candidate_moves_max(
    engine_policy: &HashMap<String, f32>,
    maia_policy: &HashMap<String, f32>,
    config: &Config,
) -> Vec<(String, f64)> {
    // Top N engine moves by policy (NN's prior for move strength)
    let mut engine_sorted: Vec<_> = engine_policy.iter().collect();
    engine_sorted.sort_by(|a, b| match b.1.partial_cmp(a.1) {
        Some(ord) => ord,
        None => std::cmp::Ordering::Equal,
    });
    let engine_top: Vec<&String> = engine_sorted
        .iter()
        .take(config.engine_top_n)
        .map(|(m, _)| *m)
        .collect();

    // Top N Maia moves by policy
    let mut maia_sorted: Vec<_> = maia_policy.iter().collect();
    maia_sorted.sort_by(|a, b| match b.1.partial_cmp(a.1) {
        Some(ord) => ord,
        None => std::cmp::Ordering::Equal,
    });
    let maia_top: Vec<&String> = maia_sorted
        .iter()
        .take(config.maia_top_n)
        .map(|(m, _)| *m)
        .collect();

    // Deduplicate
    let mut seen = HashSet::new();
    let mut candidates: Vec<(String, f64)> = engine_top
        .iter()
        .chain(maia_top.iter())
        .filter(|uci| seen.insert(uci.to_string()))
        .map(|uci| {
            let engine_p = match lookup_castling_aware(uci, engine_policy) {
                Some(v) => v,
                None => 0.0,
            } as f64 / 100.0;
            let maia_p = match lookup_castling_aware(uci, maia_policy) {
                Some(v) => v,
                None => 0.0,
            } as f64 / 100.0;

            // Blended prior: alpha * engine + (1 - alpha) * maia
            let blended = config.alpha * engine_p + (1.0 - config.alpha) * maia_p;
            ((*uci).clone(), blended)
        })
        .collect();

    // Normalize priors so they sum to 1
    let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
    if sum > 0.0 {
        candidates.iter_mut().for_each(|(_, p)| *p /= sum);
    }

    candidates
}

/// Determine candidate moves at a CHANCE node from Maia policy.
/// Returns (uci_move, maia_probability) pairs, filtered by min_prob.
pub fn candidate_moves_chance(
    maia_policy: &HashMap<String, f32>,
    config: &Config,
) -> Vec<(String, f64)> {
    let mut candidates: Vec<(String, f64)> = maia_policy
        .iter()
        .filter(|(_, p)| (**p as f64 / 100.0) >= config.maia_min_prob)
        .map(|(m, p)| (m.clone(), *p as f64 / 100.0))
        .collect();

    // Normalize
    let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
    if sum > 0.0 {
        candidates.iter_mut().for_each(|(_, p)| *p /= sum);
    }

    candidates
}

/// Select the best move at the root for final recommendation.
/// Uses safety parameter to blend expected score with worst-case.
#[allow(dead_code)] // Used by integration tests (separate crate, invisible to lint)
pub fn best_root_move(tree: &SearchTree, config: &Config) -> Option<RootMoveInfo> {
    let mut infos = root_move_infos(tree, config);
    infos.retain(|i| i.visits > 0);
    infos.into_iter().next()
}

/// Get all root move infos sorted by practical Q.
pub fn root_move_infos(tree: &SearchTree, config: &Config) -> Vec<RootMoveInfo> {
    let root = tree.root();
    let is_white = is_white_to_move_from_node(root);

    // Position-level engine value (from WDL) in side-to-move perspective [0, 1].
    // Used for delta: how much does our practical Q differ from the engine's position eval?
    let engine_position_value = match root.wdl {
        Some((w, d, _l)) => Some(w as f64 / 1000.0 + config.contempt * d as f64 / 1000.0),
        None => None,
    };

    let mut move_infos: Vec<RootMoveInfo> = root
        .children
        .iter()
        .filter_map(|&child_id| {
            let child = tree.get(child_id)?;
            let uci = child.move_uci.as_ref()?.clone();
            let visits = child.visit_count;
            let q_white = child.q_value();
            let q_stm = if is_white { q_white } else { 1.0 - q_white };

            // Engine policy percentage for this move
            let engine_pol = match root.engine_policy.as_ref() {
                Some(pol) => match lookup_castling_aware(&uci, pol) {
                    Some(p) => Some(p as f64),
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

            // Delta: how much does our practical Q differ from the position's engine eval?
            // Positive delta = move performs better against humans than the engine expects.
            let delta = match engine_position_value {
                Some(ev) => Some(practical_q - ev),
                None => None,
            };

            Some(RootMoveInfo {
                uci_move: uci,
                node_id: child_id,
                visits,
                engine_policy: engine_pol,
                practical_q,
                delta,
                q_white,
                worst_case,
                wdl: child.wdl,
            })
        })
        .collect();

    move_infos.sort_by(|a, b| match b.practical_q.partial_cmp(&a.practical_q) {
        Some(ord) => ord,
        None => std::cmp::Ordering::Equal,
    });

    move_infos
}

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
    pub q_white: f64,
    pub worst_case: f64,
    pub wdl: Option<(u32, u32, u32)>,
}

/// Compute worst-case value against likely opponent responses.
fn worst_case_value(tree: &SearchTree, node_id: NodeId, is_white: bool) -> f64 {
    let node = &tree.nodes[node_id.index()];
    if node.children.is_empty() {
        let q = node.q_value();
        return if is_white { q } else { 1.0 - q };
    }

    // Worst-case among opponent responses with >10% Maia probability (prior > 0.10)
    let qualifying_children = node.children
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

    match qualifying_children.fold(None, |acc: Option<f64>, q| Some(match acc {
        Some(prev) => prev.min(q),
        None => q,
    })) {
        Some(worst) => worst,
        None => {
            let q = node.q_value();
            if is_white { q } else { 1.0 - q }
        }
    }
}

/// Heuristic: determine if White is to move based on the move sequence.
#[inline]
fn is_white_to_move_from_node(node: &Node) -> bool {
    if node.move_sequence.is_empty() {
        true
    } else {
        // Count moves: even number = White's turn
        node.move_sequence.split_whitespace().count() % 2 == 0
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::Config;

    use super::{
        backpropagate, candidate_moves_chance, candidate_moves_max,
        is_white_to_move_from_node, root_move_infos, select, select_chance, select_puct,
        worst_case_value, Node, NodeId, NodeType, SearchState, SearchTree,
    };

    // ── TreeBuilder ──────────────────────────────────────────────────────

    /// Builder for constructing test trees with pre-set visit counts and values.
    struct TreeBuilder {
        tree: SearchTree,
    }

    impl TreeBuilder {
        fn new() -> Self {
            Self {
                tree: SearchTree::new("startpos".to_string(), String::new(), NodeType::Max),
            }
        }

        fn with_root(move_seq: &str, node_type: NodeType) -> Self {
            Self {
                tree: SearchTree::new("pos".to_string(), move_seq.to_string(), node_type),
            }
        }

        /// Add a child to the given parent with preset visits and Q value.
        fn with_child(
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
        fn expanded(mut self, node_id: NodeId) -> Self {
            self.tree.get_mut(node_id).unwrap().expanded = true;
            self
        }

        fn build(self) -> SearchTree {
            self.tree
        }
    }

    // ── PUCT Selection ───────────────────────────────────────────────────

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
        // d2d4 has far fewer visits — its U term should win
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
        // Higher prior wins when both have equal FPU Q
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

    // ── CHANCE Selection ─────────────────────────────────────────────────

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
            .with_child(NodeId(0), "d7d5", NodeType::Max, 0.001, 0, 0.0) // Below floor
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

        // High temperature spreads mass
        let mut config = Config::default();
        config.maia_temperature = 3.0;
        config.maia_floor = 0.0; // Disable floor to isolate temperature effect

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
        config.maia_temperature = 0.5; // Sharpening
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

    // ── Selection (full tree traversal) ──────────────────────────────────

    #[test]
    fn select_returns_unexpanded_leaf() {
        let tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 0, 0.0)
            .expanded(NodeId(0))
            .build();

        let config = Config::default();
        let mut state = SearchState::new();
        let leaf = select(&tree, &config, &mut state);
        // Should return the unexpanded child
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

    // ── Backpropagation ──────────────────────────────────────────────────

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
        assert!((tree.root().q_value() - 0.6).abs() < 0.001); // (0.8 + 0.4) / 2
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
        // All should have the same value (backprop always uses White's perspective)
        assert!((tree.root().q_value() - 0.6).abs() < 0.001);
    }

    // ── Candidate Moves (MAX) ────────────────────────────────────────────

    #[test]
    fn max_candidates_deduplicates_engine_and_maia() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 50.0f32);
        engine_policy.insert("d2d4".to_string(), 30.0);
        engine_policy.insert("g1f3".to_string(), 10.0);

        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 40.0f32); // Overlap with engine
        maia.insert("d2d4".to_string(), 35.0); // Overlap
        maia.insert("b1c3".to_string(), 10.0);
        maia.insert("g1f3".to_string(), 8.0); // Overlap
        maia.insert("c2c4".to_string(), 4.0);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        // 3 engine + 5 maia but e2e4, d2d4, g1f3 overlap → 5 unique
        assert_eq!(candidates.len(), 5);

        let moves: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();
        assert!(moves.contains(&"e2e4"));
        assert!(moves.contains(&"b1c3"));
        assert!(moves.contains(&"c2c4"));
    }

    #[test]
    fn max_candidates_priors_sum_to_one() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 50.0f32);
        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 80.0f32);
        maia.insert("d2d4".to_string(), 20.0);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn max_candidates_blends_engine_and_maia_priors() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 100.0f32); // 100% engine policy
        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 100.0f32); // 100% maia policy

        let config = Config::default(); // alpha = 0.7
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        // Only one move — prior should be 1.0 after normalization
        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].1 - 1.0).abs() < 0.001);
    }

    // ── Candidate Moves (CHANCE) ─────────────────────────────────────────

    #[test]
    fn chance_candidates_filters_below_min_prob() {
        let mut maia = HashMap::new();
        maia.insert("e7e5".to_string(), 40.0f32);
        maia.insert("c7c5".to_string(), 20.0);
        maia.insert("e7e6".to_string(), 15.0);
        maia.insert("a7a6".to_string(), 0.005); // 0.005% → 0.00005 < maia_min_prob(0.001)

        let config = Config::default();
        let candidates = candidate_moves_chance(&maia, &config);

        assert_eq!(candidates.len(), 3);
        let moves: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();
        assert!(!moves.contains(&"a7a6"));
    }

    #[test]
    fn chance_candidates_normalizes_to_one() {
        let mut maia = HashMap::new();
        maia.insert("e7e5".to_string(), 60.0f32);
        maia.insert("d7d5".to_string(), 30.0);

        let config = Config::default();
        let candidates = candidate_moves_chance(&maia, &config);

        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    // ── Worst-Case Value ─────────────────────────────────────────────────

    #[test]
    fn worst_case_filters_low_prior_children() {
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "e2e4", NodeType::Chance, 0.5, 10, 0.5)
            .expanded(NodeId(0))
            .build();

        let chance_id = tree.root().children[0];
        tree.get_mut(chance_id).unwrap().expanded = true;

        // High-prior response: Q_white=0.6 (decent for White)
        let good_id = tree.add_child(
            chance_id, "e7e5".to_string(), NodeType::Max,
            "p1".to_string(), "e2e4 e7e5".to_string(), 0.8,
        );
        tree.get_mut(good_id).unwrap().visit_count = 5;
        tree.get_mut(good_id).unwrap().total_value = 3.0; // Q=0.6

        // Low-prior response: Q_white=0.2 (bad for White)
        let bad_id = tree.add_child(
            chance_id, "d7d5".to_string(), NodeType::Max,
            "p2".to_string(), "e2e4 d7d5".to_string(), 0.05, // Below 10%
        );
        tree.get_mut(bad_id).unwrap().visit_count = 3;
        tree.get_mut(bad_id).unwrap().total_value = 0.6; // Q=0.2

        let worst = worst_case_value(&tree, chance_id, true);
        // Should only consider e7e5 (prior=0.8 > 0.10), ignoring d7d5 (prior=0.05)
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

        // Only child has prior=0.05 (below 10% threshold)
        let child_id = tree.add_child(
            chance_id, "e7e5".to_string(), NodeType::Max,
            "p".to_string(), "e2e4 e7e5".to_string(), 0.05,
        );
        tree.get_mut(child_id).unwrap().visit_count = 3;
        tree.get_mut(child_id).unwrap().total_value = 0.6;

        let worst = worst_case_value(&tree, chance_id, true);
        // No children qualify → falls back to node's own Q
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
        tree.get_mut(c1).unwrap().total_value = 3.5; // Q=0.7

        let c2 = tree.add_child(
            chance_id, "c7c5".to_string(), NodeType::Max,
            "p2".to_string(), "e2e4 c7c5".to_string(), 0.3,
        );
        tree.get_mut(c2).unwrap().visit_count = 5;
        tree.get_mut(c2).unwrap().total_value = 2.0; // Q=0.4

        let worst = worst_case_value(&tree, chance_id, true);
        assert!((worst - 0.4).abs() < 0.001); // min(0.7, 0.4) = 0.4
    }

    // ── Root Move Infos ──────────────────────────────────────────────────

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
        assert_eq!(infos[0].uci_move, "d2d4"); // Highest Q_white=0.7
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

        // Both moves should be in the list (unvisited ones still show)
        assert_eq!(infos.len(), 2);
    }

    #[test]
    fn root_move_infos_safety_blends_q_with_worst_case() {
        // Child with high average Q but bad worst-case should score lower with high safety
        let mut tree = TreeBuilder::new()
            .with_child(NodeId(0), "risky", NodeType::Chance, 0.5, 50, 0.7) // High average
            .with_child(NodeId(0), "safe", NodeType::Chance, 0.5, 50, 0.6) // Lower average
            .expanded(NodeId(0))
            .build();
        {
            let root = tree.get_mut(NodeId(0)).unwrap();
            root.visit_count = 100;
            root.total_value = 65.0;
        }

        // Add opponent responses to "risky" — one devastating
        let risky_id = tree.root().children[0];
        tree.get_mut(risky_id).unwrap().expanded = true;
        let r1 = tree.add_child(
            risky_id, "e7e5".to_string(), NodeType::Max,
            "p".to_string(), "risky e7e5".to_string(), 0.5,
        );
        tree.get_mut(r1).unwrap().visit_count = 10;
        tree.get_mut(r1).unwrap().total_value = 2.0; // Q=0.2 (disaster)
        let r2 = tree.add_child(
            risky_id, "d7d5".to_string(), NodeType::Max,
            "p2".to_string(), "risky d7d5".to_string(), 0.5,
        );
        tree.get_mut(r2).unwrap().visit_count = 10;
        tree.get_mut(r2).unwrap().total_value = 9.0; // Q=0.9

        let mut config = Config::default();
        config.safety = 0.8; // Very conservative

        let infos = root_move_infos(&tree, &config);
        // "safe" should rank higher because "risky" has a devastating worst-case
        let safe_info = infos.iter().find(|i| i.uci_move == "safe").unwrap();
        let risky_info = infos.iter().find(|i| i.uci_move == "risky").unwrap();
        assert!(safe_info.practical_q > risky_info.practical_q,
            "safe={:.3} should beat risky={:.3} with high safety",
            safe_info.practical_q, risky_info.practical_q);
    }

    // ── Perspective / Value Handling ──────────────────────────────────────

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
