use std::collections::HashMap;

use criterion::{Criterion, black_box, criterion_group, criterion_main};

use chess_meta::cache::Cache;

fn make_policy(n: u32) -> HashMap<String, f32> {
    (0..n).map(|i| (format!("move_{i}"), (n - i) as f32)).collect()
}

fn bench_cache_engine_write(c: &mut Criterion) {
    let cache = Cache::open_in_memory().unwrap();
    let policy = make_policy(20);
    let q_values = make_policy(10);
    c.bench_function("cache_engine_write", |b| {
        b.iter(|| {
            cache
                .put_engine_eval(
                    black_box("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -"),
                    black_box((400u32, 450u32, 150u32)),
                    black_box(&policy),
                    black_box(&q_values),
                )
                .unwrap()
        })
    });
}

fn bench_cache_engine_read_hit(c: &mut Criterion) {
    let cache = Cache::open_in_memory().unwrap();
    let epd = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -";
    let policy = make_policy(20);
    let q_values = make_policy(10);
    cache.put_engine_eval(epd, (400, 450, 150), &policy, &q_values).unwrap();
    c.bench_function("cache_engine_read_hit", |b| {
        b.iter(|| cache.get_engine_eval(black_box(epd)))
    });
}

fn bench_cache_maia_write(c: &mut Criterion) {
    let cache = Cache::open_in_memory().unwrap();
    let policy = make_policy(30);
    c.bench_function("cache_maia_write", |b| {
        b.iter(|| {
            cache
                .put_maia_policy(black_box("e2e4 e7e5 g1f3"), black_box(&policy))
                .unwrap()
        })
    });
}

fn bench_cache_maia_read_hit(c: &mut Criterion) {
    let cache = Cache::open_in_memory().unwrap();
    let move_seq = "e2e4 e7e5 g1f3";
    let policy = make_policy(30);
    cache.put_maia_policy(move_seq, &policy).unwrap();
    c.bench_function("cache_maia_read_hit", |b| {
        b.iter(|| cache.get_maia_policy(black_box(move_seq)))
    });
}

criterion_group!(
    benches,
    bench_cache_engine_write,
    bench_cache_engine_read_hit,
    bench_cache_maia_write,
    bench_cache_maia_read_hit,
);
criterion_main!(benches);
