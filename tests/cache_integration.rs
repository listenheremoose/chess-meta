use std::collections::HashMap;

use chess_meta::cache::Cache;

/// Helper: build a simple policy map.
fn sample_policy() -> HashMap<String, f32> {
    [("e2e4", 45.2), ("d2d4", 30.1), ("g1f3", 12.5)]
        .into_iter()
        .map(|(m, p)| (m.to_string(), p))
        .collect()
}

fn sample_q_values() -> HashMap<String, f32> {
    [("e2e4", 0.55), ("d2d4", 0.52), ("g1f3", 0.48)]
        .into_iter()
        .map(|(m, q)| (m.to_string(), q))
        .collect()
}

// ── Engine cache ─────────────────────────────────────────────────────

#[test]
fn engine_cache_miss_returns_none() {
    let cache = Cache::open_in_memory().unwrap();
    assert!(cache.get_engine_eval("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -").is_none());
}

#[test]
fn engine_cache_roundtrip() {
    let cache = Cache::open_in_memory().unwrap();
    let epd = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -";
    let wdl = (350, 400, 250);
    let policy = sample_policy();
    let q_values = sample_q_values();

    cache.put_engine_eval(epd, wdl, &policy, &q_values).unwrap();

    let (w, d, l, got_policy, got_q) = cache.get_engine_eval(epd).unwrap();
    assert_eq!((w, d, l), wdl);
    assert_eq!(got_policy.len(), policy.len());
    for (m, p) in &policy {
        assert!((got_policy[m] - p).abs() < 1e-6, "policy mismatch for {m}");
    }
    for (m, q) in &q_values {
        assert!((got_q[m] - q).abs() < 1e-6, "q mismatch for {m}");
    }
}

#[test]
fn engine_cache_epd_deduplication() {
    // Same EPD reached via different move orders should share cache entry.
    let cache = Cache::open_in_memory().unwrap();
    let epd = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -";

    let policy = sample_policy();
    let q_values = sample_q_values();
    cache.put_engine_eval(epd, (400, 300, 300), &policy, &q_values).unwrap();

    // A second lookup with the same EPD (regardless of how we got there) hits cache.
    assert!(cache.get_engine_eval(epd).is_some());
}

#[test]
fn engine_cache_overwrite_updates_values() {
    let cache = Cache::open_in_memory().unwrap();
    let epd = "test/epd";
    let policy = sample_policy();
    let q = sample_q_values();

    cache.put_engine_eval(epd, (100, 200, 700), &policy, &q).unwrap();
    cache.put_engine_eval(epd, (500, 300, 200), &policy, &q).unwrap();

    let (w, d, l, _, _) = cache.get_engine_eval(epd).unwrap();
    assert_eq!((w, d, l), (500, 300, 200));
}

// ── Maia cache ───────────────────────────────────────────────────────

#[test]
fn maia_cache_miss_returns_none() {
    let cache = Cache::open_in_memory().unwrap();
    assert!(cache.get_maia_policy("e2e4 e7e5").is_none());
}

#[test]
fn maia_cache_roundtrip() {
    let cache = Cache::open_in_memory().unwrap();
    let seq = "e2e4 e7e5 g1f3";
    let policy = sample_policy();

    cache.put_maia_policy(seq, &policy).unwrap();
    let got = cache.get_maia_policy(seq).unwrap();

    assert_eq!(got.len(), policy.len());
    for (m, p) in &policy {
        assert!((got[m] - p).abs() < 1e-6);
    }
}

