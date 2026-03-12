use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::Instant;

use crate::cache::Cache;
use crate::config::Config;
use crate::engine::Engine;
use crate::maia::MaiaEngine;
use crate::nn::NNEvaluator;
use crate::position::PositionState;
use crate::search::{
    NodeId, NodeType, SearchState, SearchTree,
    apply_virtual_loss, backpropagate, revert_virtual_loss, root_move_infos,
    select, select_with_path,
};

use super::{CoordinatorError, SearchSnapshot, TreeNodeInfo, TreeSnapshot};
use super::expand::{EvalBackend, LeafPrep, apply_eval_and_expand, expand_and_evaluate, prepare_leaf};

/// A leaf selected for batch evaluation, with its path for virtual loss management.
struct PendingLeaf {
    leaf_id: NodeId,
    depth: u32,
    path: Vec<NodeId>,
    prep: LeafPrep,
}

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

    let mut backend = match init_backend(&config) {
        Ok(b) => b,
        Err(e) => { log::error!("{e}"); return; }
    };

    let cache = Cache::open().ok();
    let session_id = move_sequence.clone();

    let mut tree = load_or_create_tree(cache.as_ref(), &session_id, &position, &config);

    let use_batching = matches!(&backend, EvalBackend::Onnx { .. }) && config.batch_size > 1;

    log::info!(
        "Search started position={} max_nodes={} batch_size={}",
        if move_sequence.is_empty() { "startpos" } else { &move_sequence },
        config.max_nodes,
        if use_batching { config.batch_size } else { 1 },
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

    if use_batching {
        // ── Batched MCTS loop (ONNX backend) ─────────────────────────────
        loop {
            if cancel.load(Ordering::Relaxed) { break; }
            if tree.node_count() as u64 >= config.max_nodes { break; }

            let batch_size = config.batch_size
                .min((config.max_nodes - tree.node_count() as u64) as usize);

            // Phase 1: Select leaves with virtual loss for diversity.
            // Track which leaves are already selected so we don't expand the same node twice.
            let mut pending: Vec<PendingLeaf> = Vec::with_capacity(batch_size);
            let mut selected_leaves: std::collections::HashSet<NodeId> = std::collections::HashSet::new();

            for _ in 0..batch_size {
                let (leaf_id, depth, path) = select_with_path(&tree, &config, &mut search_state);
                apply_virtual_loss(&mut tree, &path);

                // If this leaf was already selected in this batch, mark as duplicate
                // so we backprop without expanding twice.
                if !selected_leaves.insert(leaf_id) {
                    pending.push(PendingLeaf { leaf_id, depth, path, prep: LeafPrep::Duplicate });
                    continue;
                }

                let prep = match prepare_leaf(&mut tree, leaf_id, &config, cache.as_ref(), &mut cache_hits) {
                    Ok(p) => p,
                    Err(e) => {
                        revert_virtual_loss(&mut tree, &path);
                        log::error!("Prepare error at iteration {iteration}: {e}");
                        continue;
                    }
                };

                pending.push(PendingLeaf { leaf_id, depth, path, prep });
            }

            // Phase 2: Collect positions that need NN evaluation.
            let mut eval_indices: Vec<usize> = Vec::new();
            let mut eval_move_seqs: Vec<String> = Vec::new();

            for (i, p) in pending.iter().enumerate() {
                if let LeafPrep::NeedsEval { move_seq, .. } = &p.prep {
                    eval_indices.push(i);
                    eval_move_seqs.push(move_seq.clone());
                }
            }

            // Phase 3: Batch NN inference for all cache misses.
            let batch_results = if !eval_move_seqs.is_empty() {
                cache_misses += eval_move_seqs.len() as u64;
                let move_seq_refs: Vec<&str> = eval_move_seqs.iter().map(|s| s.as_str()).collect();
                match &mut backend {
                    EvalBackend::Onnx { evaluator } => {
                        match evaluator.evaluate_batch(&move_seq_refs) {
                            Ok(results) => results,
                            Err(e) => {
                                // Revert all virtual losses and abort.
                                for p in &pending {
                                    revert_virtual_loss(&mut tree, &p.path);
                                }
                                log::error!("Batch eval error at iteration {iteration}: {e}");
                                break;
                            }
                        }
                    }
                    _ => unreachable!("batching only used with ONNX backend"),
                }
            } else {
                Vec::new()
            };

            // Phase 4: Revert virtual losses, expand, and backpropagate.
            let mut eval_result_idx = 0;
            for p in &pending {
                revert_virtual_loss(&mut tree, &p.path);

                let value = match &p.prep {
                    LeafPrep::Ready { value } => *value,
                    LeafPrep::Duplicate => {
                        // Re-selected same leaf — use its current Q as backprop value.
                        tree.get(p.leaf_id).map(|n| n.q_value()).unwrap_or(0.5)
                    }
                    LeafPrep::Cached { engine_eval, maia_policy, position } => {
                        apply_eval_and_expand(
                            &mut tree, p.leaf_id, p.depth, &config,
                            engine_eval.clone(), maia_policy.clone(), position, cache.as_ref(),
                        )
                    }
                    LeafPrep::NeedsEval { position, .. } => {
                        let (engine_eval, maia_policy) = batch_results[eval_result_idx].clone();
                        eval_result_idx += 1;
                        apply_eval_and_expand(
                            &mut tree, p.leaf_id, p.depth, &config,
                            engine_eval, maia_policy, position, cache.as_ref(),
                        )
                    }
                };

                backpropagate(&mut tree, p.leaf_id, value);
                iteration += 1;
            }

            if iteration % 100 < batch_size as u64 {
                log_milestone(iteration, &tree, &config);
            }

            if iteration % (config.flush_interval as u64) < (batch_size as u64) {
                flush_tree(cache.as_ref(), &tree, &session_id, iteration);
            }

            let at_node_limit = tree.node_count() as u64 >= config.max_nodes;
            if iteration % update_interval < batch_size as u64 || at_node_limit {
                let elapsed = start_time.elapsed().as_secs_f64();
                send_snapshot(&tree, &config, &sender, iteration, elapsed, &mut best_move_history, &mut q_history);
            }
        }
    } else {
        // ── Sequential MCTS loop (lc0 backend or batch_size=1) ───────────
        loop {
            if cancel.load(Ordering::Relaxed) { break; }
            if tree.node_count() as u64 >= config.max_nodes { break; }

            let (leaf_id, depth) = select(&tree, &config, &mut search_state);

            let value = match expand_and_evaluate(
                &mut tree, leaf_id, depth, &config, &mut backend,
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

fn init_backend(config: &Config) -> Result<EvalBackend, CoordinatorError> {
    if config.onnx_configured() {
        log::info!("Using direct ONNX inference backend");
        let evaluator = NNEvaluator::new(&config.engine_onnx_path, &config.maia_onnx_path)
            .map_err(|e| CoordinatorError::NN { source: e, move_sequence: String::new() })?;
        Ok(EvalBackend::Onnx { evaluator })
    } else {
        log::info!("Using lc0 UCI process backend");
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

        Ok(EvalBackend::Lc0 { engine, maia })
    }
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

                    if depth < 20 {
                        node.children.iter().for_each(|&child_id| {
                            stack.push((child_id, depth + 1));
                        });
                    }
                }
            }
            None => {}
        }
    }

    let root_is_white = tree
        .root()
        .epd
        .split_whitespace()
        .nth(1)
        .map(|color| color == "w")
        .unwrap_or(true);

    TreeSnapshot { nodes, root_is_white }
}
