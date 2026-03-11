//! Integration tests that require lc0 and neural network files.
//!
//! These are `#[ignore]` by default. Run them with:
//!   cargo test --test lc0_integration -- --ignored
//!
//! Required environment variables:
//!   LC0_PATH           — path to lc0 executable
//!   ENGINE_WEIGHTS     — path to engine neural network weights
//!   MAIA_WEIGHTS       — path to Maia neural network weights
//!
//! Or, if the reference project's settings.toml exists at
//! c:\code\rust-chess\settings.toml, paths are read from there automatically.

use chess_meta::cache::Cache;
use chess_meta::config::Config;
use chess_meta::engine::Engine;
use chess_meta::maia::MaiaEngine;
use chess_meta::position::PositionState;
use chess_meta::search::{
    NodeType, SearchTree, backpropagate, candidate_moves_chance, candidate_moves_max, select,
};

/// Read lc0/weights paths from env vars, falling back to rust-chess settings.toml.
fn load_paths() -> (String, String, String) {
    if let (Ok(lc0), Ok(engine_w), Ok(maia_w)) = (
        std::env::var("LC0_PATH"),
        std::env::var("ENGINE_WEIGHTS"),
        std::env::var("MAIA_WEIGHTS"),
    ) {
        return (lc0, engine_w, maia_w);
    }

    // Fallback: parse rust-chess settings.toml
    let settings_path = r"c:\code\rust-chess\settings.toml";
    let text = std::fs::read_to_string(settings_path)
        .unwrap_or_else(|_| panic!(
            "Set LC0_PATH, ENGINE_WEIGHTS, MAIA_WEIGHTS env vars, \
             or ensure {settings_path} exists"
        ));

    let table: toml::Table = text.parse().expect("Failed to parse settings.toml");
    let lc0 = table["lc0_path"].as_str().unwrap().to_string();
    let engine_w = table["lc0_weights"].as_str().unwrap().to_string();
    let maia_w = table["maia_weights"].as_str().unwrap().to_string();

    (lc0, engine_w, maia_w)
}

// ── Engine tests (single process, multiple assertions) ───────────────

#[test]
#[ignore]
fn engine_eval_and_cache() {
    let (lc0, weights, _) = load_paths();
    let mut engine = Engine::new(&lc0, &weights, 128, 500).unwrap();

    // — Startpos basics —
    let eval = engine.evaluate("", 1).unwrap();

    let (w, d, l) = eval.wdl;
    assert!(
        (w + d + l) as i32 >= 990 && (w + d + l) as i32 <= 1010,
        "WDL should sum to ~1000, got {w}+{d}+{l}={}",
        w + d + l
    );
    assert!(!eval.policy.is_empty(), "policy should not be empty");
    assert!(
        eval.policy.contains_key("e2e4") || eval.policy.contains_key("d2d4"),
        "policy should contain e2e4 or d2d4"
    );

    let value = eval.value_white(0.6, true); // startpos = White to move
    assert!(
        value > 0.3 && value < 0.7,
        "startpos value should be near 0.5, got {value}"
    );

    // — Top policy moves are sorted —
    let top5 = eval.top_policy_moves(5);
    assert!(top5.len() <= 5);
    for i in 1..top5.len() {
        assert!(
            top5[i - 1].1 >= top5[i].1,
            "top policy moves not sorted: {} ({}) before {} ({})",
            top5[i - 1].0, top5[i - 1].1, top5[i].0, top5[i].1
        );
    }

    // — Multiple positions sequentially —
    let sequences = ["e2e4", "e2e4 e7e5", "d2d4 d7d5 c2c4"];
    for seq in &sequences {
        let e = engine.evaluate(seq, 1).unwrap();
        assert!(!e.policy.is_empty(), "empty policy for '{seq}'");
        let sum = e.wdl.0 + e.wdl.1 + e.wdl.2;
        assert!(sum >= 990 && sum <= 1010, "position '{seq}' WDL sum = {sum}");
    }

    // — Cache round-trip with real eval data —
    let cache = Cache::open_in_memory().unwrap();
    let pos = PositionState::from_moves("e2e4 e7e5").unwrap();
    let eval2 = engine.evaluate(&pos.move_sequence, 1).unwrap();

    cache
        .put_engine_eval(&pos.epd, eval2.wdl, &eval2.policy, &eval2.q_values)
        .unwrap();

    let (cw, cd, cl, got_policy, got_q) = cache.get_engine_eval(&pos.epd).unwrap();
    assert_eq!((cw, cd, cl), eval2.wdl);
    assert_eq!(got_policy.len(), eval2.policy.len());
    for (m, p) in &eval2.policy {
        let cached = got_policy.get(m).unwrap_or_else(|| panic!("missing {m}"));
        assert!((cached - p).abs() < 1e-6, "policy mismatch for {m}");
    }
    for (m, q) in &eval2.q_values {
        let cached = got_q.get(m).unwrap_or_else(|| panic!("missing q for {m}"));
        assert!((cached - q).abs() < 1e-6, "q mismatch for {m}");
    }
}

