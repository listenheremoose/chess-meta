use std::collections::HashMap;

use crate::config::Config;
use crate::engine::lookup_castling_aware;

/// Node type in the MCTS tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeType {
    /// Our turn — select via PUCT.
    Max,
    /// Opponent's turn — sample proportional to Maia distribution.
    Chance,
}

/// A single node in the MCTS tree.
#[derive(Debug, Clone)]
pub struct Node {
    pub id: u64,
    pub parent: Option<u64>,
    pub move_uci: Option<String>,
    pub node_type: NodeType,
    pub epd: String,
    pub move_sequence: String,

    pub visit_count: u64,
    pub total_value: f64,
    pub prior: f64,

    pub children: Vec<u64>,
    pub expanded: bool,

    /// Engine policy (percentage, 0-100) per move from this position.
    pub engine_policy: Option<HashMap<String, f32>>,
    /// Maia policy (percentage, 0-100) per move from this position.
    pub maia_policy: Option<HashMap<String, f32>>,
    /// WDL from engine eval.
    pub wdl: Option<(u32, u32, u32)>,
    /// Terminal value if game is over.
    pub terminal_value: Option<f64>,
}

impl Node {
    pub fn new(
        id: u64,
        parent: Option<u64>,
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
            maia_policy: None,
            wdl: None,
            terminal_value: None,
        }
    }

    /// Average value from White's perspective.
    pub fn q_value(&self) -> f64 {
        if self.visit_count == 0 {
            0.5
        } else {
            self.total_value / self.visit_count as f64
        }
    }
}

/// The full MCTS tree, stored as a flat arena.
pub struct SearchTree {
    pub nodes: HashMap<u64, Node>,
    pub root_id: u64,
    next_id: u64,
}

impl SearchTree {
    pub fn new(root_epd: String, root_move_sequence: String, root_type: NodeType) -> Self {
        let root = Node::new(0, None, None, root_type, root_epd, root_move_sequence);
        let mut nodes = HashMap::new();
        nodes.insert(0, root);
        Self {
            nodes,
            root_id: 0,
            next_id: 1,
        }
    }

    pub fn root(&self) -> &Node {
        &self.nodes[&self.root_id]
    }

    pub fn root_mut(&mut self) -> &mut Node {
        self.nodes.get_mut(&self.root_id).unwrap()
    }

    pub fn get(&self, id: u64) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn get_mut(&mut self, id: u64) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    /// Add a child node and return its ID.
    pub fn add_child(
        &mut self,
        parent_id: u64,
        move_uci: String,
        node_type: NodeType,
        epd: String,
        move_sequence: String,
        prior: f64,
    ) -> u64 {
        let id = self.next_id;
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

        self.nodes.insert(id, node);
        self.nodes.get_mut(&parent_id).unwrap().children.push(id);
        id
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }
}

/// Select a leaf node from the tree using PUCT at MAX nodes and
/// probability-weighted sampling at CHANCE nodes.
pub fn select(tree: &SearchTree, config: &Config) -> u64 {
    let mut current = tree.root_id;

    loop {
        let node = &tree.nodes[&current];

        // If not expanded or terminal, this is the leaf
        if !node.expanded || node.children.is_empty() {
            return current;
        }

        match node.node_type {
            NodeType::Max => {
                current = select_puct(tree, current, config);
            }
            NodeType::Chance => {
                current = select_chance(tree, current, config);
            }
        }
    }
}

/// PUCT selection at a MAX node. Picks the child with the highest UCB score.
fn select_puct(tree: &SearchTree, node_id: u64, config: &Config) -> u64 {
    let node = &tree.nodes[&node_id];
    let parent_visits = node.visit_count as f64;
    let is_white_turn = is_white_to_move_from_node(node);

    // Dynamic cpuct: C(s) = cpuct_init + cpuct_factor * ln((N(s) + cpuct_base) / cpuct_base)
    let cpuct = config.cpuct_init
        + config.cpuct_factor * ((parent_visits + config.cpuct_base) / config.cpuct_base).ln();

    let parent_q = node.q_value();

    let mut best_score = f64::NEG_INFINITY;
    let mut best_child = node.children[0];

    for &child_id in &node.children {
        let child = &tree.nodes[&child_id];
        let child_visits = child.visit_count as f64;

        // Q value from side-to-move perspective
        let q = if child.visit_count == 0 {
            // FPU: first play urgency
            let fpu = if is_white_turn {
                parent_q - config.fpu_reduction
            } else {
                parent_q + config.fpu_reduction
            };
            fpu
        } else {
            let q_white = child.q_value();
            if is_white_turn {
                q_white
            } else {
                1.0 - q_white
            }
        };

        // U = C(s) * P(s,a) * sqrt(N(s)) / (1 + N(s,a))
        let u = cpuct * child.prior * parent_visits.sqrt() / (1.0 + child_visits);

        let score = q + u;
        if score > best_score {
            best_score = score;
            best_child = child_id;
        }
    }

    best_child
}

