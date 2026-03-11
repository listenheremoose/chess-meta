use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::cache::Cache;
use crate::config::Config;
use crate::engine::Engine;
use crate::maia::MaiaEngine;
use crate::position::PositionState;
use crate::search::{
    NodeId, NodeType, SearchState, SearchTree,
    backpropagate, root_move_infos, select,
};

use super::{CoordinatorError, SearchSnapshot, TreeNodeInfo, TreeSnapshot};
use super::expand::expand_and_evaluate;

/// Main MCTS loop running in a background thread.
pub(super) fn run_mcts(
    move_sequence: String,
    config: Config,
    cancel: Arc<AtomicBool>,
    sender: mpsc::Sender<SearchSnapshot>,
) {
    let position = match PositionState::from_moves(&move_sequence) {
        Ok(position) => position,
        Err(e) => { log::error!("Invalid position: {e}"); return; }
    };

    let (mut engine, mut maia) = match init_engines(&config) {
        Ok(pair) => pair,
        Err(e) => { log::error!("{e}"); return; }
    };

    let cache = Cache::open().ok();
    let session_id = move_sequence.clone();

    let mut tree = load_or_create_tree(cache.as_ref(), &session_id, &position, &config);

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
    let update_interval = 50u64;
    let mut search_state = SearchState::new();

    if tree.node_count() > 1 {
        send_initial_snapshot(&tree, &config, &sender, &mut best_move_history, &mut q_history);
    }

    let mut iteration: u64 = 0;
    loop {
        if cancel.load(Ordering::Relaxed) { break; }
        if tree.node_count() as u64 >= config.max_nodes { break; }

        let leaf_id = select(&tree, &config, &mut search_state);

        let value = match expand_and_evaluate(
            &mut tree, leaf_id, &config, &mut engine, &mut maia,
            cache.as_ref(), &mut cache_hits, &mut cache_misses,
        ) {
            Ok(value) => value,
            Err(e) => { log::error!("Expand/evaluate error at iteration {iteration}: {e}"); break; }
        };

        backpropagate(&mut tree, leaf_id, value);
        iteration += 1;

        if iteration % 100 == 0 {
            log_milestone(iteration, &tree, &config);
        }

        if iteration % config.flush_interval as u64 == 0 {
            flush_tree(cache.as_ref(), &tree, &session_id, iteration);
        }

        let at_node_limit = tree.node_count() as u64 >= config.max_nodes;
        if iteration % update_interval == 0 || at_node_limit {
            let elapsed = start_time.elapsed().as_secs_f64();
            send_snapshot(&tree, &config, &sender, iteration, elapsed, &mut best_move_history, &mut q_history);
        }
    }

    let elapsed = start_time.elapsed().as_secs_f64();
    let moves = root_move_infos(&tree, &config);
    let best = match moves.first() {
        Some(move_info) => move_info.uci_move.as_str(),
        None => "none",
    };
    let best_q = match moves.first() {
        Some(move_info) => move_info.practical_q,
        None => 0.0,
    };
    log::info!(
        "Search complete iterations={iteration} best={best} practical_q={best_q:.4} nodes={} elapsed={elapsed:.1}s cache_hits={cache_hits} cache_misses={cache_misses}",
        tree.node_count()
    );

    match &cache {
        Some(c) => match c.save_tree(&tree, &session_id) {
            Ok(()) => log::info!("Final save: {} nodes for session '{session_id}'", tree.node_count()),
            Err(e) => log::error!("Final save failed: {e}"),
        },
        None => {}
    }
}

fn init_engines(config: &Config) -> Result<(Engine, MaiaEngine), CoordinatorError> {
    let engine = Engine::new(
        &config.lc0_path,
        &config.engine_weights_path,
        config.nn_cache_size_mb,
        config.ucinewgame_interval,
    ).map_err(|e| CoordinatorError::Engine { source: e, move_sequence: String::new() })?;

    let maia = MaiaEngine::new(
        &config.lc0_path,
        &config.maia_weights_path,
        config.ucinewgame_interval,
    ).map_err(|e| CoordinatorError::Maia { source: e, move_sequence: String::new() })?;

    Ok((engine, maia))
}

fn load_or_create_tree(
    cache: Option<&Cache>,
    session_id: &str,
    position: &PositionState,
    config: &Config,
) -> SearchTree {
    let cached_tree = match cache {
        Some(cache) => cache.load_tree(session_id),
        None => None,
    };
    match cached_tree {
        Some(loaded) => {
            log::info!("Resumed tree with {} nodes", loaded.node_count());
            repopulate_root_evals(loaded, cache, config)
        }
        None => {
            SearchTree::new(position.epd.clone(), position.move_sequence.clone(), NodeType::Max)
        }
    }
}

