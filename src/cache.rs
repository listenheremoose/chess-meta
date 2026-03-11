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
    /// Open an in-memory SQLite database (for testing).
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory()
            .map_err(|e| format!("Failed to open in-memory DB: {e}"))?;
        Self::init_tables(&conn)?;
        Ok(Self { conn })
    }

    fn init_tables(conn: &Connection) -> Result<(), String> {
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
        Ok(())
    }

    pub fn open() -> Result<Self, String> {
        let path = Self::db_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create cache dir: {e}"))?;
        }

        let conn =
            Connection::open(&path).map_err(|e| format!("Failed to open cache DB: {e}"))?;

        Self::init_tables(&conn)?;

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Open an in-memory SQLite database for testing.
    fn test_cache() -> Cache {
        let conn = Connection::open_in_memory().unwrap();
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
            ",
        )
        .unwrap();
        Cache { conn }
    }

    // -- Engine Cache --

    #[test]
    fn engine_cache_miss_returns_none() {
        let cache = test_cache();
        assert!(cache.get_engine_eval("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq -").is_none());
    }

    #[test]
    fn engine_cache_roundtrips_eval() {
        let cache = test_cache();
        let epd = "rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq -";
        let mut policy = HashMap::new();
        policy.insert("e7e5".to_string(), 35.0f32);
        policy.insert("c7c5".to_string(), 20.0);
        let mut q_values = HashMap::new();
        q_values.insert("e7e5".to_string(), -0.05f32);

        cache.put_engine_eval(epd, (400, 450, 150), &policy, &q_values).unwrap();

        let (w, d, l, cached_policy, cached_q) = cache.get_engine_eval(epd).unwrap();
        assert_eq!((w, d, l), (400, 450, 150));
        assert!((cached_policy["e7e5"] - 35.0).abs() < 0.01);
        assert!((cached_q["e7e5"] - (-0.05)).abs() < 0.001);
    }

    #[test]
    fn engine_cache_overwrites_on_duplicate_epd() {
        let cache = test_cache();
        let epd = "test_epd";
        let policy = HashMap::new();
        let q = HashMap::new();

        cache.put_engine_eval(epd, (100, 800, 100), &policy, &q).unwrap();
        cache.put_engine_eval(epd, (300, 400, 300), &policy, &q).unwrap();

        let (w, d, l, _, _) = cache.get_engine_eval(epd).unwrap();
        assert_eq!((w, d, l), (300, 400, 300));
    }

    // -- Maia Cache --

    #[test]
    fn maia_cache_miss_returns_none() {
        let cache = test_cache();
        assert!(cache.get_maia_policy("e2e4 e7e5").is_none());
    }

    #[test]
    fn maia_cache_roundtrips_policy() {
        let cache = test_cache();
        let move_seq = "e2e4 e7e5 g1f3";
        let mut policy = HashMap::new();
        policy.insert("b8c6".to_string(), 45.0f32);
        policy.insert("d7d6".to_string(), 25.0);

        cache.put_maia_policy(move_seq, &policy).unwrap();

        let cached = cache.get_maia_policy(move_seq).unwrap();
        assert!((cached["b8c6"] - 45.0).abs() < 0.01);
        assert!((cached["d7d6"] - 25.0).abs() < 0.01);
    }

    #[test]
    fn maia_cache_different_move_orders_are_separate() {
        let cache = test_cache();
        let mut policy_a = HashMap::new();
        policy_a.insert("move_a".to_string(), 50.0f32);
        let mut policy_b = HashMap::new();
        policy_b.insert("move_b".to_string(), 60.0f32);

        cache.put_maia_policy("e2e4 d7d5", &policy_a).unwrap();
        cache.put_maia_policy("d2d4 e7e5", &policy_b).unwrap();

        let cached_a = cache.get_maia_policy("e2e4 d7d5").unwrap();
        let cached_b = cache.get_maia_policy("d2d4 e7e5").unwrap();
        assert!(cached_a.contains_key("move_a"));
        assert!(cached_b.contains_key("move_b"));
    }
}
