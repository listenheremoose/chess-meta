use std::collections::{HashMap, HashSet};

use crate::config::Config;
use crate::engine::lookup_castling_aware;

/// Determine candidate moves at a MAX node.
/// Returns (uci_move, blended_prior) pairs using top engine + top Maia moves.
pub fn candidate_moves_max(
    engine_policy: &HashMap<String, f32>,
    maia_policy: &HashMap<String, f32>,
    config: &Config,
) -> Vec<(String, f64)> {
    let engine_top = top_n_by_policy(engine_policy, config.engine_top_n);
    let maia_top = top_n_by_policy(maia_policy, config.maia_top_n);

    let mut seen = HashSet::new();
    let mut candidates: Vec<(String, f64)> = engine_top
        .iter()
        .chain(maia_top.iter())
        .filter(|uci| seen.insert(uci.to_string()))
        .map(|uci| {
            let engine_p = lookup_castling_aware(uci, engine_policy).unwrap_or(0.0) as f64 / 100.0;
            let maia_p = lookup_castling_aware(uci, maia_policy).unwrap_or(0.0) as f64 / 100.0;
            let blended = config.alpha * engine_p + (1.0 - config.alpha) * maia_p;
            ((*uci).clone(), blended)
        })
        .collect();

    normalize_priors(&mut candidates);
    candidates
}

/// Determine candidate moves at a CHANCE node from Maia policy.
/// Returns (uci_move, maia_probability) pairs, filtered by min_prob.
pub fn candidate_moves_chance(
    maia_policy: &HashMap<String, f32>,
    config: &Config,
) -> Vec<(String, f64)> {
    let mut candidates: Vec<(String, f64)> = maia_policy
        .iter()
        .filter(|(_, p)| (**p as f64 / 100.0) >= config.maia_min_prob)
        .map(|(m, p)| (m.clone(), *p as f64 / 100.0))
        .collect();

    normalize_priors(&mut candidates);
    candidates
}

fn top_n_by_policy(policy: &HashMap<String, f32>, n: usize) -> Vec<&String> {
    let mut sorted: Vec<_> = policy.iter().collect();
    sorted.sort_by(|a, b| match b.1.partial_cmp(a.1) {
        Some(ord) => ord,
        None => std::cmp::Ordering::Equal,
    });
    sorted.iter().take(n).map(|(m, _)| *m).collect()
}

fn normalize_priors(candidates: &mut Vec<(String, f64)>) {
    let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
    if sum > 0.0 {
        candidates.iter_mut().for_each(|(_, p)| *p /= sum);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::config::Config;

    use super::{candidate_moves_max, candidate_moves_chance};

    #[test]
    fn max_candidates_deduplicates_engine_and_maia() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 50.0f32);
        engine_policy.insert("d2d4".to_string(), 30.0);
        engine_policy.insert("g1f3".to_string(), 10.0);

        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 40.0f32);
        maia.insert("d2d4".to_string(), 35.0);
        maia.insert("b1c3".to_string(), 10.0);
        maia.insert("g1f3".to_string(), 8.0);
        maia.insert("c2c4".to_string(), 4.0);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        assert_eq!(candidates.len(), 5);
        let moves: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();
        assert!(moves.contains(&"e2e4"));
        assert!(moves.contains(&"b1c3"));
        assert!(moves.contains(&"c2c4"));
    }

    #[test]
    fn max_candidates_priors_sum_to_one() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 50.0f32);
        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 80.0f32);
        maia.insert("d2d4".to_string(), 20.0);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < 0.001);
    }

    #[test]
    fn max_candidates_blends_engine_and_maia_priors() {
        let mut engine_policy = HashMap::new();
        engine_policy.insert("e2e4".to_string(), 100.0f32);
        let mut maia = HashMap::new();
        maia.insert("e2e4".to_string(), 100.0f32);

        let config = Config::default();
        let candidates = candidate_moves_max(&engine_policy, &maia, &config);

        assert_eq!(candidates.len(), 1);
        assert!((candidates[0].1 - 1.0).abs() < 0.001);
    }

    #[test]
    fn chance_candidates_filters_below_min_prob() {
        let mut maia = HashMap::new();
        maia.insert("e7e5".to_string(), 40.0f32);
        maia.insert("c7c5".to_string(), 20.0);
        maia.insert("e7e6".to_string(), 15.0);
        maia.insert("a7a6".to_string(), 0.005);

        let config = Config::default();
        let candidates = candidate_moves_chance(&maia, &config);

        assert_eq!(candidates.len(), 3);
        let moves: Vec<&str> = candidates.iter().map(|(m, _)| m.as_str()).collect();
        assert!(!moves.contains(&"a7a6"));
    }

    #[test]
    fn chance_candidates_normalizes_to_one() {
        let mut maia = HashMap::new();
        maia.insert("e7e5".to_string(), 60.0f32);
        maia.insert("d7d5".to_string(), 30.0);

        let config = Config::default();
        let candidates = candidate_moves_chance(&maia, &config);

        let sum: f64 = candidates.iter().map(|(_, p)| p).sum();
        assert!((sum - 1.0).abs() < 0.001);
    }
}
