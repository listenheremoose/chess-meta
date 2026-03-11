use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

const LINE_BUFFER_CAPACITY: usize = 512;

#[derive(Debug)]
pub enum EngineError {
    SpawnFailed(std::io::Error),
    StdinUnavailable,
    StdoutUnavailable,
    WriteFailed(std::io::Error),
    ReadFailed(std::io::Error),
    ProcessTerminated,
}

impl std::fmt::Display for EngineError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "Failed to spawn lc0 engine: {e}"),
            Self::StdinUnavailable => write!(f, "Engine stdin unavailable"),
            Self::StdoutUnavailable => write!(f, "Engine stdout unavailable"),
            Self::WriteFailed(e) => write!(f, "Failed to write to engine: {e}"),
            Self::ReadFailed(e) => write!(f, "Failed to read from engine: {e}"),
            Self::ProcessTerminated => write!(f, "Engine process terminated unexpectedly"),
        }
    }
}

impl std::error::Error for EngineError {}

type PolicyMap = HashMap<String, f32>;
type VerboseStatsMap = HashMap<String, (f32, Option<f32>)>;

/// Result of a nodes=1 engine evaluation.
#[derive(Debug, Clone)]
pub struct EngineEval {
    /// Win/Draw/Loss from White's perspective (each in 0..1000).
    pub wdl: (u32, u32, u32),
    /// Policy map: UCI move -> policy percentage (0-100).
    pub policy: PolicyMap,
    /// Q values from verbose stats: UCI move -> Q.
    pub q_values: PolicyMap,
}

impl EngineEval {
    /// Compute expected value from White's perspective using V = W/1000 + contempt * D/1000.
    ///
    /// lc0's WDL is from the side-to-move's perspective, so `white_to_move` must be
    /// passed to flip W and L when Black is the side to move.
    pub fn value_white(&self, contempt: f64, white_to_move: bool) -> f64 {
        let (w, d, l) = self.wdl;
        if white_to_move {
            // W = White's wins, L = White's losses
            w as f64 / 1000.0 + contempt * d as f64 / 1000.0
        } else {
            // W = Black's wins (White's losses), L = Black's losses (White's wins)
            l as f64 / 1000.0 + contempt * d as f64 / 1000.0
        }
    }

    /// Get the top N moves by policy.
    #[allow(dead_code)] // Used by integration tests (separate crate, invisible to lint)
    pub fn top_policy_moves(&self, n: usize) -> Vec<(String, f32)> {
        let mut moves = self.policy.iter().map(|(m, p)| (m.clone(), *p)).collect::<Vec<_>>();
        moves.sort_by(|a, b| match b.1.partial_cmp(&a.1) {
            Some(ord) => ord,
            None => std::cmp::Ordering::Equal,
        });
        moves.truncate(n);
        moves
    }
}

/// Persistent lc0 process for engine evaluations.
pub struct Engine {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    /// Reused across reads to avoid repeated allocation.
    line_buffer: String,
    query_count: u32,
    ucinewgame_interval: u32,
}

impl Engine {
    pub fn new(
        lc0_path: &str,
        weights_path: &str,
        nn_cache_size_mb: u32,
        ucinewgame_interval: u32,
    ) -> Result<Self, EngineError> {
        let mut cmd = Command::new(lc0_path);
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let mut child = cmd
            .spawn()
            .map_err(EngineError::SpawnFailed)?;

        let stdin = child.stdin.take().ok_or(EngineError::StdinUnavailable)?;
        let stdout = child.stdout.take().ok_or(EngineError::StdoutUnavailable)?;
        let reader = BufReader::new(stdout);

        let mut engine = Engine {
            child,
            stdin,
            reader,
            line_buffer: String::with_capacity(LINE_BUFFER_CAPACITY),
            query_count: 0,
            ucinewgame_interval,
        };

        engine.send("uci")?;
        engine.wait_for("uciok")?;

        engine.send(&format!("setoption name WeightsFile value {weights_path}"))?;
        engine.send("setoption name VerboseMoveStats value true")?;
        engine.send("setoption name UCI_ShowWDL value true")?;
        engine.send("setoption name MultiPV value 500")?;
        engine.send("setoption name SmartPruningFactor value 0")?;
        engine.send(&format!(
            "setoption name NNCacheSizeMb value {nn_cache_size_mb}"
        ))?;

        engine.send("isready")?;
        engine.wait_for("readyok")?;

        log::info!("Engine initialized path={lc0_path} weights={weights_path}");

        Ok(engine)
    }

