use std::collections::HashMap;
use std::path::PathBuf;

use rusqlite::{Connection, params};

/// SQLite cache for engine evaluations and Maia predictions.
/// Engine evals are keyed by EPD (transposition-safe).
/// Maia predictions are keyed by full move sequence (history-dependent).
pub struct Cache {
    conn: Connection,
}

impl Cache {
    pub fn open() -> Result<Self, String> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache dir: {e}"))?;
        }

        let conn =
            Connection::open(&path).map_err(|e| format!("Failed to open cache DB: {e}"))?;

        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS engine_cache (
                epd TEXT NOT NULL,
                wdl_w INTEGER NOT NULL,
                wdl_d INTEGER NOT NULL,
                wdl_l INTEGER NOT NULL,
                policy_json TEXT NOT NULL,
                q_values_json TEXT NOT NULL,
                PRIMARY KEY (epd)
            );
            CREATE TABLE IF NOT EXISTS maia_cache (
                move_sequence TEXT NOT NULL,
                policy_json TEXT NOT NULL,
                PRIMARY KEY (move_sequence)
            );
            CREATE TABLE IF NOT EXISTS tree_nodes (
                id INTEGER PRIMARY KEY,
                parent_id INTEGER,
                move_uci TEXT,
                node_type TEXT NOT NULL,
                epd TEXT NOT NULL,
                move_sequence TEXT NOT NULL,
                visit_count INTEGER NOT NULL DEFAULT 0,
                total_value REAL NOT NULL DEFAULT 0.0,
                prior REAL NOT NULL DEFAULT 0.0,
                children_json TEXT,
                session_id TEXT NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_tree_session ON tree_nodes(session_id);
            ",
        )
        .map_err(|e| format!("Failed to create cache tables: {e}"))?;

        Ok(Self { conn })
    }

    // ── Engine cache (EPD-keyed) ────────────────────────────────────────

    pub fn get_engine_eval(
        &self,
        epd: &str,
    ) -> Option<(u32, u32, u32, HashMap<String, f32>, HashMap<String, f32>)> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT wdl_w, wdl_d, wdl_l, policy_json, q_values_json FROM engine_cache WHERE epd = ?1",
            )
            .ok()?;

        stmt.query_row(params![epd], |row| {
            let w: u32 = row.get(0)?;
            let d: u32 = row.get(1)?;
            let l: u32 = row.get(2)?;
            let policy_json: String = row.get(3)?;
            let q_json: String = row.get(4)?;
            let policy: HashMap<String, f32> =
                serde_json::from_str(&policy_json).unwrap_or_default();
            let q_values: HashMap<String, f32> =
                serde_json::from_str(&q_json).unwrap_or_default();
            Ok((w, d, l, policy, q_values))
        })
        .ok()
    }

    pub fn put_engine_eval(
        &self,
        epd: &str,
        wdl: (u32, u32, u32),
        policy: &HashMap<String, f32>,
        q_values: &HashMap<String, f32>,
    ) -> Result<(), String> {
        let policy_json =
            serde_json::to_string(policy).map_err(|e| format!("JSON serialize error: {e}"))?;
        let q_json =
            serde_json::to_string(q_values).map_err(|e| format!("JSON serialize error: {e}"))?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO engine_cache (epd, wdl_w, wdl_d, wdl_l, policy_json, q_values_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![epd, wdl.0, wdl.1, wdl.2, policy_json, q_json],
            )
            .map_err(|e| format!("Failed to cache engine eval: {e}"))?;
        Ok(())
    }

    // ── Maia cache (move-sequence-keyed) ────────────────────────────────

    pub fn get_maia_policy(&self, move_sequence: &str) -> Option<HashMap<String, f32>> {
        let mut stmt = self
            .conn
            .prepare("SELECT policy_json FROM maia_cache WHERE move_sequence = ?1")
            .ok()?;

        stmt.query_row(params![move_sequence], |row| {
            let json: String = row.get(0)?;
            Ok(serde_json::from_str(&json).unwrap_or_default())
        })
        .ok()
    }

    pub fn put_maia_policy(
        &self,
        move_sequence: &str,
        policy: &HashMap<String, f32>,
    ) -> Result<(), String> {
        let json =
            serde_json::to_string(policy).map_err(|e| format!("JSON serialize error: {e}"))?;

        self.conn
            .execute(
                "INSERT OR REPLACE INTO maia_cache (move_sequence, policy_json) VALUES (?1, ?2)",
                params![move_sequence, json],
            )
            .map_err(|e| format!("Failed to cache Maia policy: {e}"))?;
        Ok(())
    }

    // ── Tree persistence ────────────────────────────────────────────────

    pub fn save_tree_node(
        &self,
        id: u64,
        parent_id: Option<u64>,
        move_uci: Option<&str>,
        node_type: &str,
        epd: &str,
        move_sequence: &str,
        visit_count: u64,
        total_value: f64,
        prior: f64,
        children_json: Option<&str>,
        session_id: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT OR REPLACE INTO tree_nodes (id, parent_id, move_uci, node_type, epd, move_sequence, visit_count, total_value, prior, children_json, session_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                params![
                    id as i64,
                    parent_id.map(|p| p as i64),
                    move_uci,
                    node_type,
                    epd,
                    move_sequence,
                    visit_count as i64,
                    total_value,
                    prior,
                    children_json,
                    session_id,
                ],
            )
            .map_err(|e| format!("Failed to save tree node: {e}"))?;
        Ok(())
    }

    pub fn clear_tree(&self, session_id: &str) -> Result<(), String> {
        self.conn
            .execute(
                "DELETE FROM tree_nodes WHERE session_id = ?1",
                params![session_id],
            )
            .map_err(|e| format!("Failed to clear tree: {e}"))?;
        Ok(())
    }

    fn db_path() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("chess-meta")
            .join("cache.db")
    }
}
