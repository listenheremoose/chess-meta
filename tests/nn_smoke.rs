//! Smoke test: loads the ONNX models and evaluates the starting position.
use chess_meta::config::Config;
use chess_meta::nn::NNEvaluator;

#[test]
fn onnx_engine_startpos() {
    let config = Config::load();
    if !config.onnx_configured() {
        eprintln!("ONNX paths not configured, skipping");
        return;
    }
    let mut eval = NNEvaluator::new(&config.engine_onnx_path, &config.maia_onnx_path).unwrap();
    let result = eval.evaluate_engine("").unwrap();
    let (w, d, l) = result.wdl;
    eprintln!("WDL: ({w}, {d}, {l})");
    assert!(w + d + l > 0, "WDL should be non-zero");
    assert!(!result.policy.is_empty(), "policy should have moves");
    let top: Vec<_> = result.top_policy_moves(5);
    eprintln!("Top 5 engine policy: {:?}", top);
}

#[test]
fn onnx_maia_startpos() {
    let config = Config::load();
    if !config.onnx_configured() {
        eprintln!("ONNX paths not configured, skipping");
        return;
    }
    let mut eval = NNEvaluator::new(&config.engine_onnx_path, &config.maia_onnx_path).unwrap();
    let policy = eval.evaluate_maia("").unwrap();
    assert!(!policy.is_empty(), "Maia policy should have moves");
    let mut moves: Vec<_> = policy.iter().collect();
    moves.sort_by(|a, b| b.1.partial_cmp(a.1).unwrap());
    eprintln!("Top 5 Maia policy: {:?}", &moves[..5.min(moves.len())]);
}