/// Probability-weighted selection at a CHANCE node.
/// Samples proportional to Maia's distribution with temperature and floor.
fn select_chance(tree: &SearchTree, node_id: u64, config: &Config) -> u64 {
    let node = &tree.nodes[&node_id];
    let children = &node.children;

    if children.is_empty() {
        return node_id;
    }

    // Build distribution from priors (already set from Maia during expansion)
    let mut probs: Vec<f64> = children
        .iter()
        .map(|&cid| {
            let child = &tree.nodes[&cid];
            child.prior.max(config.maia_floor)
        })
        .collect();

    // Apply temperature
    if (config.maia_temperature - 1.0).abs() > 1e-6 {
        let inv_t = 1.0 / config.maia_temperature;
        for p in &mut probs {
            *p = p.powf(inv_t);
        }
    }

    // Normalize
    let sum: f64 = probs.iter().sum();
    if sum <= 0.0 {
        return children[0];
    }
    for p in &mut probs {
        *p /= sum;
    }

    // Sample
    let r: f64 = rand::random();
    let mut cumulative = 0.0;
    for (i, &prob) in probs.iter().enumerate() {
        cumulative += prob;
        if r < cumulative {
            return children[i];
        }
    }

    *children.last().unwrap()
}

/// Backpropagate a value (from White's perspective) up the tree.
pub fn backpropagate(tree: &mut SearchTree, leaf_id: u64, value_white: f64) {
    let mut current = Some(leaf_id);
    while let Some(id) = current {
        let node = tree.nodes.get_mut(&id).unwrap();
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
    // Top N engine moves by policy
    let mut engine_sorted: Vec<_> = engine_policy.iter().collect();
    engine_sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    let engine_top: Vec<&String> = engine_sorted
        .iter()
        .take(config.engine_top_n)
        .map(|(m, _)| *m)
        .collect();

    // Top N Maia moves by policy
    let mut maia_sorted: Vec<_> = maia_policy.iter().collect();
    maia_sorted.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap_or(std::cmp::Ordering::Equal));
    let maia_top: Vec<&String> = maia_sorted
        .iter()
        .take(config.maia_top_n)
        .map(|(m, _)| *m)
        .collect();

    // Deduplicate
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();

    for uci in engine_top.iter().chain(maia_top.iter()) {
        if !seen.insert((*uci).clone()) {
            continue;
        }

        let engine_p = lookup_castling_aware(uci, engine_policy).unwrap_or(0.0) as f64 / 100.0;
        let maia_p = lookup_castling_aware(uci, maia_policy).unwrap_or(0.0) as f64 / 100.0;

        // Blended prior: alpha * engine + (1 - alpha) * maia
        let blended = config.alpha * engine_p + (1.0 - config.alpha) * maia_p;
        candidates.push(((*uci).clone(), blended));
    }

    // Normalize priors so they sum to 1
    let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
    if sum > 0.0 {
        for (_, p) in &mut candidates {
            *p /= sum;
        }
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
        for (_, p) in &mut candidates {
            *p /= sum;
        }
    }

    candidates
}

