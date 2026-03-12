use std::collections::HashMap;

use crate::cache::Cache;
use crate::config::Config;
use crate::engine::{Engine, EngineEval};
use crate::maia::MaiaEngine;
use crate::nn::NNEvaluator;
use crate::position::PositionState;
use crate::search::{NodeId, NodeType, SearchTree, candidate_moves_max, candidate_moves_chance};

use super::CoordinatorError;

/// Backend for position evaluation — either lc0 UCI processes or direct ONNX inference.
pub(super) enum EvalBackend {
    Lc0 {
        engine: Engine,
        maia: MaiaEngine,
    },
    Onnx {
        evaluator: NNEvaluator,
    },
}

/// Result of preparing a leaf for evaluation.
pub(super) enum LeafPrep {
    /// Leaf already has a value (terminal or re-selected expanded node). Just backprop.
    Ready { value: f64 },
    /// Same leaf selected multiple times in one batch. Just backprop its current Q.
    Duplicate,
    /// Leaf was resolved from cache. Needs child expansion but no NN eval.
    Cached {
        engine_eval: EngineEval,
        maia_policy: HashMap<String, f32>,
        position: PositionState,
    },
    /// Leaf needs NN inference. Collect into batch.
    NeedsEval {
        move_seq: String,
        position: PositionState,
    },
}

/// Prepare a leaf for evaluation: check terminal, already-expanded, and cache.
/// Returns a `LeafPrep` indicating what the coordinator should do next.
pub(super) fn prepare_leaf(
    tree: &mut SearchTree,
    leaf_id: NodeId,
    config: &Config,
    cache: Option<&Cache>,
    cache_hits: &mut u64,
) -> Result<LeafPrep, CoordinatorError> {
    let (epd, move_seq, terminal, already_expanded, wdl) = {
        let leaf = tree.get(leaf_id).ok_or(CoordinatorError::NodeNotFound { node_id: leaf_id })?;
        (
            leaf.epd.clone(),
            leaf.move_sequence.clone(),
            leaf.terminal_value,
            leaf.expanded,
            leaf.wdl,
        )
    };

    let white_to_move = move_seq.is_empty() || move_seq.split_whitespace().count() % 2 == 0;

    // Terminal node — already has a value.
    if let Some(tv) = terminal {
        return Ok(LeafPrep::Ready { value: tv });
    }

    // Re-selected an already-expanded node — use existing WDL.
    if already_expanded {
        if let Some(wdl) = wdl {
            let eval = EngineEval { wdl, policy: Default::default(), q_values: Default::default() };
            return Ok(LeafPrep::Ready { value: eval.value_white(config.contempt, white_to_move) });
        }
    }

    // Build position and check for game-over.
    let position = PositionState::from_moves(&move_seq)
        .map_err(|e| CoordinatorError::Position { source: e, move_sequence: move_seq.clone() })?;
    if let Some(tv) = position.terminal_value() {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.terminal_value = Some(tv);
        leaf.expanded = true;
        return Ok(LeafPrep::Ready { value: tv });
    }

    // Check cache.
    if let Some(c) = cache {
        let cached_engine = c.get_engine_eval(&epd).map(|(w, d, l, policy, q_values)| {
            *cache_hits += 1;
            EngineEval { wdl: (w, d, l), policy, q_values }
        });
        let cached_maia = c.get_maia_policy(&move_seq).map(|p| {
            *cache_hits += 1;
            p
        });
        if let (Some(engine_eval), Some(maia_policy)) = (cached_engine, cached_maia) {
            let value = engine_eval.value_white(config.contempt, white_to_move);
            return Ok(LeafPrep::Cached { engine_eval, maia_policy, position });
        }
    }

    Ok(LeafPrep::NeedsEval { move_seq, position })
}

/// Apply evaluation results to a leaf node and expand its children.
pub(super) fn apply_eval_and_expand(
    tree: &mut SearchTree,
    leaf_id: NodeId,
    depth: u32,
    config: &Config,
    engine_eval: EngineEval,
    maia_policy: HashMap<String, f32>,
    position: &PositionState,
    cache: Option<&Cache>,
) -> f64 {
    let node_type = tree.get(leaf_id).unwrap().node_type;
    let move_seq = &tree.get(leaf_id).unwrap().move_sequence;
    let white_to_move = move_seq.is_empty() || move_seq.split_whitespace().count() % 2 == 0;
    let value = engine_eval.value_white(config.contempt, white_to_move);

    // Store eval on the node.
    {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.engine_policy = Some(engine_eval.policy.clone());
        leaf.engine_q_values = Some(engine_eval.q_values.clone());
        leaf.maia_policy = Some(maia_policy.clone());
        leaf.wdl = Some(engine_eval.wdl);
    }

    // Cache the results.
    if let Some(c) = cache {
        let epd = &tree.get(leaf_id).unwrap().epd;
        let _ = c.put_engine_eval(epd, engine_eval.wdl, &engine_eval.policy, &engine_eval.q_values);
        let move_seq = &tree.get(leaf_id).unwrap().move_sequence;
        let _ = c.put_maia_policy(move_seq, &maia_policy);
    }

    // Generate and add children.
    let mut candidates = match node_type {
        NodeType::Max => candidate_moves_max(&engine_eval.policy, &maia_policy, config),
        NodeType::Chance => candidate_moves_chance(&maia_policy, config),
    };

    let effective_width = (config.max_width as f64 * config.width_decay.powi(depth as i32))
        .floor() as usize;
    let effective_width = effective_width.max(2);
    if candidates.len() > effective_width {
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(effective_width);
        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        if sum > 0.0 {
            candidates.iter_mut().for_each(|(_, p)| *p /= sum);
        }
    }

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
    value
}

