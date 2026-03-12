use std::collections::HashMap;

use chess_meta::config::Config;
use chess_meta::position::PositionState;
use chess_meta::search::{
    NodeId, NodeType, SearchState, SearchTree, backpropagate, best_root_move,
    candidate_moves_chance, candidate_moves_max, root_move_infos, select,
};

/// Build a realistic tree from startpos with a few expanded MAX and CHANCE nodes.
/// Returns the tree and config.
fn build_test_tree() -> (SearchTree, Config) {
    let config = Config::default();
    let start = PositionState::startpos();
    let mut tree = SearchTree::new(start.epd.clone(), start.move_sequence.clone(), NodeType::Max);

    // Expand root (MAX) with three children — our candidate moves.
    let pos_e4 = start.apply_uci("e2e4").unwrap();
    let pos_d4 = start.apply_uci("d2d4").unwrap();
    let pos_nf3 = start.apply_uci("g1f3").unwrap();

    let e4_id = tree.add_child(
        NodeId(0),
        "e2e4".into(),
        NodeType::Chance,
        pos_e4.epd.clone(),
        pos_e4.move_sequence.clone(),
        0.45,
    );
    let d4_id = tree.add_child(
        NodeId(0),
        "d2d4".into(),
        NodeType::Chance,
        pos_d4.epd.clone(),
        pos_d4.move_sequence.clone(),
        0.35,
    );
    let _nf3_id = tree.add_child(
        NodeId(0),
        "g1f3".into(),
        NodeType::Chance,
        pos_nf3.epd.clone(),
        pos_nf3.move_sequence.clone(),
        0.20,
    );

    // Mark root as expanded.
    tree.root_mut().expanded = true;
    tree.root_mut().engine_policy = Some(
        [("e2e4", 45.0f32), ("d2d4", 35.0), ("g1f3", 20.0)]
            .into_iter()
            .map(|(m, p)| (m.to_string(), p))
            .collect(),
    );

    // Expand e4 (CHANCE) — opponent responses.
    let pos_e4_e5 = pos_e4.apply_uci("e7e5").unwrap();
    let pos_e4_c5 = pos_e4.apply_uci("c7c5").unwrap();

    let e4_e5_id = tree.add_child(
        e4_id,
        "e7e5".into(),
        NodeType::Max,
        pos_e4_e5.epd.clone(),
        pos_e4_e5.move_sequence.clone(),
        0.40,
    );
    let _e4_c5_id = tree.add_child(
        e4_id,
        "c7c5".into(),
        NodeType::Max,
        pos_e4_c5.epd.clone(),
        pos_e4_c5.move_sequence.clone(),
        0.35,
    );

    tree.get_mut(e4_id).unwrap().expanded = true;
    tree.get_mut(e4_id).unwrap().maia_policy = Some(
        [("e7e5", 40.0f32), ("c7c5", 35.0), ("d7d5", 25.0)]
            .into_iter()
            .map(|(m, p)| (m.to_string(), p))
            .collect(),
    );

    // Give some visit counts and values via backpropagation.
    backpropagate(&mut tree, e4_e5_id, 0.55); // Slightly good for White
    backpropagate(&mut tree, e4_e5_id, 0.60);
    backpropagate(&mut tree, d4_id, 0.50); // Neutral

    (tree, config)
}

// ── Tree structure invariants ────────────────────────────────────────

#[test]
fn tree_visit_counts_sum_correctly() {
    let (tree, _config) = build_test_tree();

    // Root visit count should equal sum of child visit counts.
    let root = tree.root();
    let child_visits: u64 = root
        .children
        .iter()
        .map(|&id| tree.get(id).unwrap().visit_count)
        .sum();
    assert_eq!(
        root.visit_count, child_visits,
        "root visits ({}) != sum of children ({})",
        root.visit_count, child_visits
    );
}

#[test]
fn tree_values_stay_in_range() {
    let (tree, _config) = build_test_tree();

    for node in &tree.nodes {
        let q = node.q_value();
        assert!(
            (0.0..=1.0).contains(&q),
            "node {:?} has q_value {q} outside [0, 1]",
            node.id
        );
    }
}

#[test]
fn node_types_alternate() {
    let (tree, _config) = build_test_tree();

    for node in &tree.nodes {
        for &child_id in &node.children {
            let child = tree.get(child_id).unwrap();
            let expected = match node.node_type {
                NodeType::Max => NodeType::Chance,
                NodeType::Chance => NodeType::Max,
            };
            assert_eq!(
                child.node_type, expected,
                "child {:?} of {:?} node should be {:?}, got {:?}",
                child_id, node.node_type, expected, child.node_type
            );
        }
    }
}

