use shakmaty::{CastlingSide, Chess, Color, Position, Role};

/// Number of history positions encoded.
const HISTORY_DEPTH: usize = 8;
/// Planes per history position: 6 our pieces + 6 their pieces + 1 repetition.
const PLANES_PER_BOARD: usize = 13;
/// Start of auxiliary planes.
const AUX_PLANE_BASE: usize = HISTORY_DEPTH * PLANES_PER_BOARD; // 104
/// Total input planes.
pub const INPUT_PLANES: usize = AUX_PLANE_BASE + 8; // 112

const ROLES: [Role; 6] = [
    Role::Pawn,
    Role::Knight,
    Role::Bishop,
    Role::Rook,
    Role::Queen,
    Role::King,
];

/// Flip a bitboard vertically (swap rank 1↔8, 2↔7, etc.).
fn flip_vertical(bb: u64) -> u64 {
    bb.swap_bytes()
}

/// Write a 64-bit bitboard into a plane of the output tensor.
fn write_bitboard_plane(planes: &mut [f32], plane_idx: usize, bits: u64) {
    let offset = plane_idx * 64;
    for sq in 0..64 {
        if (bits >> sq) & 1 == 1 {
            planes[offset + sq] = 1.0;
        }
    }
}

/// Fill all 64 squares of a plane with a single value.
fn fill_plane(planes: &mut [f32], plane_idx: usize, value: f32) {
    let offset = plane_idx * 64;
    for i in 0..64 {
        planes[offset + i] = value;
    }
}

/// Build position history from a move sequence string.
///
/// Returns a `Vec<Chess>` where `[0]` is the current position,
/// `[1]` is one ply back, etc. Truncated to `HISTORY_DEPTH` entries.
pub fn build_history(move_sequence: &str) -> Vec<Chess> {
    let mut positions = vec![Chess::default()];

    if !move_sequence.trim().is_empty() {
        for token in move_sequence.split_whitespace() {
            let chess = positions.last().unwrap().clone();
            let uci_move: shakmaty::uci::UciMove = match token.parse() {
                Ok(m) => m,
                Err(_) => break,
            };
            let legal = match uci_move.to_move(&chess) {
                Ok(m) => m,
                Err(_) => break,
            };
            let mut next = chess;
            next.play_unchecked(legal);
            positions.push(next);
        }
    }

    positions.reverse();
    positions.truncate(HISTORY_DEPTH);
    positions
}

/// Encode a position with history into a flat `[112 * 64]` tensor
/// in NCHW layout (plane-major, then rank-major within each plane).
///
/// Uses lc0's **classical 112-plane** input format:
/// - Planes 0–103: 8 history positions × 13 planes each
///   (our P/N/B/R/Q/K, their P/N/B/R/Q/K, repetition)
/// - Planes 104–111: auxiliary (castling, side-to-move, rule50, ones)
///
/// `history[0]` is the current position, `history[1]` is one ply back, etc.
pub fn encode_position(history: &[Chess]) -> Vec<f32> {
    let mut planes = vec![0.0f32; INPUT_PLANES * 64];

    let current = &history[0];
    let current_turn = current.turn();

    // ── Piece planes for each history position ──────────────────────────
    for (i, pos) in history.iter().enumerate().take(HISTORY_DEPTH) {
        let base = i * PLANES_PER_BOARD;
        let board = pos.board();

        // Determine whose perspective this history entry is from.
        // history[0]: current side to move, history[1]: opponent, etc.
        let stm = if i % 2 == 0 { current_turn } else { !current_turn };
        let opponent = !stm;
        let flip = stm == Color::Black;

        for (r, role) in ROLES.iter().enumerate() {
            let our_bb = u64::from(board.by_color(stm) & board.by_role(*role));
            let their_bb = u64::from(board.by_color(opponent) & board.by_role(*role));

            let our_bits = if flip { flip_vertical(our_bb) } else { our_bb };
            let their_bits = if flip { flip_vertical(their_bb) } else { their_bb };

            write_bitboard_plane(&mut planes, base + r, our_bits);
            write_bitboard_plane(&mut planes, base + 6 + r, their_bits);
        }

        // Plane 12 of each block: repetition flag.
        // We don't track repetitions in MCTS, so leave as 0.
    }

    // ── Auxiliary planes (104–111) ──────────────────────────────────────
    // Classical format.
    let (our_color, their_color) = (current_turn, !current_turn);
    let castles = current.castles();

    // 104: We can castle queenside
    if castles.has(our_color, CastlingSide::QueenSide) {
        fill_plane(&mut planes, AUX_PLANE_BASE, 1.0);
    }
    // 105: We can castle kingside
    if castles.has(our_color, CastlingSide::KingSide) {
        fill_plane(&mut planes, AUX_PLANE_BASE + 1, 1.0);
    }
    // 106: They can castle queenside
    if castles.has(their_color, CastlingSide::QueenSide) {
        fill_plane(&mut planes, AUX_PLANE_BASE + 2, 1.0);
    }
    // 107: They can castle kingside
    if castles.has(their_color, CastlingSide::KingSide) {
        fill_plane(&mut planes, AUX_PLANE_BASE + 3, 1.0);
    }

    // 108: Side to move (all 1s if Black to move)
    if current_turn == Color::Black {
        fill_plane(&mut planes, AUX_PLANE_BASE + 4, 1.0);
    }

    // 109: Rule50 counter (halfmove clock as float, fills entire plane)
    let rule50 = current.halfmoves() as f32;
    fill_plane(&mut planes, AUX_PLANE_BASE + 5, rule50);

    // 110: Zeros (unused in classical format)

    // 111: All ones
    fill_plane(&mut planes, AUX_PLANE_BASE + 7, 1.0);

    planes
}