/// Select the best move at the root for final recommendation.
/// Uses safety parameter to blend expected score with worst-case.
pub fn best_root_move(tree: &SearchTree, config: &Config) -> Option<RootMoveInfo> {
    let root = tree.root();
    if root.children.is_empty() {
        return None;
    }

    let is_white = is_white_to_move_from_node(root);

    let mut move_infos: Vec<RootMoveInfo> = root
        .children
        .iter()
        .filter_map(|&child_id| {
            let child = tree.get(child_id)?;
            let uci = child.move_uci.as_ref()?.clone();
            let visits = child.visit_count;
            if visits == 0 {
                return None;
            }
            let q_white = child.q_value();
            let q_stm = if is_white { q_white } else { 1.0 - q_white };

            // Engine Q for this move (from root's engine eval)
            let engine_q = root
                .engine_policy
                .as_ref()
                .and_then(|_| root.wdl.map(|wdl| {
                    let v = wdl.0 as f64 / 1000.0 + config.contempt * wdl.1 as f64 / 1000.0;
                    if is_white { v } else { 1.0 - v }
                }));

            // Worst-case: minimum Q among likely opponent responses
            let worst_case = worst_case_value(tree, child_id, is_white);

            // Practical Q: blend expected with worst-case
            let practical_q = (1.0 - config.safety) * q_stm + config.safety * worst_case;

            let delta = engine_q.map(|eq| practical_q - eq);

            Some(RootMoveInfo {
                uci_move: uci,
                node_id: child_id,
                visits,
                engine_q,
                practical_q,
                delta,
                q_white,
                worst_case,
                wdl: child.wdl,
            })
        })
        .collect();

    move_infos.sort_by(|a, b| {
        b.practical_q
            .partial_cmp(&a.practical_q)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    move_infos.into_iter().next()
}

/// Get all root move infos sorted by practical Q.
pub fn root_move_infos(tree: &SearchTree, config: &Config) -> Vec<RootMoveInfo> {
    let root = tree.root();
    let is_white = is_white_to_move_from_node(root);

    let mut move_infos: Vec<RootMoveInfo> = root
        .children
        .iter()
        .filter_map(|&child_id| {
            let child = tree.get(child_id)?;
            let uci = child.move_uci.as_ref()?.clone();
            let visits = child.visit_count;
            let q_white = child.q_value();
            let q_stm = if is_white { q_white } else { 1.0 - q_white };

            let engine_q = root.wdl.map(|wdl| {
                let v = wdl.0 as f64 / 1000.0 + config.contempt * wdl.1 as f64 / 1000.0;
                if is_white { v } else { 1.0 - v }
            });

            let worst_case = if visits > 0 {
                worst_case_value(tree, child_id, is_white)
            } else {
                q_stm
            };
            let practical_q = (1.0 - config.safety) * q_stm + config.safety * worst_case;
            let delta = engine_q.map(|eq| practical_q - eq);

            Some(RootMoveInfo {
                uci_move: uci,
                node_id: child_id,
                visits,
                engine_q,
                practical_q,
                delta,
                q_white,
                worst_case,
                wdl: child.wdl,
            })
        })
        .collect();

    move_infos.sort_by(|a, b| {
        b.practical_q
            .partial_cmp(&a.practical_q)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    move_infos
}

/// Information about a root candidate move, for UI display.
#[derive(Debug, Clone)]
pub struct RootMoveInfo {
    pub uci_move: String,
    pub node_id: u64,
    pub visits: u64,
    pub engine_q: Option<f64>,
    pub practical_q: f64,
    pub delta: Option<f64>,
    pub q_white: f64,
    pub worst_case: f64,
    pub wdl: Option<(u32, u32, u32)>,
}

/// Compute worst-case value against likely opponent responses.
fn worst_case_value(tree: &SearchTree, node_id: u64, is_white: bool) -> f64 {
    let node = &tree.nodes[&node_id];
    if node.children.is_empty() {
        let q = node.q_value();
        return if is_white { q } else { 1.0 - q };
    }

    // Look at opponent response children (CHANCE nodes have children representing our next move)
    let mut worst = f64::MAX;
    let mut any_visited = false;

    for &child_id in &node.children {
        let child = &tree.nodes[&child_id];
        if child.visit_count > 0 {
            any_visited = true;
            let child_q_stm = if is_white {
                child.q_value()
            } else {
                1.0 - child.q_value()
            };
            worst = worst.min(child_q_stm);
        }
    }

    if any_visited {
        worst
    } else {
        let q = node.q_value();
        if is_white { q } else { 1.0 - q }
    }
}

/// Heuristic: determine if White is to move based on the move sequence.
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
    use super::*;

    #[test]
    fn test_candidate_moves_max() {
        let mut engine = HashMap::new();
        engine.insert("e2e4".to_string(), 50.0f32);
        engine.insert("d2d4".to_string(), 30.0);
        engine.insert("g1f3".to_string(), 10.0);
        engine.insert("c2c4".to_string(), 5.0);

        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 40.0f32);
        maia.insert("d2d4".to_string(), 35.0);
        maia.insert("b1c3".to_string(), 10.0);
        maia.insert("g1f3".to_string(), 8.0);
        maia.insert("c2c4".to_string(), 4.0);
        maia.insert("e2e3".to_string(), 3.0);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine, &maia, &config);

        // Should have top 3 engine + top 5 maia, deduped
        assert!(!candidates.is_empty());
        assert!(candidates.len() <= 8);

        // Priors should sum to ~1
        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_candidate_moves_chance() {
        let mut maia = HashMap::new();
        maia.insert("e7e5".to_string(), 40.0f32);
        maia.insert("c7c5".to_string(), 20.0);
        maia.insert("e7e6".to_string(), 15.0);
        maia.insert("a7a6".to_string(), 0.005); // Below 0.1% threshold

        let config = Config::default();
        let candidates = candidate_moves_chance(&maia, &config);

        // a7a6 should be filtered out (0.005% < 0.1%)
        assert_eq!(candidates.len(), 3);
    }

    #[test]
    fn test_backpropagate() {
        let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
        let child_id = tree.add_child(
            0,
            "e2e4".to_string(),
            NodeType::Chance,
            "after_e4".to_string(),
            "e2e4".to_string(),
            0.5,
        );

        backpropagate(&mut tree, child_id, 0.7);

        assert_eq!(tree.root().visit_count, 1);
        assert!((tree.root().q_value() - 0.7).abs() < 0.001);
        assert_eq!(tree.get(child_id).unwrap().visit_count, 1);
    }
}
