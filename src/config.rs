use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// All tunable constants and engine paths for the application.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    // ── Engine paths ────────────────────────────────────────────────────
    pub lc0_path: String,
    pub engine_weights_path: String,
    pub maia_weights_path: String,

    // ── PUCT / Exploration ──────────────────────────────────────────────
    pub cpuct_init: f64,
    pub cpuct_base: f64,
    pub cpuct_factor: f64,
    pub fpu_reduction: f64,
    /// Prior blend: alpha * engine_policy + (1-alpha) * maia_policy
    pub alpha: f64,

    // ── Maia / Opponent modeling ────────────────────────────────────────
    pub maia_temperature: f64,
    pub maia_floor: f64,
    pub maia_min_prob: f64,

    // ── Evaluation ─────────────────────────────────────────────────────
    pub engine_nodes: u64,
    pub contempt: f64,

    // ── Final move selection ───────────────────────────────────────────
    pub safety: f64,

    // ── Search budget ──────────────────────────────────────────────────
    pub max_nodes: u64,

    // ── Candidate selection ────────────────────────────────────────────
    pub engine_top_n: usize,
    pub maia_top_n: usize,

    // ── lc0 process management ─────────────────────────────────────────
    pub nn_cache_size_mb: u32,
    pub ucinewgame_interval: u32,

    // ── Persistence ────────────────────────────────────────────────────
    pub flush_interval: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            lc0_path: String::new(),
            engine_weights_path: String::new(),
            maia_weights_path: String::new(),

            cpuct_init: 1.5,
            cpuct_base: 19652.0,
            cpuct_factor: 1.0,
            fpu_reduction: 0.3,
            alpha: 0.7,

            maia_temperature: 1.0,
            maia_floor: 0.01,
            maia_min_prob: 0.001,

            engine_nodes: 1,
            contempt: 0.6,

            safety: 0.2,

            max_nodes: 150_000,

            engine_top_n: 3,
            maia_top_n: 5,

            nn_cache_size_mb: 512,
            ucinewgame_interval: 500,

            flush_interval: 100,
        }
    }
}

impl Config {
    /// Returns true if all three engine paths are non-empty.
    pub fn engine_paths_configured(&self) -> bool {
        !self.lc0_path.is_empty()
            && !self.engine_weights_path.is_empty()
            && !self.maia_weights_path.is_empty()
    }

    /// Load config from the standard settings file, or return defaults.
    pub fn load() -> Self {
        let path = Self::config_path();
        match std::fs::read_to_string(&path) {
            Ok(contents) => toml::from_str(&contents).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    fn config_path() -> PathBuf {
        PathBuf::from("settings.toml")
    }
}

#[cfg(test)]
mod tests {
    use super::Config;

    // -- Default Values --

    #[test]
    fn defaults_match_documented_parameters() {
        let config = Config::default();
        assert!((config.cpuct_init - 1.5).abs() < 0.001);
        assert!((config.cpuct_base - 19652.0).abs() < 1.0);
        assert!((config.cpuct_factor - 1.0).abs() < 0.001);
        assert!((config.fpu_reduction - 0.3).abs() < 0.001);
        assert!((config.alpha - 0.7).abs() < 0.001);
        assert!((config.maia_temperature - 1.0).abs() < 0.001);
        assert!((config.maia_floor - 0.01).abs() < 0.001);
        assert!((config.maia_min_prob - 0.001).abs() < 0.0001);
        assert_eq!(config.engine_nodes, 1);
        assert!((config.contempt - 0.6).abs() < 0.001);
        assert!((config.safety - 0.2).abs() < 0.001);
        assert_eq!(config.max_nodes, 150_000);
        assert_eq!(config.engine_top_n, 3);
        assert_eq!(config.maia_top_n, 5);
        assert_eq!(config.nn_cache_size_mb, 512);
        assert_eq!(config.ucinewgame_interval, 500);
    }

    // -- Engine Path Validation --

    #[test]
    fn engine_paths_configured_returns_false_when_empty() {
        let config = Config::default();
        assert!(!config.engine_paths_configured());
    }

    #[test]
    fn engine_paths_configured_returns_true_when_all_set() {
        let mut config = Config::default();
        config.lc0_path = "/usr/bin/lc0".to_string();
        config.engine_weights_path = "/weights/net.pb".to_string();
        config.maia_weights_path = "/weights/maia.pb".to_string();
        assert!(config.engine_paths_configured());
    }

    #[test]
    fn engine_paths_configured_returns_false_with_partial_paths() {
        let mut config = Config::default();
        config.lc0_path = "/usr/bin/lc0".to_string();
        // engine_weights_path and maia_weights_path still empty
        assert!(!config.engine_paths_configured());
    }

    // -- Serialization --

    #[test]
    fn config_roundtrips_through_toml() {
        let config = Config::default();
        let toml_str = toml::to_string_pretty(&config).unwrap();
        let restored: Config = toml::from_str(&toml_str).unwrap();
        assert!((restored.cpuct_init - config.cpuct_init).abs() < 0.001);
        assert_eq!(restored.max_nodes, config.max_nodes);
        assert_eq!(restored.engine_top_n, config.engine_top_n);
    }

    #[test]
    fn config_deserializes_with_missing_fields_using_defaults() {
        let partial_toml = r#"
            lc0_path = "/usr/bin/lc0"
            max_nodes = 200000
        "#;
        let config: Config = toml::from_str(partial_toml).unwrap();
        assert_eq!(config.lc0_path, "/usr/bin/lc0");
        assert_eq!(config.max_nodes, 200_000);
        // Other fields should have defaults
        assert!((config.cpuct_init - 1.5).abs() < 0.001);
        assert_eq!(config.engine_top_n, 3);
    }
}
