use shakmaty::{Chess, Color, Move, Position, fen::Epd, uci::UciMove};

/// Tracks position state with both EPD (for engine cache) and move sequence (for Maia cache).
#[derive(Debug, Clone)]
pub struct PositionState {
    /// Current chess position.
    pub chess: Chess,
    /// Space-separated UCI moves from game start.
    pub move_sequence: String,
    /// EPD string for the current position (transposition-safe key).
    pub epd: String,
}

impl PositionState {
    /// Create a new position from the starting position.
    pub fn startpos() -> Self {
        let chess = Chess::default();
        let epd = format_epd(&chess);
        Self {
            chess,
            move_sequence: String::new(),
            epd,
        }
    }

    /// Create a position from a space-separated UCI move sequence.
    pub fn from_moves(moves_str: &str) -> Result<Self, String> {
        let mut chess = Chess::default();
        let mut move_sequence = String::new();

        if moves_str.trim().is_empty() {
            return Ok(Self::startpos());
        }

        for token in moves_str.split_whitespace() {
            let uci_move: UciMove = token
                .parse()
                .map_err(|e| format!("Invalid UCI move '{token}': {e}"))?;
            let legal_move = uci_move
                .to_move(&chess)
                .map_err(|e| format!("Illegal move '{token}': {e}"))?;
            chess.play_unchecked(&legal_move);

            if !move_sequence.is_empty() {
                move_sequence.push(' ');
            }
            move_sequence.push_str(token);
        }

        let epd = format_epd(&chess);
        Ok(Self {
            chess,
            move_sequence,
            epd,
        })
    }

    /// Apply a UCI move string to this position, returning a new PositionState.
    pub fn apply_uci(&self, uci_str: &str) -> Result<Self, String> {
        let uci_move: UciMove = uci_str
            .parse()
            .map_err(|e| format!("Invalid UCI move '{uci_str}': {e}"))?;
        let legal_move = uci_move
            .to_move(&self.chess)
            .map_err(|e| format!("Illegal move '{uci_str}': {e}"))?;

        let mut new_chess = self.chess.clone();
        new_chess.play_unchecked(&legal_move);

        let mut new_move_seq = self.move_sequence.clone();
        if !new_move_seq.is_empty() {
            new_move_seq.push(' ');
        }
        new_move_seq.push_str(uci_str);

        let epd = format_epd(&new_chess);
        Ok(Self {
            chess: new_chess,
            move_sequence: new_move_seq,
            epd,
        })
    }

    /// The side to move.
    pub fn turn(&self) -> Color {
        self.chess.turn()
    }

    /// Is the game over? (checkmate, stalemate, or insufficient material)
    pub fn is_game_over(&self) -> bool {
        self.chess.is_game_over()
    }

    /// Terminal value from White's perspective in [0, 1], if the game is over.
    pub fn terminal_value(&self) -> Option<f64> {
        if self.chess.is_checkmate() {
            // Side to move is checkmated
            Some(if self.chess.turn() == Color::White {
                0.0
            } else {
                1.0
            })
        } else if self.chess.is_stalemate() || self.chess.is_insufficient_material() {
            Some(0.5)
        } else {
            None
        }
    }

    /// Get all legal moves as UCI strings.
    pub fn legal_moves_uci(&self) -> Vec<String> {
        let legals = self.chess.legal_moves();
        legals
            .iter()
            .map(|m| uci_move_str(m))
            .collect()
    }
}

/// Format a position as EPD string.
fn format_epd(chess: &Chess) -> String {
    Epd::from_position(chess.clone(), shakmaty::EnPassantMode::Legal).to_string()
}

/// Convert a shakmaty Move to UCI string.
fn uci_move_str(m: &Move) -> String {
    let uci = UciMove::from_move(m, shakmaty::CastlingMode::Standard);
    uci.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // -- Startpos --

    #[test]
    fn startpos_produces_initial_epd() {
        let pos = PositionState::startpos();
        assert!(pos.epd.contains("rnbqkbnr"));
        assert!(pos.move_sequence.is_empty());
    }

    // -- from_moves --

    #[test]
    fn from_moves_empty_returns_startpos() {
        let pos = PositionState::from_moves("").unwrap();
        assert!(pos.move_sequence.is_empty());
        assert_eq!(pos.turn(), Color::White);
    }

    #[test]
    fn from_moves_tracks_sequence_and_turn() {
        let pos = PositionState::from_moves("e2e4 e7e5").unwrap();
        assert_eq!(pos.move_sequence, "e2e4 e7e5");
        assert_eq!(pos.turn(), Color::White);
    }

    #[test]
    fn from_moves_invalid_uci_returns_error() {
        let result = PositionState::from_moves("zzzz");
        assert!(result.is_err());
    }

    #[test]
    fn from_moves_illegal_move_returns_error() {
        let result = PositionState::from_moves("e2e5"); // Pawn can't jump to e5
        assert!(result.is_err());
    }

    // -- apply_uci --

    #[test]
    fn apply_uci_updates_sequence_and_turn() {
        let pos = PositionState::startpos();
        let pos2 = pos.apply_uci("e2e4").unwrap();
        assert_eq!(pos2.move_sequence, "e2e4");
        assert_eq!(pos2.turn(), Color::Black);
    }

    #[test]
    fn apply_uci_preserves_original_position() {
        let pos = PositionState::startpos();
        let _pos2 = pos.apply_uci("e2e4").unwrap();
        assert_eq!(pos.turn(), Color::White); // Original unchanged
        assert!(pos.move_sequence.is_empty());
    }

    #[test]
    fn apply_uci_illegal_move_returns_error() {
        let pos = PositionState::startpos();
        let result = pos.apply_uci("e1e2"); // King can't move there
        assert!(result.is_err());
    }

    // -- Terminal detection --

    #[test]
    fn terminal_checkmate_returns_winner_value() {
        // Scholar's mate — White wins
        let pos = PositionState::from_moves("e2e4 e7e5 d1h5 b8c6 f1c4 g8f6 h5f7").unwrap();
        assert!(pos.is_game_over());
        assert_eq!(pos.terminal_value(), Some(1.0)); // White wins
    }

    #[test]
    fn terminal_non_game_over_returns_none() {
        let pos = PositionState::from_moves("e2e4").unwrap();
        assert!(!pos.is_game_over());
        assert_eq!(pos.terminal_value(), None);
    }

    // -- EPD transposition safety --

    #[test]
    fn same_position_different_move_order_same_epd() {
        // 1. Nf3 Nf6 2. Nc3 vs 1. Nc3 Nf6 2. Nf3
        let pos_a = PositionState::from_moves("g1f3 g8f6 b1c3").unwrap();
        let pos_b = PositionState::from_moves("b1c3 g8f6 g1f3").unwrap();
        assert_eq!(pos_a.epd, pos_b.epd);
        assert_ne!(pos_a.move_sequence, pos_b.move_sequence);
    }
}