fn repopulate_root_evals(mut tree: SearchTree, cache: Option<&Cache>, _config: &Config) -> SearchTree {
    match cache {
        None => tree,
        Some(cache) => {
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
                    None => log::error!("run_mcts: root node {:?} missing from loaded tree", root_id),
                },
                None => {}
            }
            match cache.get_maia_policy(&root_move_seq) {
                Some(maia_policy) => match tree.get_mut(root_id) {
                    Some(root) => { root.maia_policy = Some(maia_policy); }
                    None => log::error!("run_mcts: root node {:?} missing from loaded tree", root_id),
                },
                None => {}
            }
            tree
        }
    }
}

fn log_milestone(iteration: u64, tree: &SearchTree, config: &Config) {
    let moves = root_move_infos(tree, config);
    let best = match moves.first() {
        Some(move_info) => move_info.uci_move.as_str(),
        None => "?",
    };
    let best_q = match moves.first() {
        Some(move_info) => move_info.practical_q,
        None => 0.0,
    };
    log::info!(
        "Search milestone iteration={iteration} best={best} practical_q={best_q:.4} nodes={}",
        tree.node_count()
    );
}

fn flush_tree(cache: Option<&Cache>, tree: &SearchTree, session_id: &str, iteration: u64) {
    match cache {
        Some(c) => match c.save_tree(tree, session_id) {
            Ok(()) => log::debug!("Flushed tree ({} nodes) at iteration {iteration}", tree.node_count()),
            Err(e) => log::error!("Failed to flush tree: {e}"),
        },
        None => {}
    }
}

fn send_initial_snapshot(
    tree: &SearchTree,
    config: &Config,
    sender: &mpsc::Sender<SearchSnapshot>,
    best_move_history: &mut Vec<(u64, String)>,
    q_history: &mut Vec<(u64, f64)>,
) {
    let moves = root_move_infos(tree, config);
    let best = moves.first().map(|move_info| move_info.uci_move.clone());
    match &best {
        Some(best_move) => best_move_history.push((0, best_move.clone())),
        None => {}
    }
    match moves.first() {
        Some(best_move_info) => q_history.push((0, best_move_info.practical_q)),
        None => {}
    }
    let tree_snap = build_tree_snapshot(tree, 10);
    let _ = sender.send(SearchSnapshot {
        iteration: 0,
        elapsed_secs: 0.0,
        root_moves: moves,
        best_move: best,
        node_count: tree.node_count(),
        iterations_per_sec: 0.0,
        best_move_history: best_move_history.clone(),
        q_history: q_history.clone(),
        tree_snapshot: Some(tree_snap),
    });
}

fn send_snapshot(
    tree: &SearchTree,
    config: &Config,
    sender: &mpsc::Sender<SearchSnapshot>,
    iteration: u64,
    elapsed: f64,
    best_move_history: &mut Vec<(u64, String)>,
    q_history: &mut Vec<(u64, f64)>,
) {
    let moves = root_move_infos(tree, config);
    let best = moves.first().map(|move_info| move_info.uci_move.clone());

    match &best {
        Some(best_move) => {
            let last_recorded = best_move_history.last().map(|(_, move_uci)| move_uci);
            if last_recorded != Some(best_move) {
                best_move_history.push((iteration, best_move.clone()));
            }
        }
        None => {}
    }
    match moves.first() {
        Some(best_move_info) => q_history.push((iteration, best_move_info.practical_q)),
        None => {}
    }

    let tree_snap = build_tree_snapshot(tree, 10);
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

    match sender.send(snapshot) {
        Err(_) => log::debug!("UI receiver dropped"),
        Ok(_) => {}
    }
}

/// Build a tree snapshot for the UI, filtering by minimum visit count.
pub(super) fn build_tree_snapshot(tree: &SearchTree, min_visits: u64) -> TreeSnapshot {
    let mut nodes = Vec::new();
    let mut stack: Vec<(NodeId, u32)> = vec![(tree.root_id, 0)];

    while let Some((id, depth)) = stack.pop() {
        match tree.get(id) {
            Some(node) => {
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
                        node.children.iter().for_each(|&child_id| {
                            stack.push((child_id, depth + 1));
                        });
                    }
                }
            }
            None => {}
        }
    }

    TreeSnapshot { nodes }
}
