use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::Instant;

use crate::cache::Cache;
use crate::config::Config;
use crate::engine::{Engine, EngineEval};
use crate::maia::MaiaEngine;
use crate::position::PositionState;
use crate::search::{
    NodeType, RootMoveInfo, SearchTree, backpropagate, candidate_moves_chance,
    candidate_moves_max, root_move_infos, select,
};

/// Snapshot of search state sent to the UI on each update.
#[derive(Debug, Clone)]
pub struct SearchSnapshot {
    pub iteration: u64,
    pub elapsed_secs: f64,
    pub root_moves: Vec<RootMoveInfo>,
    pub best_move: Option<String>,
    pub node_count: usize,
    pub iterations_per_sec: f64,
    pub best_move_history: Vec<(u64, String)>,
    pub q_history: Vec<(u64, f64)>,
    pub tree_snapshot: Option<TreeSnapshot>,
}

/// Minimal tree data for the tree view panel.
#[derive(Debug, Clone)]
pub struct TreeSnapshot {
    pub nodes: Vec<TreeNodeInfo>,
}

#[derive(Debug, Clone)]
pub struct TreeNodeInfo {
    pub id: u64,
    pub parent_id: Option<u64>,
    pub move_uci: Option<String>,
    pub node_type: NodeType,
    pub visit_count: u64,
    pub q_value: f64,
    pub depth: u32,
}

/// Controls for the MCTS search running in a background thread.
pub struct Coordinator {
    cancel: Option<Arc<AtomicBool>>,
    receiver: Option<mpsc::Receiver<SearchSnapshot>>,
    pub latest_snapshot: Option<SearchSnapshot>,
    pub running: bool,
}

impl Coordinator {
    pub fn new() -> Self {
        Self {
            cancel: None,
            receiver: None,
            latest_snapshot: None,
            running: false,
        }
    }

    /// Start the MCTS search in a background thread.
    pub fn start(&mut self, move_sequence: String, config: Config) {
        self.stop();

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);
        let (tx, rx) = mpsc::channel();

        self.cancel = Some(cancel);
        self.receiver = Some(rx);
        self.running = true;
        self.latest_snapshot = None;

        thread::spawn(move || {
            run_mcts(move_sequence, config, cancel_clone, tx);
        });
    }

    /// Pause/stop the search.
    pub fn stop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        self.running = false;
        self.cancel = None;
        self.receiver = None;
    }

    /// Poll for new snapshots. Returns true if a new snapshot was received.
    pub fn poll(&mut self) -> bool {
        let mut updated = false;
        if let Some(rx) = &self.receiver {
            // Drain all pending snapshots, keep the latest
            loop {
                match rx.try_recv() {
                    Ok(snapshot) => {
                        self.latest_snapshot = Some(snapshot);
                        updated = true;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        self.running = false;
                        self.receiver = None;
                        break;
                    }
                }
            }
        }
        updated
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Main MCTS loop running in a background thread.
fn run_mcts(
    move_sequence: String,
    config: Config,
    cancel: Arc<AtomicBool>,
    tx: mpsc::Sender<SearchSnapshot>,
) {
    // Initialize position
    let position = match PositionState::from_moves(&move_sequence) {
        Ok(p) => p,
        Err(e) => {
            log::error!("Invalid position: {e}");
            return;
        }
    };

    // Initialize engines
    let mut engine = match Engine::new(
        &config.lc0_path,
        &config.engine_weights_path,
        config.nn_cache_size_mb,
        config.ucinewgame_interval,
    ) {
        Ok(e) => e,
        Err(e) => {
            log::error!("Failed to start engine: {e}");
            return;
        }
    };

    let mut maia = match MaiaEngine::new(&config.lc0_path, &config.maia_weights_path, config.ucinewgame_interval) {
        Ok(m) => m,
        Err(e) => {
            log::error!("Failed to start Maia: {e}");
            return;
        }
    };

    // Open cache
    let cache = Cache::open().ok();

    // Initialize search tree
    // Root is always MAX — we're deciding our move regardless of which color we play
    let root_type = NodeType::Max;
    let mut tree = SearchTree::new(position.epd.clone(), position.move_sequence.clone(), root_type);

    let start_time = Instant::now();
    let mut best_move_history: Vec<(u64, String)> = Vec::new();
    let mut q_history: Vec<(u64, f64)> = Vec::new();
    let update_interval = 50; // Send snapshot every N iterations

    for iteration in 0..config.max_iterations {
        if cancel.load(Ordering::Relaxed) {
            break;
        }

        // 1. SELECT — traverse tree to a leaf
        let leaf_id = select(&tree, &config);

        // 2. EXPAND & EVALUATE the leaf
        let value = match expand_and_evaluate(
            &mut tree,
            leaf_id,
            &config,
            &mut engine,
            &mut maia,
            cache.as_ref(),
        ) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Expand/evaluate error at iteration {iteration}: {e}");
                break;
            }
        };

        // 3. BACKPROPAGATE
        backpropagate(&mut tree, leaf_id, value);

        // 4. Send periodic updates to UI
        if (iteration + 1) % update_interval as u64 == 0 || iteration == config.max_iterations - 1 {
            let elapsed = start_time.elapsed().as_secs_f64();
            let moves = root_move_infos(&tree, &config);
            let best = moves.first().map(|m| m.uci_move.clone());

            if let Some(ref bm) = best {
                if best_move_history.last().map(|(_, m)| m) != Some(bm) {
                    best_move_history.push((iteration, bm.clone()));
                }
            }
            if let Some(ref bm_info) = moves.first() {
                q_history.push((iteration, bm_info.practical_q));
            }

            let tree_snap = build_tree_snapshot(&tree, 10); // min 10 visits for tree view

            let snapshot = SearchSnapshot {
                iteration: iteration + 1,
                elapsed_secs: elapsed,
                root_moves: moves,
                best_move: best,
                node_count: tree.node_count(),
                iterations_per_sec: (iteration + 1) as f64 / elapsed.max(0.001),
                best_move_history: best_move_history.clone(),
                q_history: q_history.clone(),
                tree_snapshot: Some(tree_snap),
            };

            if tx.send(snapshot).is_err() {
                break; // Receiver dropped
            }
        }
    }
}

