use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;

use crate::cache::Cache;
use crate::config::Config;
use crate::search::{NodeId, NodeType, RootMoveInfo, root_move_infos};

mod expand;
mod run;

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
    /// True if White is the side we're searching for (i.e., White is to move at the root).
    pub root_is_white: bool,
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

pub(super) enum CoordinatorError {
    Position { source: crate::position::PositionError, move_sequence: String },
    Engine { source: crate::engine::EngineError, move_sequence: String },
    Maia { source: crate::maia::MaiaError, move_sequence: String },
    NN { source: crate::nn::NNError, move_sequence: String },
    NodeNotFound { node_id: NodeId },
}

impl std::fmt::Display for CoordinatorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Position { source, move_sequence } =>
                write!(f, "Position error for '{move_sequence}': {source}"),
            Self::Engine { source, move_sequence } =>
                write!(f, "Engine error for '{move_sequence}': {source}"),
            Self::Maia { source, move_sequence } =>
                write!(f, "Maia error for '{move_sequence}': {source}"),
            Self::NN { source, move_sequence } =>
                write!(f, "NN error for '{move_sequence}': {source}"),
            Self::NodeNotFound { node_id } =>
                write!(f, "Node {:?} not found in tree", node_id),
        }
    }
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

        // Show any persisted results immediately, before engines initialize.
        self.latest_snapshot = None;
        self.load_persisted(&move_sequence, &config);

        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = Arc::clone(&cancel);
        let (sender, receiver) = mpsc::channel();

        self.cancel = Some(cancel);
        self.receiver = Some(receiver);
        self.running = true;

        let handle = thread::spawn(move || {
            run::run_mcts(move_sequence, config, cancel_clone, sender);
        });
        self.join_handle = Some(handle);
    }

    /// Load a persisted tree from the DB and populate `latest_snapshot`
    /// so the UI can display previous results without starting a search.
    pub fn load_persisted(&mut self, move_sequence: &str, config: &Config) {
        let cache = match Cache::open() {
            Ok(cache) => cache,
            Err(e) => {
                log::error!("Failed to open cache for load_persisted: {e}");
                return;
            }
        };
        let mut tree = match cache.load_tree(move_sequence) {
            Some(loaded_tree) if loaded_tree.node_count() > 1 => {
                log::info!("load_persisted: found tree with {} nodes for session '{move_sequence}'", loaded_tree.node_count());
                loaded_tree
            }
            _ => {
                log::info!("load_persisted: no persisted tree for session '{move_sequence}'");
                return;
            }
        };

        let root_id = tree.root_id;
        let root_epd = tree.root().epd.clone();
        let root_move_seq = tree.root().move_sequence.clone();
        match cache.get_engine_eval(&root_epd) {
            Some((w, d, l, policy, q_values)) => match tree.get_mut(root_id) {
                Some(root) => {
                    root.wdl = Some((w, d, l));
                    root.engine_policy = Some(policy);
                    root.engine_q_values = Some(q_values);
                }
                None => log::error!("load_persisted: root node {:?} missing from loaded tree", root_id),
            },
            None => {}
        }
        match cache.get_maia_policy(&root_move_seq) {
            Some(maia_policy) => match tree.get_mut(root_id) {
                Some(root) => { root.maia_policy = Some(maia_policy); }
                None => log::error!("load_persisted: root node {:?} missing from loaded tree", root_id),
            },
            None => {}
        }

        let moves = root_move_infos(&tree, config);
        let best = moves.first().map(|move_info| move_info.uci_move.clone());
        let tree_snap = run::build_tree_snapshot(&tree, 10);

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
        match Cache::open() {
            Ok(cache) => { let _ = cache.clear_tree(move_sequence); }
            Err(_) => {}
        }
    }

    /// Pause/stop the search. Waits for the background thread to finish
    /// its final save before returning.
    pub fn stop(&mut self) {
        match &self.cancel {
            Some(cancel) => cancel.store(true, Ordering::Relaxed),
            None => {}
        }
        match self.join_handle.take() {
            Some(handle) => { let _ = handle.join(); }
            None => {}
        }
        self.running = false;
        self.cancel = None;
        self.receiver = None;
    }

    /// Poll for new snapshots. Returns true if a new snapshot was received.
    pub fn poll(&mut self) -> bool {
        let mut updated = false;
        match &self.receiver {
            Some(receiver) => loop {
                match receiver.try_recv() {
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
            },
            None => {}
        }
        updated
    }
}

impl Drop for Coordinator {
    fn drop(&mut self) {
        self.stop();
    }
}
