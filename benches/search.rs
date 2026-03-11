use std::collections::HashMap;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use chess_meta::config::Config;
use chess_meta::engine::EngineEval;
use chess_meta::search::{
    NodeId, NodeType, SearchState, SearchTree, backpropagate, candidate_moves_chance,
    candidate_moves_max, select,
};

// --- Tree builders ---

/// Shallow tree: root with 8 children, each visited 100 times.
fn shallow_tree() -> SearchTree {
    let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
    for i in 0..8u32 {
        let id = tree.add_child(
            NodeId(0),
            format!("move_{i}"),
            NodeType::Chance,
            format!("epd_{i}"),
            format!("move_{i}"),
            0.125,
        );
        let node = tree.get_mut(id).unwrap();
        node.visit_count = 100;
        node.total_value = 50.0;
    }
    let root = tree.get_mut(NodeId(0)).unwrap();
    root.expanded = true;
    root.visit_count = 800;
    root.total_value = 400.0;
    tree
}

/// Deep tree: linear path of 20 nodes, alternating Max/Chance.
/// All intermediate nodes are expanded; the leaf is not.
fn deep_tree() -> SearchTree {
    let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
    let mut parent = NodeId(0);
    let mut move_seq = String::new();
    for i in 0..19u32 {
        let child_type = if i % 2 == 0 { NodeType::Chance } else { NodeType::Max };
        if !move_seq.is_empty() {
            move_seq.push(' ');
        }
        move_seq.push_str(&format!("m{i}"));
        let id = tree.add_child(
            parent,
            format!("m{i}"),
            child_type,
            format!("epd_{i}"),
            move_seq.clone(),
            1.0,
        );
        let child = tree.get_mut(id).unwrap();
        child.visit_count = 10;
        child.total_value = 5.0;
        let p = tree.get_mut(parent).unwrap();
        p.expanded = true;
        p.visit_count = 10;
        p.total_value = 5.0;
        parent = id;
    }
    tree
}

/// Wide tree: root with 30 children, varying visit counts.
fn wide_tree() -> SearchTree {
    let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
    let total = 30u32;
    for i in 0..total {
        let id = tree.add_child(
            NodeId(0),
            format!("move_{i}"),
            NodeType::Chance,
            format!("epd_{i}"),
            format!("move_{i}"),
            1.0 / total as f64,
        );
        let visits = (i + 1) as u64 * 10;
        let node = tree.get_mut(id).unwrap();
        node.visit_count = visits;
        node.total_value = visits as f64 * 0.5;
    }
    let root = tree.get_mut(NodeId(0)).unwrap();
    root.expanded = true;
    root.visit_count = 4650; // sum 10+20+...+300
    root.total_value = 4650.0 * 0.5;
    tree
}

/// Build a test tree with `n` nodes for benchmarking.
fn build_test_tree(n: u32) -> SearchTree {
    let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);

    // Expand root with some children
    let root_children: Vec<NodeId> = (0..5)
        .map(|i| {
            tree.add_child(
                NodeId(0),
                format!("move_{i}"),
                NodeType::Chance,
                format!("epd_{i}"),
                format!("move_{i}"),
                0.2,
            )
        })
        .collect();
    tree.get_mut(NodeId(0)).unwrap().expanded = true;
    tree.get_mut(NodeId(0)).unwrap().visit_count = n as u64;
    tree.get_mut(NodeId(0)).unwrap().total_value = n as f64 * 0.5;

    // Add children to each root child
    let mut next_parent_batch = root_children.clone();
    let mut created = 6u32; // root + 5 children

    while created < n {
        let mut new_batch = Vec::new();
        for &parent_id in &next_parent_batch {
            if created >= n {
                break;
            }
            let parent_type = tree.get(parent_id).unwrap().node_type;
            let child_type = match parent_type {
                NodeType::Max => NodeType::Chance,
                NodeType::Chance => NodeType::Max,
            };

            let child_count = 3.min(n - created);
            for j in 0..child_count {
                let child_id = tree.add_child(
                    parent_id,
                    format!("m_{created}_{j}"),
                    child_type,
                    format!("epd_{created}_{j}"),
                    format!("seq_{created}_{j}"),
                    1.0 / child_count as f64,
                );
                let node = tree.get_mut(child_id).unwrap();
                node.visit_count = 10;
                node.total_value = 5.0;
                new_batch.push(child_id);
                created += 1;
            }
            tree.get_mut(parent_id).unwrap().expanded = true;
            tree.get_mut(parent_id).unwrap().visit_count = 30;
            tree.get_mut(parent_id).unwrap().total_value = 15.0;
        }
        next_parent_batch = new_batch;
    }

    tree
}

/// Build a linear chain of `depth` nodes and return the leaf id.
fn linear_chain(depth: u32) -> (SearchTree, NodeId) {
    let mut tree = SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
    let mut parent = NodeId(0);
    let mut move_seq = String::new();
    for i in 0..depth {
        let child_type = if i % 2 == 0 { NodeType::Chance } else { NodeType::Max };
        if !move_seq.is_empty() {
            move_seq.push(' ');
        }
        move_seq.push_str(&format!("m{i}"));
        let id = tree.add_child(
            parent,
            format!("m{i}"),
            child_type,
            format!("epd_{i}"),
            move_seq.clone(),
            1.0,
        );
        let child = tree.get_mut(id).unwrap();
        child.visit_count = 5;
        child.total_value = 2.5;
        let p = tree.get_mut(parent).unwrap();
        p.expanded = true;
        p.visit_count = 5;
        p.total_value = 2.5;
        parent = id;
    }
    (tree, parent)
}

