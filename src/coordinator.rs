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
    NodeId, NodeType, RootMoveInfo, SearchState, SearchTree, backpropagate,
    candidate_moves_chance, candidate_moves_max, root_move_infos, select,
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
    pub id: NodeId,
    pub parent_id: Option<NodeId>,
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
    join_handle: Option<thread::JoinHandle<()>>,
    pub latest_snapshot: Option<SearchSnapshot>,
    pub running: bool,
}

impl Coordinator {
    pub fn new() -> Self {
        Self {
            cancel: None,
            receiver: None,
            join_handle: None,
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

        let handle = thread::spawn(move || {
            run_mcts(move_sequence, config, cancel_clone, tx);
        });
        self.join_handle = Some(handle);
    }

    /// Load a persisted tree from the DB and populate `latest_snapshot`
    /// so the UI can display previous results without starting a search.
    pub fn load_persisted(&mut self, move_sequence: &str, config: &Config) {
        let cache = match Cache::open() {
            Ok(c) => c,
            Err(e) => {
                log::error!("Failed to open cache for load_persisted: {e}");
                return;
            }
        };
        let mut tree = match cache.load_tree(move_sequence) {
            Some(t) if t.node_count() > 1 => {
                log::info!("load_persisted: found tree with {} nodes for session '{move_sequence}'", t.node_count());
                t
            }
            _ => {
                log::info!("load_persisted: no persisted tree for session '{move_sequence}'");
                return;
            }
        };

        // Re-populate root eval data from caches
        let root_id = tree.root_id;
        let root_epd = tree.root().epd.clone();
        let root_move_seq = tree.root().move_sequence.clone();
        if let Some((w, d, l, policy, q_values)) = cache.get_engine_eval(&root_epd) {
            let root = tree.get_mut(root_id).unwrap();
            root.wdl = Some((w, d, l));
            root.engine_policy = Some(policy);
            root.engine_q_values = Some(q_values);
        }
        if let Some(maia_pol) = cache.get_maia_policy(&root_move_seq) {
            let root = tree.get_mut(root_id).unwrap();
            root.maia_policy = Some(maia_pol);
        }

        let moves = root_move_infos(&tree, config);
        let best = match moves.first() {
            Some(m) => Some(m.uci_move.clone()),
            None => None,
        };
        let tree_snap = build_tree_snapshot(&tree, 10);

        self.latest_snapshot = Some(SearchSnapshot {
            iteration: 0,
            elapsed_secs: 0.0,
            root_moves: moves,
            best_move: best,
            node_count: tree.node_count(),
            iterations_per_sec: 0.0,
            best_move_history: Vec::new(),
            q_history: Vec::new(),
            tree_snapshot: Some(tree_snap),
        });

        log::info!("Loaded persisted tree with {} nodes", tree.node_count());
    }

    /// Clear persisted tree data for the given move sequence.
    pub fn clear_session(&self, move_sequence: &str) {
        if let Ok(cache) = Cache::open() {
            let _ = cache.clear_tree(move_sequence);
        }
    }

    /// Pause/stop the search. Waits for the background thread to finish
    /// its final save before returning.
    pub fn stop(&mut self) {
        if let Some(cancel) = &self.cancel {
            cancel.store(true, Ordering::Relaxed);
        }
        // Wait for the background thread to complete its final DB save
        if let Some(handle) = self.join_handle.take() {
            let _ = handle.join();
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

    // Session ID = the root move sequence (identifies which position we're searching)
    let session_id = move_sequence.clone();

    // Try to load existing tree from cache, otherwise create fresh
    let cached_tree = match cache.as_ref() {
        Some(c) => c.load_tree(&session_id),
        None => None,
    };
    let mut tree = if let Some(loaded) = cached_tree {
        log::info!("Resumed tree with {} nodes", loaded.node_count());
        // Re-populate root eval data from engine/maia caches
        if let Some(c) = &cache {
            let root = loaded.root();
            let root_id = loaded.root_id;
            let root_epd = root.epd.clone();
            let root_move_seq = root.move_sequence.clone();
            let _ = root;
            // Need mutable access — reconstruct after loading evals
            let mut tree = loaded;
            if let Some((w, d, l, policy, q_values)) = c.get_engine_eval(&root_epd) {
                let root = tree.get_mut(root_id).unwrap();
                root.wdl = Some((w, d, l));
                root.engine_policy = Some(policy);
                root.engine_q_values = Some(q_values);
            }
            if let Some(maia_pol) = c.get_maia_policy(&root_move_seq) {
                let root = tree.get_mut(root_id).unwrap();
                root.maia_policy = Some(maia_pol);
            }
            tree
        } else {
            loaded
        }
    } else {
        // Root is always MAX — we're deciding our move regardless of which color we play
        SearchTree::new(position.epd.clone(), position.move_sequence.clone(), NodeType::Max)
    };

    log::info!(
        "Search started position={} max_nodes={}",
        if move_sequence.is_empty() { "startpos" } else { &move_sequence },
        config.max_nodes
    );

    let start_time = Instant::now();
    let mut best_move_history: Vec<(u64, String)> = Vec::new();
    let mut q_history: Vec<(u64, f64)> = Vec::new();
    let mut cache_hits: u64 = 0;
    let mut cache_misses: u64 = 0;
    let update_interval = 50; // Send snapshot every N iterations

    // Pre-allocated search state — reused across all iterations
    let mut search_state = SearchState::new();

    // Send an initial snapshot immediately if we resumed a non-trivial tree
    if tree.node_count() > 1 {
        let moves = root_move_infos(&tree, &config);
        let best = match moves.first() {
            Some(m) => Some(m.uci_move.clone()),
            None => None,
        };
        if let Some(ref bm) = best {
            best_move_history.push((0, bm.clone()));
        }
        if let Some(bm_info) = moves.first() {
            q_history.push((0, bm_info.practical_q));
        }
        let tree_snap = build_tree_snapshot(&tree, 10);
        let snapshot = SearchSnapshot {
            iteration: 0,
            elapsed_secs: 0.0,
            root_moves: moves,
            best_move: best,
            node_count: tree.node_count(),
            iterations_per_sec: 0.0,
            best_move_history: best_move_history.clone(),
            q_history: q_history.clone(),
            tree_snapshot: Some(tree_snap),
        };
        let _ = tx.send(snapshot);
    }

    let mut iteration: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if tree.node_count() as u64 >= config.max_nodes {
            break;
        }

        // 1. SELECT — traverse tree to a leaf
        let leaf_id = select(&tree, &config, &mut search_state);

        // 2. EXPAND & EVALUATE the leaf
        let value = match expand_and_evaluate(
            &mut tree,
            leaf_id,
            &config,
            &mut engine,
            &mut maia,
            cache.as_ref(),
            &mut cache_hits,
            &mut cache_misses,
        ) {
            Ok(v) => v,
            Err(e) => {
                log::error!("Expand/evaluate error at iteration {iteration}: {e}");
                break;
            }
        };

        // 3. BACKPROPAGATE
        backpropagate(&mut tree, leaf_id, value);
        iteration += 1;

        // Log search milestones every 100 iterations
        if iteration % 100 == 0 {
            let moves = root_move_infos(&tree, &config);
            let best = match moves.first() {
                Some(m) => m.uci_move.as_str(),
                None => "?",
            };
            let best_q = match moves.first() {
                Some(m) => m.practical_q,
                None => 0.0,
            };
            log::info!(
                "Search milestone iteration={iteration} best={best} practical_q={best_q:.4} nodes={}",
                tree.node_count()
            );
        }

        // 4. Periodic flush to SQLite
        if iteration % config.flush_interval as u64 == 0 {
            if let Some(c) = &cache {
                match c.save_tree(&tree, &session_id) {
                    Ok(()) => log::debug!("Flushed tree ({} nodes) at iteration {iteration}", tree.node_count()),
                    Err(e) => log::error!("Failed to flush tree: {e}"),
                }
            }
        }

        // 5. Send periodic updates to UI
        let at_node_limit = tree.node_count() as u64 >= config.max_nodes;
        if iteration % update_interval as u64 == 0 || at_node_limit {
            let elapsed = start_time.elapsed().as_secs_f64();
            let moves = root_move_infos(&tree, &config);
            let best = match moves.first() {
                Some(m) => Some(m.uci_move.clone()),
                None => None,
            };

            if let Some(ref bm) = best {
                let last_move = match best_move_history.last() {
                    Some((_, m)) => Some(m),
                    None => None,
                };
                if last_move != Some(bm) {
                    best_move_history.push((iteration, bm.clone()));
                }
            }
            if let Some(bm_info) = moves.first() {
                q_history.push((iteration, bm_info.practical_q));
            }

            let tree_snap = build_tree_snapshot(&tree, 10); // min 10 visits for tree view

            let snapshot = SearchSnapshot {
                iteration,
                elapsed_secs: elapsed,
                root_moves: moves,
                best_move: best,
                node_count: tree.node_count(),
                iterations_per_sec: iteration as f64 / elapsed.max(0.001),
                best_move_history: best_move_history.clone(),
                q_history: q_history.clone(),
                tree_snapshot: Some(tree_snap),
            };

            if tx.send(snapshot).is_err() {
                break; // Receiver dropped
            }
        }
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    let moves = root_move_infos(&tree, &config);
    let best = match moves.first() {
        Some(m) => m.uci_move.as_str(),
        None => "none",
    };
    let best_q = match moves.first() {
        Some(m) => m.practical_q,
        None => 0.0,
    };
    log::info!(
        "Search complete iterations={iteration} best={best} practical_q={best_q:.4} nodes={} elapsed={elapsed:.1}s cache_hits={cache_hits} cache_misses={cache_misses}",
        tree.node_count()
    );

    // Final save — persist tree state for resumption
    if let Some(c) = &cache {
        match c.save_tree(&tree, &session_id) {
            Ok(()) => log::info!("Final save: {} nodes for session '{session_id}'", tree.node_count()),
            Err(e) => log::error!("Final save failed: {e}"),
        }
    }
}

/// Expand a leaf node and return its evaluation (from White's perspective).
fn expand_and_evaluate(
    tree: &mut SearchTree,
    leaf_id: NodeId,
    config: &Config,
    engine: &mut Engine,
    maia: &mut MaiaEngine,
    cache: Option<&Cache>,
    cache_hits: &mut u64,
    cache_misses: &mut u64,
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

    // Determine side to move from move sequence (even count = White)
    let white_to_move = if move_seq.is_empty() {
        true
    } else {
        move_seq.split_whitespace().count() % 2 == 0
    };

    // Check for terminal
    if let Some(tv) = terminal {
        return Ok(tv);
    }

    // Already expanded but no children (all candidates filtered out) — reuse cached value
    if already_expanded {
        let leaf = tree.get(leaf_id).ok_or("Node not found")?;
        if let Some(wdl) = leaf.wdl {
            let eval = EngineEval { wdl, policy: Default::default(), q_values: Default::default() };
            return Ok(eval.value_white(config.contempt, white_to_move));
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
    let cached_engine = match cache {
        Some(c) => c.get_engine_eval(&epd),
        None => None,
    };
    let engine_eval = if let Some(cached) = cached_engine {
        let (w, d, l, policy, q_values) = cached;
        *cache_hits += 1;
        EngineEval {
            wdl: (w, d, l),
            policy,
            q_values,
        }
    } else {
        *cache_misses += 1;
        let eval = engine.evaluate(&move_seq, config.engine_nodes)?;
        if let Some(c) = cache {
            let _ = c.put_engine_eval(&epd, eval.wdl, &eval.policy, &eval.q_values);
        }
        eval
    };

    // Get Maia prediction (check cache first)
    let cached_maia = match cache {
        Some(c) => c.get_maia_policy(&move_seq),
        None => None,
    };
    let maia_policy = if let Some(cached) = cached_maia {
        *cache_hits += 1;
        cached
    } else {
        *cache_misses += 1;
        let policy = maia.predict(&move_seq)?;
        if let Some(c) = cache {
            let _ = c.put_maia_policy(&move_seq, &policy);
        }
        policy
    };

    // Compute value from White's perspective (flip WDL when Black to move)
    let value = engine_eval.value_white(config.contempt, white_to_move);

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
        NodeType::Max => candidate_moves_max(&engine_eval.policy, &maia_policy, config),
        NodeType::Chance => candidate_moves_chance(&maia_policy, config),
    };

    let child_type = match node_type {
        NodeType::Max => NodeType::Chance,
        NodeType::Chance => NodeType::Max,
    };

    candidates.iter().for_each(|(uci_move, prior)| {
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
    });

    tree.get_mut(leaf_id).unwrap().expanded = true;

    Ok(value)
}

/// Build a tree snapshot for the UI, filtering by minimum visit count.
fn build_tree_snapshot(tree: &SearchTree, min_visits: u64) -> TreeSnapshot {
    let mut nodes = Vec::new();
    let mut stack: Vec<(NodeId, u32)> = vec![(tree.root_id, 0)];

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

                if depth < 10 {
                    // Max depth for UI
                    node.children.iter().for_each(|&child_id| {
                        stack.push((child_id, depth + 1));
                    });
                }
            }
        }
    }

    TreeSnapshot { nodes }
}