    /// Evaluate a position at nodes=1. Returns engine eval with WDL, policy, and Q values.
    /// `move_sequence` is space-separated UCI moves from startpos.
    pub fn evaluate(&mut self, move_sequence: &str, nodes: u64) -> Result<EngineEval, EngineError> {
        // Periodically send ucinewgame to clear lc0 internal tree
        self.query_count += 1;
        if self.query_count % self.ucinewgame_interval == 0 {
            log::debug!("Engine ucinewgame reset at query_count={}", self.query_count);
            self.send("ucinewgame")?;
            self.send("isready")?;
            self.wait_for("readyok")?;
        }

        self.send(&format_position_cmd(move_sequence))?;
        self.send(&format!("go nodes {nodes}"))?;

        let mut wdl = (0u32, 0u32, 0u32);
        let mut verbose_stats: VerboseStatsMap = HashMap::new();

        loop {
            self.read_line_into_buffer()?;
            let line = self.line_buffer.trim();

            if line.starts_with("bestmove") {
                break;
            }

            if line.starts_with("info string") {
                match parse_verbose_move_stats(line) {
                    Some((uci_move, policy_pct, q_value)) => { verbose_stats.insert(uci_move, (policy_pct, q_value)); }
                    None => {}
                }
            } else if line.starts_with("info") && line.contains("wdl") {
                match parse_wdl(line) {
                    Some(parsed_wdl) => wdl = parsed_wdl,
                    None => {}
                }
            }
        }

        let policy = verbose_stats
            .iter()
            .map(|(uci_move, (p, _))| (uci_move.clone(), *p))
            .collect::<PolicyMap>();
        let q_values = verbose_stats
            .iter()
            .filter_map(|(uci_move, (_, q))| q.map(|q_val| (uci_move.clone(), q_val)))
            .collect::<PolicyMap>();

        Ok(EngineEval {
            wdl,
            policy,
            q_values,
        })
    }

    fn send(&mut self, cmd: &str) -> Result<(), EngineError> {
        writeln!(self.stdin, "{cmd}").map_err(|e| {
            log::error!("Failed to write to engine: {e}");
            EngineError::WriteFailed(e)
        })?;
        Ok(())
    }

    /// Read a line into the reusable buffer, avoiding repeated allocation.
    fn read_line_into_buffer(&mut self) -> Result<(), EngineError> {
        self.line_buffer.clear();
        let bytes = self.reader
            .read_line(&mut self.line_buffer)
            .map_err(|e| {
                log::error!("Failed to read from engine: {e}");
                EngineError::ReadFailed(e)
            })?;
        if bytes == 0 {
            log::error!("Engine process terminated unexpectedly");
            return Err(EngineError::ProcessTerminated);
        }
        Ok(())
    }

