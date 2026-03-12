use std::collections::HashMap;
use std::sync::LazyLock;

/// Queen-type move directions: N, NE, E, SE, S, SW, W, NW.
/// Each is (delta_file, delta_rank).
const QUEEN_DIRS: [(i8, i8); 8] = [
    (0, 1),   // N
    (1, 1),   // NE
    (1, 0),   // E
    (1, -1),  // SE
    (0, -1),  // S
    (-1, -1), // SW
    (-1, 0),  // W
    (-1, 1),  // NW
];

/// Knight move deltas (clockwise from ~1 o'clock).
const KNIGHT_DELTAS: [(i8, i8); 8] = [
    (1, 2),
    (2, 1),
    (2, -1),
    (1, -2),
    (-1, -2),
    (-2, -1),
    (-2, 1),
    (-1, 2),
];

/// Underpromotion pieces: knight, bishop, rook (queen is the default, not encoded here).
const UNDERPROMO_PIECES: [char; 3] = ['n', 'b', 'r'];

/// Underpromotion movement directions: NW (left-capture), N (straight), NE (right-capture).
const UNDERPROMO_DIRS: [(i8, i8); 3] = [(-1, 1), (0, 1), (1, 1)];

/// Pre-computed mapping: NN policy index (0..1857) → UCI move string.
static INDEX_TO_MOVE: LazyLock<Vec<String>> = LazyLock::new(generate_move_table);

/// Pre-computed reverse mapping: UCI move string → NN policy index.
static MOVE_TO_INDEX: LazyLock<HashMap<String, usize>> = LazyLock::new(|| {
    INDEX_TO_MOVE
        .iter()
        .enumerate()
        .map(|(i, m)| (m.clone(), i))
        .collect()
});

fn square_to_str(file: i8, rank: i8) -> String {
    let f = (b'a' + file as u8) as char;
    let r = (b'1' + rank as u8) as char;
    format!("{f}{r}")
}

/// Generate the full 1858-entry move table matching lc0's policy index ordering.
///
/// Iteration order: 73 movement planes × 64 source squares, keeping only moves
/// where the destination is on the board.
///
/// - Planes 0–55: queen-type moves (8 directions × 7 distances)
/// - Planes 56–63: knight moves (8 deltas)
/// - Planes 64–72: underpromotions (3 pieces × 3 directions)
fn generate_move_table() -> Vec<String> {
    let mut moves = Vec::with_capacity(1858);

    // Queen-type moves: 56 planes
    for dir in &QUEEN_DIRS {
        for dist in 1..=7i8 {
            for sq in 0..64 {
                let file = sq % 8;
                let rank = sq / 8;
                let nf = file + dir.0 * dist;
                let nr = rank + dir.1 * dist;
                if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                    let from = square_to_str(file, rank);
                    let to = square_to_str(nf, nr);
                    moves.push(format!("{from}{to}"));
                }
            }
        }
    }

    // Knight moves: 8 planes
    for delta in &KNIGHT_DELTAS {
        for sq in 0..64i8 {
            let file = sq % 8;
            let rank = sq / 8;
            let nf = file + delta.0;
            let nr = rank + delta.1;
            if nf >= 0 && nf < 8 && nr >= 0 && nr < 8 {
                let from = square_to_str(file, rank);
                let to = square_to_str(nf, nr);
                moves.push(format!("{from}{to}"));
            }
        }
    }

    // Underpromotions: 9 planes (only from rank 6 → rank 7)
    for piece in &UNDERPROMO_PIECES {
        for dir in &UNDERPROMO_DIRS {
            for sq in 0..64i8 {
                let file = sq % 8;
                let rank = sq / 8;
                if rank != 6 {
                    continue;
                }
                let nf = file + dir.0;
                if nf < 0 || nf >= 8 {
                    continue;
                }
                let from = square_to_str(file, rank);
                let to = square_to_str(nf, 7);
                moves.push(format!("{from}{to}{piece}"));
            }
        }
    }

    assert_eq!(
        moves.len(),
        1858,
        "Expected 1858 policy moves, got {}",
        moves.len()
    );
    moves
}

/// Flip a rank character: '1'↔'8', '2'↔'7', etc.
fn flip_rank_char(c: char) -> char {
    let r = c as u8 - b'1'; // 0..7
    (b'8' - r) as char
}

/// Flip a UCI move string vertically (for Black's perspective).
///
/// "e7e5" → "e2e4", "a7a8q" → "a2a1q"
fn flip_move(uci: &str) -> String {
    let bytes = uci.as_bytes();
    let mut result = String::with_capacity(uci.len());
    result.push(bytes[0] as char); // from file
    result.push(flip_rank_char(bytes[1] as char)); // from rank
    result.push(bytes[2] as char); // to file
    result.push(flip_rank_char(bytes[3] as char)); // to rank
    if bytes.len() > 4 {
        result.push(bytes[4] as char); // promotion piece
    }
    result
}

/// Look up the NN policy index for a UCI move.
///
/// When `flip` is true (Black to move), the move is vertically flipped
/// before lookup, since the NN always sees moves from White's perspective.
///
/// Queen promotions: strips the trailing 'q' since lc0 encodes queen
/// promotions as regular moves (within the queen-type movement planes).
pub fn move_to_nn_index(uci: &str, flip: bool) -> Option<usize> {
    let mut key = if flip {
        flip_move(uci)
    } else {
        uci.to_string()
    };

    // Strip queen promotion suffix — lc0 encodes queen promo as a regular move.
    if key.len() == 5 && key.ends_with('q') {
        key.pop();
    }

    MOVE_TO_INDEX.get(&key).copied()
}

