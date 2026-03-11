use criterion::{Criterion, black_box, criterion_group, criterion_main};

use chess_meta::engine::{format_position_cmd, parse_verbose_move_stats};

fn bench_parse_verbose_move_stats(c: &mut Criterion) {
    let line = "info string d2d4  (293 ) N:    7934 (+18) (P: 12.71%) (WL:  0.05704) (D: 0.745) (M: 197.1) (Q:  0.05704) (U: 0.00749) (S:  0.06484) (V:  0.0303)";
    c.bench_function("parse_verbose_move_stats", |b| {
        b.iter(|| parse_verbose_move_stats(black_box(line)))
    });
}

fn bench_parse_verbose_stats_no_q(c: &mut Criterion) {
    let line = "info string e2e4  (0  ) N:       0 (+ 0) (P: 45.20%)";
    c.bench_function("parse_verbose_stats_no_q", |b| {
        b.iter(|| parse_verbose_move_stats(black_box(line)))
    });
}

fn bench_format_position_cmd(c: &mut Criterion) {
    let moves = "e2e4 e7e5 g1f3 b8c6 f1b5";
    c.bench_function("format_position_cmd", |b| {
        b.iter(|| format_position_cmd(black_box(moves)))
    });
}

criterion_group!(
    benches,
    bench_parse_verbose_move_stats,
    bench_parse_verbose_stats_no_q,
    bench_format_position_cmd,
);
criterion_main!(benches);