#[test]
fn parent_child_links_consistent() {
    let (tree, _config) = build_test_tree();

    for node in &tree.nodes {
        let id = node.id;
        // Every child should point back to this node as parent.
        for &child_id in &node.children {
            let child = tree.get(child_id).unwrap();
            assert_eq!(
                child.parent,
                Some(id),
                "child {:?} parent mismatch: expected {:?}, got {:?}",
                child_id, id, child.parent
            );
        }

        // If node has a parent, the parent should list it as a child.
        if let Some(parent_id) = node.parent {
            let parent = tree.get(parent_id).unwrap();
            assert!(
                parent.children.contains(&id),
                "node {:?} claims parent {:?}, but parent doesn't list it as child",
                id, parent_id
            );
        }
    }
}

// ── Backpropagation ──────────────────────────────────────────────────

#[test]
fn backprop_increments_all_ancestors() {
    let start = PositionState::startpos();
    let mut tree = SearchTree::new(start.epd.clone(), start.move_sequence.clone(), NodeType::Max);

    let pos_e4 = start.apply_uci("e2e4").unwrap();
    let e4_id = tree.add_child(NodeId(0), "e2e4".into(), NodeType::Chance, pos_e4.epd.clone(), pos_e4.move_sequence.clone(), 0.5);
    tree.root_mut().expanded = true;

    let pos_e5 = pos_e4.apply_uci("e7e5").unwrap();
    let e5_id = tree.add_child(e4_id, "e7e5".into(), NodeType::Max, pos_e5.epd.clone(), pos_e5.move_sequence.clone(), 0.4);
    tree.get_mut(e4_id).unwrap().expanded = true;

    backpropagate(&mut tree, e5_id, 0.6);

    assert_eq!(tree.get(e5_id).unwrap().visit_count, 1);
    assert_eq!(tree.get(e4_id).unwrap().visit_count, 1);
    assert_eq!(tree.root().visit_count, 1);

    // All should have the same total_value (from White's perspective).
    assert!((tree.get(e5_id).unwrap().total_value - 0.6).abs() < 1e-9);
    assert!((tree.get(e4_id).unwrap().total_value - 0.6).abs() < 1e-9);
    assert!((tree.root().total_value - 0.6).abs() < 1e-9);
}

#[test]
fn multiple_backprops_accumulate() {
    let start = PositionState::startpos();
    let mut tree = SearchTree::new(start.epd.clone(), start.move_sequence.clone(), NodeType::Max);

    let pos_e4 = start.apply_uci("e2e4").unwrap();
    let e4_id = tree.add_child(NodeId(0), "e2e4".into(), NodeType::Chance, pos_e4.epd.clone(), pos_e4.move_sequence.clone(), 0.5);
    tree.root_mut().expanded = true;

    backpropagate(&mut tree, e4_id, 0.6);
    backpropagate(&mut tree, e4_id, 0.4);
    backpropagate(&mut tree, e4_id, 0.8);

    assert_eq!(tree.get(e4_id).unwrap().visit_count, 3);
    let expected_q = (0.6 + 0.4 + 0.8) / 3.0;
    assert!(
        (tree.get(e4_id).unwrap().q_value() - expected_q).abs() < 1e-9,
        "expected q={expected_q}, got {}",
        tree.get(e4_id).unwrap().q_value()
    );
}

// ── Selection ────────────────────────────────────────────────────────

#[test]
fn select_returns_unexpanded_leaf() {
    let (tree, config) = build_test_tree();
    let mut state = SearchState::new();
    let (leaf, _depth) = select(&tree, &config, &mut state);
    let node = tree.get(leaf).unwrap();

    // Selected node should be either unexpanded or have no children.
    assert!(
        !node.expanded || node.children.is_empty(),
        "select returned expanded node {:?} with {} children",
        leaf, node.children.len()
    );
}

#[test]
fn select_explores_unvisited_children() {
    // With FPU reduction, unvisited children should eventually be selected.
    let start = PositionState::startpos();
    let config = Config::default();
    let mut tree = SearchTree::new(start.epd.clone(), start.move_sequence.clone(), NodeType::Max);

    let pos_e4 = start.apply_uci("e2e4").unwrap();
    let pos_d4 = start.apply_uci("d2d4").unwrap();

    let e4_id = tree.add_child(NodeId(0), "e2e4".into(), NodeType::Chance, pos_e4.epd.clone(), pos_e4.move_sequence.clone(), 0.5);
    let d4_id = tree.add_child(NodeId(0), "d2d4".into(), NodeType::Chance, pos_d4.epd.clone(), pos_d4.move_sequence.clone(), 0.5);
    tree.root_mut().expanded = true;

    // Visit e4 many times so d4 becomes attractive via exploration.
    for _ in 0..20 {
        backpropagate(&mut tree, e4_id, 0.5);
    }

    let mut state = SearchState::new();
    let (selected, _depth) = select(&tree, &config, &mut state);
    assert_eq!(selected, d4_id, "expected unvisited d4 to be selected");
}