/// Expand a leaf node and return its evaluation (from White's perspective).
fn expand_and_evaluate(
    tree: &mut SearchTree,
    leaf_id: u64,
    config: &Config,
    engine: &mut Engine,
    maia: &mut MaiaEngine,
    cache: Option<&Cache>,
) -> Result<f64, String> {
    let (epd, move_seq, node_type, terminal, already_expanded) = {
        let leaf = tree.get(leaf_id).ok_or("Node not found")?;
        (
            leaf.epd.clone(),
            leaf.move_sequence.clone(),
            leaf.node_type,
            leaf.terminal_value,
            leaf.expanded,
        )
    };

    // Check for terminal
    if let Some(tv) = terminal {
        return Ok(tv);
    }

    // Already expanded but no children (all candidates filtered out) — reuse cached value
    if already_expanded {
        let leaf = tree.get(leaf_id).ok_or("Node not found")?;
        if let Some(wdl) = leaf.wdl {
            return Ok(wdl.0 as f64 / 1000.0 + config.contempt * wdl.1 as f64 / 1000.0);
        }
    }

    // Check terminal via position
    let position = PositionState::from_moves(&move_seq)?;
    if let Some(tv) = position.terminal_value() {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.terminal_value = Some(tv);
        leaf.expanded = true;
        return Ok(tv);
    }

    // Get engine eval (check cache first)
    let engine_eval = if let Some(cached) = cache.and_then(|c| c.get_engine_eval(&epd)) {
        let (w, d, l, policy, q_values) = cached;
        EngineEval {
            wdl: (w, d, l),
            policy,
            q_values,
        }
    } else {
        let eval = engine.evaluate(&move_seq, config.engine_nodes)?;
        if let Some(c) = cache {
            let _ = c.put_engine_eval(&epd, eval.wdl, &eval.policy, &eval.q_values);
        }
        eval
    };

    // Get Maia prediction (check cache first)
    let maia_policy = if let Some(cached) = cache.and_then(|c| c.get_maia_policy(&move_seq)) {
        cached
    } else {
        let policy = maia.predict(&move_seq)?;
        if let Some(c) = cache {
            let _ = c.put_maia_policy(&move_seq, &policy);
        }
        policy
    };

    // Compute value from White's perspective
    let value = engine_eval.value_white(config.contempt);

    // Store eval data on the node
    {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.engine_policy = Some(engine_eval.policy.clone());
        leaf.engine_q_values = Some(engine_eval.q_values.clone());
        leaf.maia_policy = Some(maia_policy.clone());
        leaf.wdl = Some(engine_eval.wdl);
    }

    // Expand children based on node type
    let candidates = match node_type {
        NodeType::Max => candidate_moves_max(&engine_eval.policy, &engine_eval.q_values, &maia_policy, config),
        NodeType::Chance => candidate_moves_chance(&maia_policy, config),
    };

    let child_type = match node_type {
        NodeType::Max => NodeType::Chance,
        NodeType::Chance => NodeType::Max,
    };

    for (uci_move, prior) in &candidates {
        match position.apply_uci(uci_move) {
            Ok(new_pos) => {
                let child_terminal = new_pos.terminal_value();
                let child_id = tree.add_child(
                    leaf_id,
                    uci_move.clone(),
                    child_type,
                    new_pos.epd.clone(),
                    new_pos.move_sequence.clone(),
                    *prior,
                );
                if let Some(tv) = child_terminal {
                    tree.get_mut(child_id).unwrap().terminal_value = Some(tv);
                }
            }
            Err(e) => {
                log::warn!("Failed to apply move {uci_move}: {e}");
            }
        }
    }

    tree.get_mut(leaf_id).unwrap().expanded = true;

    Ok(value)
}

/// Build a tree snapshot for the UI, filtering by minimum visit count.
fn build_tree_snapshot(tree: &SearchTree, min_visits: u64) -> TreeSnapshot {
    let mut nodes = Vec::new();
    let mut stack: Vec<(u64, u32)> = vec![(tree.root_id, 0)];

    while let Some((id, depth)) = stack.pop() {
        if let Some(node) = tree.get(id) {
            if node.visit_count >= min_visits || id == tree.root_id {
                nodes.push(TreeNodeInfo {
                    id: node.id,
                    parent_id: node.parent,
                    move_uci: node.move_uci.clone(),
                    node_type: node.node_type,
                    visit_count: node.visit_count,
                    q_value: node.q_value(),
                    depth,
                });

                for &child_id in &node.children {
                    if depth < 10 {
                        // Max depth for UI
                        stack.push((child_id, depth + 1));
                    }
                }
            }
        }
    }

    TreeSnapshot { nodes }
}
