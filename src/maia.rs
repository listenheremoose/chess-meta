use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};

use crate::engine::parse_verbose_move_stats;

#[derive(Debug)]
pub enum MaiaError {
    SpawnFailed(std::io::Error),
    StdinUnavailable,
    StdoutUnavailable,
    WriteFailed(std::io::Error),
    ReadFailed(std::io::Error),
    ProcessTerminated,
}

impl std::fmt::Display for MaiaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SpawnFailed(e) => write!(f, "Failed to spawn Maia engine: {e}"),
            Self::StdinUnavailable => write!(f, "Maia engine stdin unavailable"),
            Self::StdoutUnavailable => write!(f, "Maia engine stdout unavailable"),
            Self::WriteFailed(e) => write!(f, "Failed to write to Maia engine: {e}"),
            Self::ReadFailed(e) => write!(f, "Failed to read from Maia engine: {e}"),
            Self::ProcessTerminated => write!(f, "Maia engine process terminated unexpectedly"),
        }
    }
}

impl std::error::Error for MaiaError {}

/// Persistent lc0 process running Maia weights for human-play prediction.
pub struct MaiaEngine {
    child: Child,
    stdin: ChildStdin,
    reader: BufReader<std::process::ChildStdout>,
    /// Reused across reads to avoid repeated allocation.
    line_buffer: String,
    query_count: u32,
    ucinewgame_interval: u32,
}

impl MaiaEngine {
    pub fn new(lc0_path: &str, maia_weights_path: &str, ucinewgame_interval: u32) -> Result<Self, MaiaError> {
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
            .map_err(MaiaError::SpawnFailed)?;

        let stdin = child.stdin.take().ok_or(MaiaError::StdinUnavailable)?;
        let stdout = child.stdout.take().ok_or(MaiaError::StdoutUnavailable)?;
        let reader = BufReader::new(stdout);

        let mut engine = MaiaEngine {
            child,
            stdin,
            reader,
            line_buffer: String::with_capacity(512),
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

        log::info!("Maia initialized path={lc0_path} weights={maia_weights_path}");

        Ok(engine)
    }

    /// Get Maia's human-play probability distribution for a position.
    /// `move_sequence` must be the full move sequence from game start (Maia requires history).
    /// Returns map of UCI move -> policy percentage (0-100).
    pub fn predict(&mut self, move_sequence: &str) -> Result<HashMap<String, f32>, MaiaError> {
        // Periodically send ucinewgame to clear internal state
        self.query_count += 1;
        if self.query_count % self.ucinewgame_interval == 0 {
            log::debug!("Maia ucinewgame reset at query_count={}", self.query_count);
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
            self.read_line_into_buffer()?;
            let line = self.line_buffer.trim();

            if line.starts_with("bestmove") {
                break;
            }

            if line.starts_with("info string") {
                match parse_verbose_move_stats(line) {
                    Some((uci_move, policy_pct, _q_value)) => { policy_map.insert(uci_move, policy_pct); }
                    None => {}
                }
            }
        }

        Ok(policy_map)
    }

    fn send(&mut self, cmd: &str) -> Result<(), MaiaError> {
        writeln!(self.stdin, "{cmd}")
            .map_err(|e| {
                log::error!("Failed to write to Maia engine: {e}");
                MaiaError::WriteFailed(e)
            })?;
        Ok(())
    }

    /// Read a line into the reusable buffer, avoiding repeated allocation.
    fn read_line_into_buffer(&mut self) -> Result<(), MaiaError> {
        self.line_buffer.clear();
        let bytes = self.reader
            .read_line(&mut self.line_buffer)
            .map_err(|e| {
                log::error!("Failed to read from Maia engine: {e}");
                MaiaError::ReadFailed(e)
            })?;
        if bytes == 0 {
            log::error!("Maia engine process terminated unexpectedly");
            return Err(MaiaError::ProcessTerminated);
        }
        Ok(())
    }

    fn wait_for(&mut self, expected: &str) -> Result<(), MaiaError> {
        loop {
            self.read_line_into_buffer()?;
            if self.line_buffer.trim().starts_with(expected) {
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