    fn wait_for(&mut self, expected: &str) -> Result<(), EngineError> {
        loop {
            self.read_line_into_buffer()?;
            if self.line_buffer.trim().starts_with(expected) {
                return Ok(());
            }
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Build the UCI position command from a move sequence.
pub fn format_position_cmd(move_sequence: &str) -> String {
    if move_sequence.is_empty() {
        "position startpos".to_string()
    } else {
        format!("position startpos moves {move_sequence}")
    }
}

/// Parse WDL from an info line.
fn parse_wdl(line: &str) -> Option<(u32, u32, u32)> {
    let tokens = line.split_whitespace().collect::<Vec<&str>>();
    let idx = tokens.iter().position(|&t| t == "wdl")?;
    if idx + 3 < tokens.len() {
        let w: u32 = tokens[idx + 1].parse().ok()?;
        let d: u32 = tokens[idx + 2].parse().ok()?;
        let l: u32 = tokens[idx + 3].parse().ok()?;
        Some((w, d, l))
    } else {
        None
    }
}

/// Parse verbose move stats line.
/// Example: `info string d2d4  (293 ) N:    7934 (+18) (P: 12.71%) (WL: ...) ... (Q:  0.05704) ...`
pub fn parse_verbose_move_stats(line: &str) -> Option<(String, f32, Option<f32>)> {
    let rest = line.strip_prefix("info string ")?;
    let uci_move = rest.split_whitespace().next()?;
    if uci_move == "node" {
        return None;
    }

    let policy_start = rest.find("(P:")?;
    let policy_value_str = &rest[policy_start + 3..];
    let close = policy_value_str.find(')')?;
    let pct_str = policy_value_str[..close]
        .trim()
        .trim_end_matches('%')
        .trim();
    let policy_pct: f32 = pct_str.parse().ok()?;

    let q_value = match rest.find("(Q:") {
        Some(q_start) => {
            let q_value_str = &rest[q_start + 3..];
            match q_value_str.find(')') {
                Some(close) => q_value_str[..close].trim().parse::<f32>().ok(),
                None => None,
            }
        }
        None => None,
    };

    Some((uci_move.to_string(), policy_pct, q_value))
}

/// Map king-destination castling notation to king-rook notation used by lc0 verbose stats.
pub fn castle_to_king_rook(uci: &str) -> Option<&'static str> {
    match uci {
        "e1g1" => Some("e1h1"),
        "e1c1" => Some("e1a1"),
        "e8g8" => Some("e8h8"),
        "e8c8" => Some("e8a8"),
        _ => None,
    }
}

/// Look up a move in a map, trying the king-rook castling alias if direct lookup fails.
pub fn lookup_castling_aware<T>(
    uci_move: &str,
    map: &HashMap<String, T>,
) -> Option<T>
where
    T: Copy,
{
    match map.get(uci_move) {
        Some(v) => Some(*v),
        None => match castle_to_king_rook(uci_move) {
            Some(alt) => map.get(alt).copied(),
            None => None,
        },
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        castle_to_king_rook, format_position_cmd, lookup_castling_aware, parse_verbose_move_stats,
        parse_wdl, EngineEval,
    };

    // -- UCI Output Parsing --

    #[test]
    fn verbose_stats_extracts_move_policy_and_q() {
        let line = "info string d2d4  (293 ) N:    7934 (+18) (P: 12.71%) (WL:  0.05704) (D: 0.745) (M: 197.1) (Q:  0.05704) (U: 0.00749) (S:  0.06484) (V:  0.0303)";
        let (uci_move, policy_pct, q_value) = parse_verbose_move_stats(line).unwrap();
        assert_eq!(uci_move, "d2d4");
        assert!((policy_pct - 12.71).abs() < 0.01);
        assert!((q_value.unwrap() - 0.05704).abs() < 0.0001);
    }

    #[test]
    fn verbose_stats_skips_node_summary_line() {
        let line = "info string node (0 ) N: 100000 (+ 0) (P: 100.00%) (WL: 0.05) (D: 0.7) (M: 200.0) (Q: 0.05) (V: 0.05)";
        assert!(parse_verbose_move_stats(line).is_none());
    }

    #[test]
    fn verbose_stats_handles_missing_q_value() {
        // Some lc0 versions may not include Q for unvisited moves
        let line = "info string e2e4  (0  ) N:       0 (+ 0) (P: 45.20%)";
        let result = parse_verbose_move_stats(line);
        assert!(result.is_some());
        let (uci_move, policy_pct, q_value) = result.unwrap();
        assert_eq!(uci_move, "e2e4");
        assert!((policy_pct - 45.20).abs() < 0.01);
        assert!(q_value.is_none());
    }

    #[test]
    fn wdl_parses_from_info_line() {
        let line = "info depth 1 score cp 30 wdl 400 500 100 pv e2e4";
        assert_eq!(parse_wdl(line), Some((400, 500, 100)));
    }

    #[test]
    fn wdl_returns_none_when_missing() {
        let line = "info depth 1 score cp 30 pv e2e4";
        assert_eq!(parse_wdl(line), None);
    }

    // -- Position Command --

    #[test]
    fn format_position_empty_returns_startpos() {
        assert_eq!(format_position_cmd(""), "position startpos");
    }

    #[test]
    fn format_position_with_moves_appends_them() {
        assert_eq!(
            format_position_cmd("e2e4 e7e5"),
            "position startpos moves e2e4 e7e5"
        );
    }

    // -- Castling Notation --

    #[test]
    fn castle_maps_king_destination_to_king_rook() {
        assert_eq!(castle_to_king_rook("e1g1"), Some("e1h1")); // White kingside
        assert_eq!(castle_to_king_rook("e1c1"), Some("e1a1")); // White queenside
        assert_eq!(castle_to_king_rook("e8g8"), Some("e8h8")); // Black kingside
        assert_eq!(castle_to_king_rook("e8c8"), Some("e8a8")); // Black queenside
    }

    #[test]
    fn castle_returns_none_for_non_castling_move() {
        assert_eq!(castle_to_king_rook("e2e4"), None);
    }

    #[test]
    fn lookup_castling_aware_finds_king_rook_alias() {
        let mut map = HashMap::new();
        map.insert("e1h1".to_string(), 0.5f32); // King-rook notation
        // Lookup by king-destination notation should find it
        assert_eq!(lookup_castling_aware("e1g1", &map), Some(0.5));
    }

    #[test]
    fn lookup_castling_aware_prefers_direct_match() {
        let mut map = HashMap::new();
        map.insert("e1g1".to_string(), 0.7f32); // Direct
        map.insert("e1h1".to_string(), 0.5f32); // King-rook
        assert_eq!(lookup_castling_aware("e1g1", &map), Some(0.7));
    }

    // -- Value Computation --

    #[test]
    fn value_white_applies_contempt_to_draws() {
        let eval = EngineEval {
            wdl: (300, 500, 200),
            policy: HashMap::new(),
            q_values: HashMap::new(),
        };
        let v = eval.value_white(0.6, true);
        assert!((v - 0.6).abs() < 0.001); // 300/1000 + 0.6 * 500/1000 = 0.3 + 0.3 = 0.6
    }

    #[test]
    fn value_white_zero_contempt_ignores_draws() {
        let eval = EngineEval {
            wdl: (300, 500, 200),
            policy: HashMap::new(),
            q_values: HashMap::new(),
        };
        let v = eval.value_white(0.0, true);
        assert!((v - 0.3).abs() < 0.001); // 300/1000 + 0 = 0.3
    }

    #[test]
    fn value_white_full_contempt_treats_draws_as_wins() {
        let eval = EngineEval {
            wdl: (0, 1000, 0),
            policy: HashMap::new(),
            q_values: HashMap::new(),
        };
        let v = eval.value_white(1.0, true);
        assert!((v - 1.0).abs() < 0.001); // 0 + 1.0 * 1000/1000 = 1.0
    }

    #[test]
    fn value_white_flips_wdl_when_black_to_move() {
        // WDL from lc0 when Black to move: W=Black wins, D=draw, L=Black loses (=White wins)
        let eval = EngineEval {
            wdl: (700, 200, 100), // Black wins 70%, draw 20%, White wins 10%
            policy: HashMap::new(),
            q_values: HashMap::new(),
        };
        // White's perspective: W_white=100(L), D=200, contempt=0.6
        let v = eval.value_white(0.6, false);
        assert!((v - 0.22).abs() < 0.001); // 100/1000 + 0.6 * 200/1000 = 0.1 + 0.12 = 0.22
    }
}
