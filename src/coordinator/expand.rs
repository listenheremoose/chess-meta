use std::collections::HashMap;

use crate::cache::Cache;
use crate::config::Config;
use crate::engine::{Engine, EngineEval};
use crate::maia::MaiaEngine;
use crate::position::PositionState;
use crate::search::{NodeId, NodeType, SearchTree, candidate_moves_max, candidate_moves_chance};

use super::CoordinatorError;

/// Expand a leaf node and return its evaluation (from White's perspective).
pub(super) fn expand_and_evaluate(
    tree: &mut SearchTree,
    leaf_id: NodeId,
    config: &Config,
    engine: &mut Engine,
    maia: &mut MaiaEngine,
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

    if let Some(tv) = terminal {
        return Ok(tv);
    }

    if already_expanded {
        let leaf = tree.get(leaf_id).ok_or(CoordinatorError::NodeNotFound { node_id: leaf_id })?;
        if let Some(wdl) = leaf.wdl {
            let eval = EngineEval { wdl, policy: Default::default(), q_values: Default::default() };
            return Ok(eval.value_white(config.contempt, white_to_move));
        }
    }

    let position = PositionState::from_moves(&move_seq)
        .map_err(|e| CoordinatorError::Position { source: e, move_sequence: move_seq.clone() })?;
    if let Some(tv) = position.terminal_value() {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.terminal_value = Some(tv);
        leaf.expanded = true;
        return Ok(tv);
    }

    let engine_eval = get_engine_eval_cached(&epd, &move_seq, engine, cache, config, cache_hits, cache_misses)?;
    let maia_policy = get_maia_policy_cached(&move_seq, maia, cache, cache_hits, cache_misses)?;

    let value = engine_eval.value_white(config.contempt, white_to_move);

    {
        let leaf = tree.get_mut(leaf_id).unwrap();
        leaf.engine_policy = Some(engine_eval.policy.clone());
        leaf.engine_q_values = Some(engine_eval.q_values.clone());
        leaf.maia_policy = Some(maia_policy.clone());
        leaf.wdl = Some(engine_eval.wdl);
    }

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

fn get_engine_eval_cached(
    epd: &str,
    move_seq: &str,
    engine: &mut Engine,
    cache: Option<&Cache>,
    config: &Config,
    cache_hits: &mut u64,
    cache_misses: &mut u64,
) -> Result<EngineEval, CoordinatorError> {
    if let Some(c) = cache {
        if let Some((w, d, l, policy, q_values)) = c.get_engine_eval(epd) {
            *cache_hits += 1;
            return Ok(EngineEval { wdl: (w, d, l), policy, q_values });
        }
    }

    *cache_misses += 1;
    let eval = engine.evaluate(move_seq, config.engine_nodes)
        .map_err(|e| CoordinatorError::Engine { source: e, move_sequence: move_seq.to_string() })?;
    if let Some(c) = cache {
        let _ = c.put_engine_eval(epd, eval.wdl, &eval.policy, &eval.q_values);
    }
    Ok(eval)
}

fn get_maia_policy_cached(
    move_seq: &str,
    maia: &mut MaiaEngine,
    cache: Option<&Cache>,
    cache_hits: &mut u64,
    cache_misses: &mut u64,
) -> Result<HashMap<String, f32>, CoordinatorError> {
    if let Some(c) = cache {
        if let Some(cached) = c.get_maia_policy(move_seq) {
            *cache_hits += 1;
            return Ok(cached);
        }
    }

    *cache_misses += 1;
    let policy = maia.predict(move_seq)
        .map_err(|e| CoordinatorError::Maia { source: e, move_sequence: move_seq.to_string() })?;
    if let Some(c) = cache {
        let _ = c.put_maia_policy(move_seq, &policy);
    }
    Ok(policy)
}