/// Convert an NN policy index back to a UCI move string.
///
/// When `flip` is true (Black to move), the resulting move is vertically
/// flipped back to the original board orientation.
pub fn nn_index_to_move(idx: usize, flip: bool) -> Option<String> {
    let move_str = INDEX_TO_MOVE.get(idx)?;
    if flip {
        Some(flip_move(move_str))
    } else {
        Some(move_str.clone())
    }
}

/// Decode a raw policy logit vector into a `HashMap<UCI move, probability>`.
///
/// Applies softmax over all legal moves, filters out moves below `min_pct`,
/// and returns probabilities as percentages (0–100).
///
/// `legal_moves` should be UCI move strings for all legal moves in the position.
/// `flip` should be true when Black is the side to move.
pub fn decode_policy(
    logits: &[f32],
    legal_moves: &[String],
    flip: bool,
    min_pct: f32,
) -> HashMap<String, f32> {
    // Gather logits for legal moves.
    let mut move_logits: Vec<(&String, f32)> = Vec::with_capacity(legal_moves.len());
    for uci in legal_moves {
        let logit = match move_to_nn_index(uci, flip) {
            Some(idx) => logits[idx],
            None => {
                log::warn!("No NN index for legal move '{uci}' (flip={flip})");
                continue;
            }
        };
        move_logits.push((uci, logit));
    }

    if move_logits.is_empty() {
        return HashMap::new();
    }

    // Softmax over legal moves only.
    let max_logit = move_logits
        .iter()
        .map(|(_, l)| *l)
        .fold(f32::NEG_INFINITY, f32::max);
    let exps: Vec<f32> = move_logits.iter().map(|(_, l)| (l - max_logit).exp()).collect();
    let sum: f32 = exps.iter().sum();

    let mut policy = HashMap::with_capacity(move_logits.len());
    for (i, (uci, _)) in move_logits.iter().enumerate() {
        let pct = exps[i] / sum * 100.0;
        if pct >= min_pct {
            policy.insert((*uci).clone(), pct);
        }
    }
    policy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_table_has_1858_entries() {
        assert_eq!(INDEX_TO_MOVE.len(), 1858);
    }

    #[test]
    fn move_table_has_no_duplicates() {
        assert_eq!(MOVE_TO_INDEX.len(), 1858);
    }

    #[test]
    fn e2e4_has_an_index() {
        assert!(MOVE_TO_INDEX.contains_key("e2e4"));
    }

    #[test]
    fn knight_move_g1f3_has_an_index() {
        assert!(MOVE_TO_INDEX.contains_key("g1f3"));
    }

    #[test]
    fn underpromotion_a7a8n_has_an_index() {
        assert!(MOVE_TO_INDEX.contains_key("a7a8n"));
    }

    #[test]
    fn queen_promotion_not_in_table() {
        // Queen promotions are encoded as regular moves (no 'q' suffix).
        assert!(!MOVE_TO_INDEX.contains_key("a7a8q"));
        // But the regular move "a7a8" IS in the table.
        assert!(MOVE_TO_INDEX.contains_key("a7a8"));
    }

    #[test]
    fn move_to_nn_index_strips_queen_promo() {
        let idx_plain = move_to_nn_index("a7a8", false);
        let idx_queen = move_to_nn_index("a7a8q", false);
        assert_eq!(idx_plain, idx_queen);
    }

    #[test]
    fn move_to_nn_index_flip_reverses_rank() {
        // e7e5 for Black (flip=true) should map to e2e4
        let idx_flipped = move_to_nn_index("e7e5", true);
        let idx_direct = move_to_nn_index("e2e4", false);
        assert_eq!(idx_flipped, idx_direct);
    }

    #[test]
    fn nn_index_to_move_roundtrips() {
        for (i, m) in INDEX_TO_MOVE.iter().enumerate() {
            let idx = move_to_nn_index(m, false).unwrap();
            assert_eq!(idx, i, "roundtrip failed for move '{m}'");
            let back = nn_index_to_move(i, false).unwrap();
            assert_eq!(&back, m, "reverse failed for index {i}");
        }
    }

    #[test]
    fn flip_move_symmetric() {
        assert_eq!(flip_move("e2e4"), "e7e5");
        assert_eq!(flip_move("e7e5"), "e2e4");
        assert_eq!(flip_move("a7a8q"), "a2a1q");
    }

    #[test]
    fn decode_policy_softmax_sums_to_100() {
        // Create fake logits where only a few moves are legal.
        let logits = vec![0.0f32; 1858];
        let legal = vec!["e2e4".to_string(), "d2d4".to_string(), "g1f3".to_string()];
        let policy = decode_policy(&logits, &legal, false, 0.0);

        let sum: f32 = policy.values().sum();
        assert!((sum - 100.0).abs() < 0.01, "sum was {sum}");
        // Equal logits → equal probabilities
        for (_, pct) in &policy {
            assert!((*pct - 100.0 / 3.0).abs() < 0.1);
        }
    }

    #[test]
    fn decode_policy_filters_below_min_pct() {
        let mut logits = vec![0.0f32; 1858];
        // Give one move a much higher logit
        let e2e4_idx = move_to_nn_index("e2e4", false).unwrap();
        logits[e2e4_idx] = 10.0;

        let legal = vec![
            "e2e4".to_string(),
            "d2d4".to_string(),
            "g1f3".to_string(),
        ];
        let policy = decode_policy(&logits, &legal, false, 1.0);

        // e2e4 should dominate, others should be filtered out
        assert!(policy.contains_key("e2e4"));
        assert!(*policy.get("e2e4").unwrap() > 99.0);
    }
}