/// Expand a leaf node and return its evaluation (from White's perspective).
/// Used by the sequential (non-batched) code path (lc0 backend).
pub(super) fn expand_and_evaluate(
    tree: &mut SearchTree,
    leaf_id: NodeId,
    depth: u32,
    config: &Config,
    backend: &mut EvalBackend,
    cache: Option<&Cache>,
    cache_hits: &mut u64,
    cache_misses: &mut u64,
) -> Result<f64, CoordinatorError> {
    let (epd, move_seq, node_type, terminal, already_expanded) = {
        let leaf = tree.get(leaf_id).ok_or(CoordinatorError::NodeNotFound { node_id: leaf_id })?;
        (
            leaf.epd.clone(),
            leaf.move_sequence.clone(),
            leaf.node_type,
            leaf.terminal_value,
            leaf.expanded,
        )
    };

    let white_to_move = move_seq.is_empty() || move_seq.split_whitespace().count() % 2 == 0;

    match terminal {
        Some(tv) => return Ok(tv),
        None => {}
    }

    if already_expanded {
        let leaf = tree.get(leaf_id).ok_or(CoordinatorError::NodeNotFound { node_id: leaf_id })?;
        match leaf.wdl {
            Some(wdl) => {
                let eval = EngineEval { wdl, policy: Default::default(), q_values: Default::default() };
                return Ok(eval.value_white(config.contempt, white_to_move));
            }
            None => {}
        }
    }

    let position = PositionState::from_moves(&move_seq)
        .map_err(|e| CoordinatorError::Position { source: e, move_sequence: move_seq.clone() })?;
    match position.terminal_value() {
        Some(tv) => {
            let leaf = tree.get_mut(leaf_id).unwrap();
            leaf.terminal_value = Some(tv);
            leaf.expanded = true;
            return Ok(tv);
        }
        None => {}
    }

    let (engine_eval, maia_policy) = get_evals_cached(
        &epd, &move_seq, backend, cache, config, cache_hits, cache_misses,
    )?;

    let value = engine_eval.value_white(config.contempt, white_to_move);

    {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.engine_policy = Some(engine_eval.policy.clone());
        leaf.engine_q_values = Some(engine_eval.q_values.clone());
        leaf.maia_policy = Some(maia_policy.clone());
        leaf.wdl = Some(engine_eval.wdl);
    }

    let mut candidates = match node_type {
        NodeType::Max => candidate_moves_max(&engine_eval.policy, &maia_policy, config),
        NodeType::Chance => candidate_moves_chance(&maia_policy, config),
    };

    let effective_width = (config.max_width as f64 * config.width_decay.powi(depth as i32))
        .floor() as usize;
    let effective_width = effective_width.max(2);
    if candidates.len() > effective_width {
        candidates.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        candidates.truncate(effective_width);
        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        if sum > 0.0 {
            candidates.iter_mut().for_each(|(_, p)| *p /= sum);
        }
    }

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
                match child_terminal {
                    Some(tv) => { tree.get_mut(child_id).unwrap().terminal_value = Some(tv); }
                    None => {}
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

fn get_evals_cached(
    epd: &str,
    move_seq: &str,
    backend: &mut EvalBackend,
    cache: Option<&Cache>,
    config: &Config,
    cache_hits: &mut u64,
    cache_misses: &mut u64,
) -> Result<(EngineEval, HashMap<String, f32>), CoordinatorError> {
    let cached_engine = match cache {
        Some(c) => c.get_engine_eval(epd).map(|(w, d, l, policy, q_values)| {
            *cache_hits += 1;
            EngineEval { wdl: (w, d, l), policy, q_values }
        }),
        None => None,
    };
    let cached_maia = match cache {
        Some(c) => {
            let result = c.get_maia_policy(move_seq);
            if result.is_some() { *cache_hits += 1; }
            result
        }
        None => None,
    };

    if let (Some(engine_eval), Some(maia_policy)) = (cached_engine.clone(), cached_maia.clone()) {
        return Ok((engine_eval, maia_policy));
    }

    let engine_eval = match cached_engine {
        Some(eval) => eval,
        None => {
            *cache_misses += 1;
            let eval = match backend {
                EvalBackend::Lc0 { engine, .. } => engine
                    .evaluate(move_seq, config.engine_nodes)
                    .map_err(|e| CoordinatorError::Engine {
                        source: e,
                        move_sequence: move_seq.to_string(),
                    })?,
                EvalBackend::Onnx { evaluator } => evaluator
                    .evaluate_engine(move_seq)
                    .map_err(|e| CoordinatorError::NN {
                        source: e,
                        move_sequence: move_seq.to_string(),
                    })?,
            };
            if let Some(c) = cache {
                let _ = c.put_engine_eval(epd, eval.wdl, &eval.policy, &eval.q_values);
            }
            eval
        }
    };

    let maia_policy = match cached_maia {
        Some(policy) => policy,
        None => {
            *cache_misses += 1;
            let policy = match backend {
                EvalBackend::Lc0 { maia, .. } => maia
                    .predict(move_seq)
                    .map_err(|e| CoordinatorError::Maia {
                        source: e,
                        move_sequence: move_seq.to_string(),
                    })?,
                EvalBackend::Onnx { evaluator } => evaluator
                    .evaluate_maia(move_seq)
                    .map_err(|e| CoordinatorError::NN {
                        source: e,
                        move_sequence: move_seq.to_string(),
                    })?,
            };
            if let Some(c) = cache {
                let _ = c.put_maia_policy(move_seq, &policy);
            }
            policy
        }
    };

    Ok((engine_eval, maia_policy))
}