// --- Selection benchmarks ---

fn bench_puct_selection_shallow_tree(c: &mut Criterion) {
    let tree = shallow_tree();
    let config = Config::default();
    let mut state = SearchState::new();
    c.bench_function("puct_selection_shallow_tree", |b| {
        b.iter(|| select(black_box(&tree), black_box(&config), &mut state))
    });
}

fn bench_puct_selection_deep_tree(c: &mut Criterion) {
    let tree = deep_tree();
    let config = Config::default();
    let mut state = SearchState::new();
    c.bench_function("puct_selection_deep_tree", |b| {
        b.iter(|| select(black_box(&tree), black_box(&config), &mut state))
    });
}

fn bench_puct_selection_wide_tree(c: &mut Criterion) {
    let tree = wide_tree();
    let config = Config::default();
    let mut state = SearchState::new();
    c.bench_function("puct_selection_wide_tree", |b| {
        b.iter(|| select(black_box(&tree), black_box(&config), &mut state))
    });
}

fn bench_puct_selection_1k_nodes(c: &mut Criterion) {
    let tree = build_test_tree(1000);
    let config = Config::default();
    let mut state = SearchState::new();
    c.bench_function("puct_selection_1k_nodes", |b| {
        b.iter(|| select(black_box(&tree), black_box(&config), &mut state))
    });
}

// --- Backpropagation benchmarks ---

fn bench_backprop_short_path(c: &mut Criterion) {
    let (mut tree, leaf) = linear_chain(5);
    c.bench_function("backprop_short_path", |b| {
        b.iter(|| backpropagate(&mut tree, black_box(leaf), black_box(0.55)))
    });
}

fn bench_backprop_long_path(c: &mut Criterion) {
    let (mut tree, leaf) = linear_chain(20);
    c.bench_function("backprop_long_path", |b| {
        b.iter(|| backpropagate(&mut tree, black_box(leaf), black_box(0.55)))
    });
}

fn bench_backprop_1k_nodes(c: &mut Criterion) {
    let mut tree = build_test_tree(1000);
    let config = Config::default();
    let mut state = SearchState::new();
    let leaf = select(&tree, &config, &mut state);
    c.bench_function("backprop_1k_nodes", |b| {
        b.iter(|| backpropagate(&mut tree, black_box(leaf), black_box(0.55)))
    });
}

// --- Candidate move filtering ---

fn bench_candidate_moves_max_30(c: &mut Criterion) {
    let mut engine_policy = HashMap::new();
    let mut maia_policy = HashMap::new();
    for i in 0..30 {
        engine_policy.insert(format!("move_{i}"), (30 - i) as f32);
        maia_policy.insert(format!("move_{i}"), (i + 1) as f32);
    }
    let config = Config::default();
    c.bench_function("candidate_moves_max_30", |b| {
        b.iter(|| candidate_moves_max(black_box(&engine_policy), black_box(&maia_policy), black_box(&config)))
    });
}

fn bench_candidate_moves_chance_20(c: &mut Criterion) {
    let mut maia_policy = HashMap::new();
    for i in 0..20u32 {
        maia_policy.insert(format!("move_{i}"), (i + 1) as f32);
    }
    let config = Config::default();
    c.bench_function("candidate_moves_chance_20", |b| {
        b.iter(|| candidate_moves_chance(black_box(&maia_policy), black_box(&config)))
    });
}

// --- Tree node creation ---

fn bench_tree_node_creation(c: &mut Criterion) {
    c.bench_function("tree_node_creation_100", |b| {
        b.iter(|| {
            let mut tree =
                SearchTree::new("startpos".to_string(), String::new(), NodeType::Max);
            for i in 0..100u32 {
                tree.add_child(
                    NodeId(0),
                    format!("m{i}"),
                    NodeType::Chance,
                    format!("e{i}"),
                    format!("m{i}"),
                    0.01,
                );
            }
            black_box(tree)
        })
    });
}

// --- Value conversion ---

fn bench_value_white(c: &mut Criterion) {
    let eval = EngineEval {
        wdl: (400, 450, 150),
        policy: HashMap::new(),
        q_values: HashMap::new(),
    };
    c.bench_function("value_white_conversion", |b| {
        b.iter(|| black_box(eval.value_white(black_box(0.6), black_box(true))))
    });
}

criterion_group!(
    benches,
    bench_puct_selection_shallow_tree,
    bench_puct_selection_deep_tree,
    bench_puct_selection_wide_tree,
    bench_puct_selection_1k_nodes,
    bench_backprop_short_path,
    bench_backprop_long_path,
    bench_backprop_1k_nodes,
    bench_candidate_moves_max_30,
    bench_candidate_moves_chance_20,
    bench_tree_node_creation,
    bench_value_white,
);
criterion_main!(benches);