// ── Maia tests (single process, multiple assertions) ─────────────────

#[test]
#[ignore]
fn maia_predict_and_cache() {
    let (lc0, _, maia_w) = load_paths();
    let mut maia = MaiaEngine::new(&lc0, &maia_w, 500).unwrap();

    // — Startpos basics —
    let policy = maia.predict("").unwrap();

    assert!(!policy.is_empty(), "Maia policy should not be empty");
    let has_common = policy.contains_key("e2e4")
        || policy.contains_key("d2d4")
        || policy.contains_key("g1f3");
    assert!(has_common, "Maia should predict common opening moves");

    let sum: f32 = policy.values().sum();
    assert!(
        sum > 90.0 && sum < 110.0,
        "Maia policy should sum to ~100%, got {sum}"
    );

    // — Different positions produce different move sets —
    let policy_e4 = maia.predict("e2e4").unwrap();
    assert!(!policy_e4.is_empty(), "after e4 should have moves");
    assert!(
        !policy_e4.contains_key("e2e4"),
        "after 1.e4, e2e4 shouldn't be in Black's moves"
    );

    // — Cache round-trip —
    let cache = Cache::open_in_memory().unwrap();
    let move_seq = "e2e4 e7e5 g1f3";
    let policy3 = maia.predict(move_seq).unwrap();

    cache.put_maia_policy(move_seq, &policy3).unwrap();
    let got = cache.get_maia_policy(move_seq).unwrap();
    assert_eq!(got.len(), policy3.len());
    for (m, p) in &policy3 {
        assert!((got[m] - p).abs() < 1e-6);
    }
}

// ── Combined: engine + Maia → tree expansion + mini MCTS ─────────────

