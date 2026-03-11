use std::collections::HashMap;

use criterion::{Criterion, criterion_group, criterion_main};

use chess_meta::search::{
    NodeId, NodeType, SearchState, SearchTree, backpropagate, candidate_moves_max, select,
};
use chess_meta::config::Config;

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

fn bench_puct_selection(c: &mut Criterion) {
    let tree = build_test_tree(1000);
    let config = Config::default();
    let mut state = SearchState::new();

    c.bench_function("puct_selection_1k_nodes", |b| {
        b.iter(|| select(&tree, &config, &mut state))
    });
}

fn bench_backpropagation(c: &mut Criterion) {
    let mut tree = build_test_tree(1000);
    let config = Config::default();
    let mut state = SearchState::new();

    // Find a leaf to backprop from
    let leaf = select(&tree, &config, &mut state);

    c.bench_function("backprop_1k_nodes", |b| {
        b.iter(|| backpropagate(&mut tree, leaf, 0.55))
    });
}

fn bench_candidate_moves_max(c: &mut Criterion) {
    let mut engine_policy = HashMap::new();
    let mut maia_policy = HashMap::new();
    for i in 0..30 {
        engine_policy.insert(format!("move_{i}"), (30 - i) as f32);
        maia_policy.insert(format!("move_{i}"), (i + 1) as f32);
    }
    let config = Config::default();

    c.bench_function("candidate_moves_max_30", |b| {
        b.iter(|| candidate_moves_max(&engine_policy, &maia_policy, &config))
    });
}

criterion_group!(
    benches,
    bench_puct_selection,
    bench_backpropagation,
    bench_candidate_moves_max,
);
criterion_main!(benches);