#[cfg(test)]
mod tests {
    use super::*;
    use shakmaty::Chess;

    #[test]
    fn encode_startpos_produces_correct_size() {
        let history = vec![Chess::default()];
        let planes = encode_position(&history);
        assert_eq!(planes.len(), INPUT_PLANES * 64);
    }

    #[test]
    fn encode_startpos_has_white_pawns_on_rank_2() {
        let history = vec![Chess::default()];
        let planes = encode_position(&history);

        // Plane 0 = our pawns (White). White pawns on rank 2 = bits 8..15.
        let pawn_plane = &planes[0..64];
        for sq in 0..64u32 {
            let rank = sq / 8;
            let expected = if rank == 1 { 1.0 } else { 0.0 };
            assert_eq!(
                pawn_plane[sq as usize], expected,
                "square {sq} (rank {rank}): expected {expected}"
            );
        }
    }

    #[test]
    fn encode_startpos_has_their_pawns_on_rank_7() {
        let history = vec![Chess::default()];
        let planes = encode_position(&history);

        // Plane 6 = their pawns (Black). Black pawns on rank 7 = bits 48..55.
        let their_pawn_plane = &planes[6 * 64..7 * 64];
        for sq in 0..64u32 {
            let rank = sq / 8;
            let expected = if rank == 6 { 1.0 } else { 0.0 };
            assert_eq!(
                their_pawn_plane[sq as usize], expected,
                "square {sq} (rank {rank}): expected {expected}"
            );
        }
    }

    #[test]
    fn encode_startpos_castling_all_set() {
        let history = vec![Chess::default()];
        let planes = encode_position(&history);

        // All 4 castling planes should be all-1s at startpos.
        for plane_offset in 0..4 {
            let start = (AUX_PLANE_BASE + plane_offset) * 64;
            let plane = &planes[start..start + 64];
            let sum: f32 = plane.iter().sum();
            assert_eq!(sum, 64.0, "castling plane {plane_offset} should be all 1s");
        }
    }

    #[test]
    fn encode_startpos_side_to_move_is_zero() {
        // White to move → plane 108 should be all 0s.
        let history = vec![Chess::default()];
        let planes = encode_position(&history);
        let start = (AUX_PLANE_BASE + 4) * 64;
        let sum: f32 = planes[start..start + 64].iter().sum();
        assert_eq!(sum, 0.0);
    }

    #[test]
    fn encode_startpos_ones_plane() {
        let history = vec![Chess::default()];
        let planes = encode_position(&history);
        let start = (AUX_PLANE_BASE + 7) * 64;
        let sum: f32 = planes[start..start + 64].iter().sum();
        assert_eq!(sum, 64.0);
    }

    #[test]
    fn encode_black_to_move_flips_board() {
        // After 1. e4, Black to move. Black's pawns should appear on rank 2 (flipped).
        let history = build_history("e2e4");
        let planes = encode_position(&history);

        // Plane 0 = our pawns (Black, the side to move). After flip, Black pawns on rank 2.
        let pawn_plane = &planes[0..64];
        // Black has 8 pawns, all on the 7th rank originally, flipped to rank 2.
        let pawn_count: f32 = pawn_plane.iter().sum();
        assert_eq!(pawn_count, 8.0);
        for sq in 0..64u32 {
            let rank = sq / 8;
            if rank == 1 {
                assert_eq!(pawn_plane[sq as usize], 1.0, "sq {sq}");
            }
        }
    }

    #[test]
    fn encode_black_to_move_side_plane_is_one() {
        let history = build_history("e2e4");
        let planes = encode_position(&history);
        let start = (AUX_PLANE_BASE + 4) * 64;
        let sum: f32 = planes[start..start + 64].iter().sum();
        assert_eq!(sum, 64.0);
    }

    #[test]
    fn build_history_startpos_has_one_entry() {
        let history = build_history("");
        assert_eq!(history.len(), 1);
    }

    #[test]
    fn build_history_after_two_moves_has_three_entries() {
        let history = build_history("e2e4 e7e5");
        assert_eq!(history.len(), 3);
        // history[0] = after e4 e5, history[1] = after e4, history[2] = startpos
    }

    #[test]
    fn build_history_truncates_to_eight() {
        // 10 moves = 11 positions, should truncate to 8
        let history = build_history("e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7");
        assert_eq!(history.len(), HISTORY_DEPTH);
    }

    #[test]
    fn history_positions_encode_different_planes() {
        // With 2+ history positions, the piece planes should differ between blocks.
        let history = build_history("e2e4 e7e5");
        let planes = encode_position(&history);

        // Block 0 (current) and block 1 (after e4) should have different pawn configurations.
        let block0_pawns = &planes[0..64];
        let block1_pawns = &planes[13 * 64..(13 + 1) * 64];
        assert_ne!(block0_pawns, block1_pawns);
    }
}