// ── Candidate moves ─────────────────────────────────────────────────

#[test]
fn candidate_moves_max_blends_policies() {
    let config = Config::default();

    let engine_policy: HashMap<String, f32> = [
        ("e2e4".into(), 50.0),
        ("d2d4".into(), 30.0),
        ("c2c4".into(), 20.0),
    ]
    .into();
    let maia_policy: HashMap<String, f32> = [
        ("e2e4".into(), 40.0),
        ("d2d4".into(), 25.0),
        ("g1f3".into(), 20.0),
        ("b1c3".into(), 10.0),
        ("g2g3".into(), 5.0),
    ]
    .into();

    let candidates = candidate_moves_max(&engine_policy, &maia_policy, &config);

    // Should include both engine top moves and Maia top moves (deduped).
    let move_names: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();
    assert!(move_names.contains(&"e2e4"), "should include engine top move e2e4");
    assert!(move_names.contains(&"d2d4"), "should include engine top move d2d4");
    assert!(move_names.contains(&"g1f3"), "should include maia top move g1f3");

    // No duplicates.
    let mut sorted = move_names.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), move_names.len(), "candidate moves should be unique");

    // All priors should be positive.
    for (m, p) in &candidates {
        assert!(*p > 0.0, "move {m} has non-positive prior {p}");
    }
}

#[test]
fn candidate_moves_chance_filters_low_probability() {
    let config = Config::default(); // maia_min_prob = 0.001 (0.1%)

    let maia_policy: HashMap<String, f32> = [
        ("e7e5".into(), 40.0),
        ("c7c5".into(), 35.0),
        ("d7d5".into(), 20.0),
        ("a7a6".into(), 0.05), // 0.05% — below threshold
    ]
    .into();

    let candidates = candidate_moves_chance(&maia_policy, &config);
    let move_names: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();

    assert!(move_names.contains(&"e7e5"));
    assert!(move_names.contains(&"c7c5"));
    assert!(!move_names.contains(&"a7a6"), "low-prob move should be filtered");

    // Probabilities should sum to ~1 (normalized).
    let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
    assert!(
        (sum - 1.0).abs() < 0.05,
        "CHANCE priors should sum to ~1, got {sum}"
    );
}

// ── Root move info ───────────────────────────────────────────────────

#[test]
fn root_move_infos_match_children() {
    let (tree, config) = build_test_tree();
    let infos = root_move_infos(&tree, &config);
    let root = tree.root();

    // Should have one info per root child.
    assert_eq!(
        infos.len(),
        root.children.len(),
        "info count should match root children"
    );

    // Each info's visits should match the child node's visits.
    for info in &infos {
        let child = tree.get(info.node_id).unwrap();
        assert_eq!(info.visits, child.visit_count);
    }
}

#[test]
fn best_root_move_returns_highest_practical_q() {
    let (tree, config) = build_test_tree();
    let best = best_root_move(&tree, &config);
    let infos = root_move_infos(&tree, &config);

    if let Some(best) = best {
        // Best move should have the highest practical Q.
        for info in &infos {
            assert!(
                best.practical_q >= info.practical_q - 1e-9,
                "best move {} (pq={}) is worse than {} (pq={})",
                best.uci_move,
                best.practical_q,
                info.uci_move,
                info.practical_q
            );
        }
    }
}

// ── Position + Search integration ────────────────────────────────────

#[test]
fn position_epd_consistency_across_tree() {
    // Verify that EPDs stored in tree nodes match what PositionState produces.
    let (tree, _config) = build_test_tree();

    for node in &tree.nodes {
        if node.move_sequence.is_empty() {
            let pos = PositionState::startpos();
            assert_eq!(node.epd, pos.epd, "root EPD mismatch");
        } else {
            let pos = PositionState::from_moves(&node.move_sequence).unwrap();
            assert_eq!(
                node.epd, pos.epd,
                "EPD mismatch for move sequence '{}'",
                node.move_sequence
            );
        }
    }
}

#[test]
fn position_move_sequence_tracks_path() {
    let start = PositionState::startpos();
    let p1 = start.apply_uci("e2e4").unwrap();
    let p2 = p1.apply_uci("e7e5").unwrap();
    let p3 = p2.apply_uci("g1f3").unwrap();

    assert_eq!(p1.move_sequence, "e2e4");
    assert_eq!(p2.move_sequence, "e2e4 e7e5");
    assert_eq!(p3.move_sequence, "e2e4 e7e5 g1f3");
}
