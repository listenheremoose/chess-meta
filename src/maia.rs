use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::engine::parse_verbose_move_stats;

/// Persistent lc0 process running Maia weights for human-play prediction.
pub struct MaiaEngine {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    query_count: u32,
    ucinewgame_interval: u32,
}

impl MaiaEngine {
    pub fn new(lc0_path: &str, maia_weights_path: &str, ucinewgame_interval: u32) -> Result<Self, String> {
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
            .map_err(|e| format!("Failed to spawn Maia engine: {e}"))?;

        let stdin = child.stdin.take().ok_or("No stdin")?;
        let stdout = child.stdout.take().ok_or("No stdout")?;
        let reader = BufReader::new(stdout);

        let mut engine = MaiaEngine {
            child,
            stdin,
            reader,
            query_count: 0,
            ucinewgame_interval,
        };

        engine.send("uci")?;
        engine.wait_for("uciok")?;

        engine.send(&format!(
            "setoption name WeightsFile value {maia_weights_path}"
        ))?;
        engine.send("setoption name VerboseMoveStats value true")?;
        engine.send("setoption name MultiPV value 500")?;
        engine.send("setoption name SmartPruningFactor value 0")?;

        engine.send("isready")?;
        engine.wait_for("readyok")?;

        Ok(engine)
    }

    /// Get Maia's human-play probability distribution for a position.
    /// `move_sequence` must be the full move sequence from game start (Maia requires history).
    /// Returns map of UCI move -> policy percentage (0-100).
    pub fn predict(&mut self, move_sequence: &str) -> Result<HashMap<String, f32>, String> {
        // Periodically send ucinewgame to clear internal state
        self.query_count += 1;
        if self.query_count % self.ucinewgame_interval == 0 {
            self.send("ucinewgame")?;
            self.send("isready")?;
            self.wait_for("readyok")?;
        }

        let position_cmd = if move_sequence.is_empty() {
            "position startpos".to_string()
        } else {
            format!("position startpos moves {move_sequence}")
        };

        self.send(&position_cmd)?;
        self.send("go nodes 1")?;

        let mut policy_map: HashMap<String, f32> = HashMap::new();

        loop {
            let line = self.read_line()?;

            if line.starts_with("bestmove") {
                break;
            }

            if line.starts_with("info string") {
                if let Some((uci_move, policy_pct, _q_value)) = parse_verbose_move_stats(&line) {
                    policy_map.insert(uci_move, policy_pct);
                }
            }
        }

        Ok(policy_map)
    }

    fn send(&mut self, cmd: &str) -> Result<(), String> {
        writeln!(self.stdin, "{cmd}")
            .map_err(|e| format!("Failed to write to Maia engine: {e}"))?;
        Ok(())
    }

    fn read_line(&mut self) -> Result<String, String> {
        let mut line = String::new();
        let bytes = self.reader
            .read_line(&mut line)
            .map_err(|e| format!("Failed to read from Maia engine: {e}"))?;
        if bytes == 0 {
            return Err("Maia engine process terminated unexpectedly".to_string());
        }
        Ok(line.trim().to_string())
    }

    fn wait_for(&mut self, expected: &str) -> Result<(), String> {
        loop {
            let line = self.read_line()?;
            if line.starts_with(expected) {
                return Ok(());
            }
        }
    }
}

impl Drop for MaiaEngine {
    fn drop(&mut self) {
        let _ = writeln!(self.stdin, "quit");
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
