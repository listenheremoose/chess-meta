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
    pub max_iterations: u64,

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

            max_iterations: 5000,

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

    /// Save config to the standard settings file.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create config dir: {e}"))?;
        }
        let contents =
            toml::to_string_pretty(self).map_err(|e| format!("Failed to serialize config: {e}"))?;
        std::fs::write(&path, contents).map_err(|e| format!("Failed to write config: {e}"))?;
        Ok(())
    }

    fn config_path() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chess-meta")
            .join("settings.toml")
    }
}