#[test]
#[ignore]
fn search_tree_with_real_engines() {
    let (lc0, weights, maia_w) = load_paths();
    let mut engine = Engine::new(&lc0, &weights, 128, 500).unwrap();
    let mut maia = MaiaEngine::new(&lc0, &maia_w, 500).unwrap();
    let config = Config::default();

    let start = PositionState::startpos();
    let mut tree = SearchTree::new(start.epd.clone(), start.move_sequence.clone(), NodeType::Max);

    // ── Part 1: Expand root ──────────────────────────────────────────

    let eval = engine.evaluate("", 1).unwrap();
    let maia_policy = maia.predict("").unwrap();

    let candidates = candidate_moves_max(&eval.policy, &maia_policy, &config);
    assert!(
        candidates.len() >= 3,
        "should have at least 3 candidate moves, got {}",
        candidates.len()
    );

    for (uci_move, prior) in &candidates {
        let child_pos = start.apply_uci(uci_move).unwrap();
        tree.add_child(
            0,
            uci_move.clone(),
            NodeType::Chance,
            child_pos.epd.clone(),
            child_pos.move_sequence.clone(),
            *prior,
        );
    }
    tree.root_mut().expanded = true;
    tree.root_mut().engine_policy = Some(eval.policy.clone());
    tree.root_mut().engine_q_values = Some(eval.q_values.clone());
    tree.root_mut().maia_policy = Some(maia_policy.clone());
    tree.root_mut().wdl = Some(eval.wdl);

    assert_eq!(tree.root().children.len(), candidates.len());
    for &child_id in &tree.root().children {
        let child = tree.get(child_id).unwrap();
        assert_eq!(child.node_type, NodeType::Chance);
        assert!(!child.epd.is_empty());
        assert!(child.prior > 0.0);
    }

    let prior_sum: f64 = tree
        .root()
        .children
        .iter()
        .map(|&id| tree.get(id).unwrap().prior)
        .sum();
    assert!(
        (prior_sum - 1.0).abs() < 0.05,
        "priors should sum to ~1, got {prior_sum}"
    );

    // ── Part 2: Mini MCTS loop (5 iterations) ───────────────────────

    let iterations: u64 = 5;

    for _iter in 0..iterations {
        // 1. Select
        let leaf_id = select(&tree, &config);
        let leaf = tree.get(leaf_id).unwrap();

        if leaf.expanded {
            let value = leaf.terminal_value.unwrap_or(0.5);
            backpropagate(&mut tree, leaf_id, value);
            continue;
        }

        let leaf_seq = leaf.move_sequence.clone();
        let leaf_type = leaf.node_type;
        let white_to_move = if leaf_seq.is_empty() {
            true
        } else {
            leaf_seq.split_whitespace().count() % 2 == 0
        };

        // 2. Evaluate
        let leaf_eval = engine.evaluate(&leaf_seq, 1).unwrap();
        let leaf_maia = maia.predict(&leaf_seq).unwrap();
        let value = leaf_eval.value_white(config.contempt, white_to_move);

        // 3. Expand
        let leaf_candidates = match leaf_type {
            NodeType::Max => candidate_moves_max(
                &leaf_eval.policy,
                &leaf_maia,
                &config,
            ),
            NodeType::Chance => candidate_moves_chance(&leaf_maia, &config),
        };

        let child_type = match leaf_type {
            NodeType::Max => NodeType::Chance,
            NodeType::Chance => NodeType::Max,
        };

        let leaf_pos = if leaf_seq.is_empty() {
            PositionState::startpos()
        } else {
            PositionState::from_moves(&leaf_seq).unwrap()
        };

        for (uci_move, prior) in &leaf_candidates {
            if let Ok(child_pos) = leaf_pos.apply_uci(uci_move) {
                tree.add_child(
                    leaf_id,
                    uci_move.clone(),
                    child_type,
                    child_pos.epd.clone(),
                    child_pos.move_sequence.clone(),
                    *prior,
                );
            }
        }

        {
            let leaf_mut = tree.get_mut(leaf_id).unwrap();
            leaf_mut.expanded = true;
            leaf_mut.engine_policy = Some(leaf_eval.policy);
            leaf_mut.engine_q_values = Some(leaf_eval.q_values);
            leaf_mut.maia_policy = Some(leaf_maia);
            leaf_mut.wdl = Some(leaf_eval.wdl);
        }

        // 4. Backpropagate
        backpropagate(&mut tree, leaf_id, value);
    }

    // ── Verify invariants ────────────────────────────────────────────

    assert!(
        tree.node_count() > 1,
        "tree should have grown beyond root, has {} nodes",
        tree.node_count()
    );

    // Root visits = initial expansion (0) + loop iterations
    // (root had 0 visits before the loop since we expanded manually without backprop)
    assert_eq!(
        tree.root().visit_count, iterations,
        "root should have {iterations} visits"
    );

    for (id, node) in &tree.nodes {
        let q = node.q_value();
        assert!(
            (0.0..=1.0).contains(&q),
            "node {id} q={q} out of range"
        );
    }

    for (_id, node) in &tree.nodes {
        let expected_child_type = match node.node_type {
            NodeType::Max => NodeType::Chance,
            NodeType::Chance => NodeType::Max,
        };
        for &cid in &node.children {
            let child = tree.get(cid).unwrap();
            assert_eq!(child.node_type, expected_child_type);
        }
    }
}