#[test]
fn maia_cache_different_move_sequences_are_separate() {
    // Maia is keyed by move sequence, not EPD — different sequences should be distinct.
    let cache = Cache::open_in_memory().unwrap();

    let policy_a: HashMap<String, f32> = [("d7d5".to_string(), 60.0)].into();
    let policy_b: HashMap<String, f32> = [("c7c5".to_string(), 40.0)].into();

    cache.put_maia_policy("e2e4", &policy_a).unwrap();
    cache.put_maia_policy("d2d4", &policy_b).unwrap();

    let got_a = cache.get_maia_policy("e2e4").unwrap();
    let got_b = cache.get_maia_policy("d2d4").unwrap();

    assert!(got_a.contains_key("d7d5"));
    assert!(!got_a.contains_key("c7c5"));
    assert!(got_b.contains_key("c7c5"));
    assert!(!got_b.contains_key("d7d5"));
}

// ── Tree persistence ─────────────────────────────────────────────────

use chess_meta::search::{NodeType, SearchTree};

#[test]
fn tree_save_load_roundtrip() {
    let cache = Cache::open_in_memory().unwrap();

    let mut tree = SearchTree::new("startpos-epd".into(), "".into(), NodeType::Max);
    let root_id = tree.root_id;
    tree.add_child(root_id, "e2e4".into(), NodeType::Chance, "after-e4-epd".into(), "e2e4".into(), 0.45);
    tree.add_child(root_id, "d2d4".into(), NodeType::Chance, "after-d4-epd".into(), "d2d4".into(), 0.35);

    cache.save_tree(&tree, "session-1").unwrap();

    let loaded = cache.load_tree("session-1").unwrap();
    assert_eq!(loaded.node_count(), tree.node_count());
    assert_eq!(loaded.root().epd, "startpos-epd");
    assert_eq!(loaded.root().children.len(), 2);
}

#[test]
fn tree_save_and_clear() {
    let cache = Cache::open_in_memory().unwrap();

    let tree = SearchTree::new("startpos-epd".into(), "".into(), NodeType::Max);
    cache.save_tree(&tree, "session-1").unwrap();

    cache.clear_tree("session-1").unwrap();
    assert!(cache.load_tree("session-1").is_none());

    // Clearing a non-existent session is also fine.
    cache.clear_tree("no-such-session").unwrap();
}

#[test]
fn tree_save_overwrites_previous() {
    let cache = Cache::open_in_memory().unwrap();

    let tree1 = SearchTree::new("epd-v1".into(), "".into(), NodeType::Max);
    cache.save_tree(&tree1, "s1").unwrap();

    let mut tree2 = SearchTree::new("epd-v2".into(), "".into(), NodeType::Max);
    let root_id = tree2.root_id;
    tree2.add_child(root_id, "e2e4".into(), NodeType::Chance, "child-epd".into(), "e2e4".into(), 0.5);
    cache.save_tree(&tree2, "s1").unwrap();

    let loaded = cache.load_tree("s1").unwrap();
    assert_eq!(loaded.root().epd, "epd-v2");
    assert_eq!(loaded.node_count(), 2);
}

// ── Cross-module: cache + position EPD keys ──────────────────────────

#[test]
fn position_epd_used_as_cache_key() {
    use chess_meta::position::PositionState;

    let cache = Cache::open_in_memory().unwrap();
    let pos = PositionState::from_moves("e2e4 e7e5").unwrap();

    let policy = sample_policy();
    let q = sample_q_values();
    cache.put_engine_eval(&pos.epd, (400, 350, 250), &policy, &q).unwrap();

    // Same position reached — should hit.
    let hit = cache.get_engine_eval(&pos.epd);
    assert!(hit.is_some());
}

#[test]
fn different_positions_have_different_epds() {
    use chess_meta::position::PositionState;

    let cache = Cache::open_in_memory().unwrap();
    let pos_a = PositionState::from_moves("e2e4").unwrap();
    let pos_b = PositionState::from_moves("d2d4").unwrap();

    let policy = sample_policy();
    let q = sample_q_values();
    cache.put_engine_eval(&pos_a.epd, (400, 350, 250), &policy, &q).unwrap();

    // Different position should miss.
    assert!(cache.get_engine_eval(&pos_b.epd).is_none());
}
