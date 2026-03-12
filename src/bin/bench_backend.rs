//! Backend benchmark: compares ONNX (batched) vs lc0 UCI throughput.
//!
//! Usage: cargo run --release --bin bench_backend [iterations]
//!
//! Loads settings.toml for engine paths. Requires both lc0+weights and ONNX paths configured.

use std::time::Instant;

use chess_meta::config::Config;
use chess_meta::engine::Engine;
use chess_meta::maia::MaiaEngine;
use chess_meta::nn::NNEvaluator;
use chess_meta::position::PositionState;
use chess_meta::search::{
    NodeId, NodeType, SearchState, SearchTree,
    apply_virtual_loss, backpropagate, revert_virtual_loss,
    select, select_with_path,
};

/// Positions to benchmark (move sequences from startpos).
const POSITIONS: &[&str] = &[
    "",                          // startpos
    "e2e4",                      // 1. e4
    "e2e4 e7e5 g1f3",           // 1. e4 e5 2. Nf3
    "d2d4 g8f6 c2c4 e7e6 b1c3", // 1. d4 Nf6 2. c4 e6 3. Nc3
];

fn main() {
    simplelog::SimpleLogger::init(simplelog::LevelFilter::Warn, simplelog::Config::default()).ok();

    let config = Config::load();
    let iterations: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5000);

    println!("Backend Benchmark");
    println!("=================");
    println!("Iterations per position: {iterations}");
    println!("Positions: {}", POSITIONS.len());
    println!();

    let has_lc0 = config.engine_paths_configured();
    let has_onnx = config.onnx_configured();

    if !has_lc0 && !has_onnx {
        eprintln!("Error: No backends configured in settings.toml");
        eprintln!("Need lc0_path + weights, or engine_onnx_path + maia_onnx_path");
        std::process::exit(1);
    }

    // ── ONNX batched ────────────────────────────────────────────────────
    if has_onnx {
        for &batch_size in &[64, 128, 256] {
            println!("--- ONNX batched (batch_size={batch_size}) ---");
            let mut evaluator = NNEvaluator::new(&config.engine_onnx_path, &config.maia_onnx_path)
                .expect("Failed to load ONNX models");

            // Warmup
            let _ = evaluator.evaluate_batch(&[""]);

            for &pos in POSITIONS {
                let label = if pos.is_empty() { "startpos" } else { pos };
                let position = PositionState::from_moves(pos).unwrap();
                let mut tree = SearchTree::new(position.epd.clone(), position.move_sequence.clone(), NodeType::Max);
                let mut state = SearchState::new();

                let start = Instant::now();
                let mut iters_done: u64 = 0;
                let mut total_unique: u64 = 0;
                let mut total_selected: u64 = 0;
                while iters_done < iterations {
                    let actual_batch = batch_size.min((iterations - iters_done) as usize);

                    // Phase 1: Select leaves with virtual loss.
                    let mut leaves: Vec<(NodeId, u32, Vec<NodeId>)> = Vec::with_capacity(actual_batch);
                    let mut seen = std::collections::HashSet::new();
                    for _ in 0..actual_batch {
                        let (leaf_id, depth, path) = select_with_path(&tree, &config, &mut state);
                        apply_virtual_loss(&mut tree, &path);
                        leaves.push((leaf_id, depth, path));
                    }

                    // Phase 2: Collect move sequences for unique non-terminal leaves.
                    let mut eval_requests: Vec<(usize, String)> = Vec::new();
                    let mut leaf_values: Vec<Option<f64>> = vec![None; actual_batch];

                    for (i, (leaf_id, _depth, _path)) in leaves.iter().enumerate() {
                        if !seen.insert(*leaf_id) {
                            leaf_values[i] = Some(tree.get(*leaf_id).map(|n| n.q_value()).unwrap_or(0.5));
                            continue;
                        }
                        let leaf = tree.get(*leaf_id).unwrap();
                        let move_seq = leaf.move_sequence.clone();
                        let leaf_pos = match PositionState::from_moves(&move_seq) {
                            Ok(p) => p,
                            Err(_) => { leaf_values[i] = Some(0.5); continue; }
                        };
                        if let Some(tv) = leaf_pos.terminal_value() {
                            leaf_values[i] = Some(tv);
                            continue;
                        }
                        eval_requests.push((i, move_seq));
                    }

                    total_unique += eval_requests.len() as u64;
                    total_selected += actual_batch as u64;

                    // Phase 3: Batch NN inference.
                    let batch_results = if !eval_requests.is_empty() {
                        let seqs: Vec<&str> = eval_requests.iter().map(|(_, s)| s.as_str()).collect();
                        evaluator.evaluate_batch(&seqs).unwrap()
                    } else {
                        Vec::new()
                    };

                    // Phase 4: Expand and backprop.
                    let mut result_idx = 0;
                    for (i, (leaf_id, _depth, path)) in leaves.iter().enumerate() {
                        revert_virtual_loss(&mut tree, path);

                        let value = if let Some(v) = leaf_values[i] {
                            v
                        } else {
                            let (engine_eval, _maia_policy) = &batch_results[result_idx];
                            result_idx += 1;
                            let value = engine_eval.value_white(config.contempt, true);

                            // Expand
                            let node_type = tree.get(*leaf_id).unwrap().node_type;
                            let child_type = match node_type {
                                NodeType::Max => NodeType::Chance,
                                NodeType::Chance => NodeType::Max,
                            };
                            if !tree.get(*leaf_id).unwrap().expanded {
                                let move_seq = tree.get(*leaf_id).unwrap().move_sequence.clone();
                                if let Ok(leaf_pos) = PositionState::from_moves(&move_seq) {
                                    let candidates: Vec<_> = engine_eval.policy.iter()
                                        .take(5)
                                        .map(|(m, p)| (m.clone(), *p as f64 / 100.0))
                                        .collect();
                                    for (uci_move, prior) in &candidates {
                                        if let Ok(new_pos) = leaf_pos.apply_uci(uci_move) {
                                            tree.add_child(*leaf_id, uci_move.clone(), child_type, new_pos.epd, new_pos.move_sequence, *prior);
                                        }
                                    }
                                    tree.get_mut(*leaf_id).unwrap().expanded = true;
                                }
                            }
                            value
                        };

                        backpropagate(&mut tree, *leaf_id, value);
                        iters_done += 1;
                    }
                }
                let elapsed = start.elapsed();
                let it_per_sec = iters_done as f64 / elapsed.as_secs_f64();
                let unique_pct = if total_selected > 0 { 100.0 * total_unique as f64 / total_selected as f64 } else { 0.0 };
                println!("  {label:<40} {iters_done} iters in {:.2}s  ({it_per_sec:.1} it/s, {unique_pct:.0}% unique)", elapsed.as_secs_f64());
            }
            println!();
        }
    }

    // ── lc0 UCI backend ─────────────────────────────────────────────────
    if has_lc0 {
        println!("--- lc0 UCI (sequential, nodes=1) ---");
        let mut engine = Engine::new(
            &config.lc0_path,
            &config.engine_weights_path,
            config.nn_cache_size_mb,
            config.ucinewgame_interval,
        ).expect("Failed to start engine");
        let mut maia = MaiaEngine::new(
            &config.lc0_path,
            &config.maia_weights_path,
            config.ucinewgame_interval,
        ).expect("Failed to start maia");

        // Warmup
        let _ = engine.evaluate("", 1);
        let _ = maia.predict("");

        for &pos in POSITIONS {
            let label = if pos.is_empty() { "startpos" } else { pos };
            let position = PositionState::from_moves(pos).unwrap();
            let mut tree = SearchTree::new(position.epd.clone(), position.move_sequence.clone(), NodeType::Max);
            let mut state = SearchState::new();

            let start = Instant::now();
            for _ in 0..iterations {
                let (leaf_id, _depth) = select(&tree, &config, &mut state);
                let leaf = tree.get(leaf_id).unwrap();
                let move_seq = leaf.move_sequence.clone();
                let epd = leaf.epd.clone();
                let node_type = leaf.node_type;

                let leaf_pos = match PositionState::from_moves(&move_seq) {
                    Ok(p) => p,
                    Err(_) => break,
                };
                if leaf_pos.terminal_value().is_some() {
                    backpropagate(&mut tree, leaf_id, leaf_pos.terminal_value().unwrap());
                    continue;
                }

                let engine_eval = engine.evaluate(&move_seq, 1).unwrap();
                let maia_policy = maia.predict(&move_seq).unwrap();
                let value = engine_eval.value_white(config.contempt, true);

                // Minimal expansion
                let child_type = match node_type {
                    NodeType::Max => NodeType::Chance,
                    NodeType::Chance => NodeType::Max,
                };
                if !tree.get(leaf_id).unwrap().expanded {
                    let candidates: Vec<_> = engine_eval.policy.iter()
                        .take(5)
                        .map(|(m, p)| (m.clone(), *p as f64 / 100.0))
                        .collect();
                    for (uci_move, prior) in &candidates {
                        if let Ok(new_pos) = leaf_pos.apply_uci(uci_move) {
                            tree.add_child(leaf_id, uci_move.clone(), child_type, new_pos.epd, new_pos.move_sequence, *prior);
                        }
                    }
                    tree.get_mut(leaf_id).unwrap().expanded = true;
                }

                backpropagate(&mut tree, leaf_id, value);
            }
            let elapsed = start.elapsed();
            let it_per_sec = iterations as f64 / elapsed.as_secs_f64();
            println!("  {label:<40} {iterations} iters in {:.2}s  ({it_per_sec:.1} it/s)", elapsed.as_secs_f64());
        }
        println!();
    }

    println!("Done.");
}
