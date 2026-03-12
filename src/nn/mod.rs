pub mod encoding;
pub mod policy_map;

use std::collections::HashMap;

use ort::value::Tensor;
use shakmaty::Position;

use crate::engine::EngineEval;
use encoding::{INPUT_PLANES, build_history, encode_position};
use policy_map::decode_policy;

#[derive(Debug)]
pub enum NNError {
    OrtError(String),
    ModelLoad(String),
    ShapeError(String),
    NoOutput(String),
}

impl std::fmt::Display for NNError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OrtError(e) => write!(f, "ONNX Runtime error: {e}"),
            Self::ModelLoad(e) => write!(f, "Failed to load ONNX model: {e}"),
            Self::ShapeError(e) => write!(f, "Tensor shape error: {e}"),
            Self::NoOutput(e) => write!(f, "Missing model output: {e}"),
        }
    }
}

impl std::error::Error for NNError {}

impl From<ort::Error> for NNError {
    fn from(e: ort::Error) -> Self {
        NNError::OrtError(e.to_string())
    }
}

/// Direct neural network evaluator using ONNX Runtime.
///
/// Replaces the lc0 UCI process approach with direct inference on the
/// weight files, eliminating I/O overhead and enabling future batching.
pub struct NNEvaluator {
    engine_session: ort::session::Session,
    maia_session: ort::session::Session,
}

impl NNEvaluator {
    /// Create a new evaluator by loading two ONNX models (engine + Maia weights).
    ///
    /// The weight files must be in ONNX format. Convert lc0 `.pb.gz` weights via:
    /// ```text
    /// lc0 --backend=onnx --weights=<file>.pb.gz --onnx-export=<file>.onnx
    /// ```
    pub fn new(engine_onnx_path: &str, maia_onnx_path: &str) -> Result<Self, NNError> {
        let engine_session = ort::session::Session::builder()?
            .commit_from_file(engine_onnx_path)
            .map_err(|e| NNError::ModelLoad(format!("{engine_onnx_path}: {e}")))?;

        let maia_session = ort::session::Session::builder()?
            .commit_from_file(maia_onnx_path)
            .map_err(|e| NNError::ModelLoad(format!("{maia_onnx_path}: {e}")))?;

        log::info!(
            "NNEvaluator initialized engine={engine_onnx_path} maia={maia_onnx_path}"
        );

        Ok(Self {
            engine_session,
            maia_session,
        })
    }

    /// Run the engine network on a position specified by its move sequence.
    ///
    /// Returns an `EngineEval` with WDL, policy percentages, and empty Q values
    /// (Q values require search depth; with a single forward pass they aren't meaningful).
    pub fn evaluate_engine(&mut self, move_sequence: &str) -> Result<EngineEval, NNError> {
        let history = build_history(move_sequence);
        let current = &history[0];
        let is_black = current.turn() == shakmaty::Color::Black;

        let encoded = encode_position(&history);
        let input = Tensor::<f32>::from_array(([1usize, INPUT_PLANES, 8, 8], encoded))?;

        let outputs = self.engine_session.run(ort::inputs![input])?;

        let policy_logits = extract_policy(&outputs)?;
        let wdl = extract_wdl(&outputs)?;

        let legal_moves = legal_uci_moves(current);
        let policy = decode_policy(&policy_logits, &legal_moves, is_black, 0.0);

        Ok(EngineEval {
            wdl,
            policy,
            q_values: HashMap::new(),
        })
    }

    /// Run the Maia network on a position specified by its move sequence.
    ///
    /// Returns a policy map (UCI move → percentage) representing predicted
    /// human move probabilities.
    pub fn evaluate_maia(&mut self, move_sequence: &str) -> Result<HashMap<String, f32>, NNError> {
        let history = build_history(move_sequence);
        let current = &history[0];
        let is_black = current.turn() == shakmaty::Color::Black;

        let encoded = encode_position(&history);
        let input = Tensor::<f32>::from_array(([1usize, INPUT_PLANES, 8, 8], encoded))?;

        let outputs = self.maia_session.run(ort::inputs![input])?;

        let policy_logits = extract_policy(&outputs)?;
        let legal_moves = legal_uci_moves(current);
        let policy = decode_policy(&policy_logits, &legal_moves, is_black, 0.0);

        Ok(policy)
    }

    /// Batch-evaluate multiple positions through both engine and Maia networks.
    ///
    /// Encodes all positions into a single `[N, 112, 8, 8]` tensor and runs
    /// one inference call per network, maximizing GPU utilization.
    pub fn evaluate_batch(
        &mut self,
        move_sequences: &[&str],
    ) -> Result<Vec<(EngineEval, HashMap<String, f32>)>, NNError> {
        let n = move_sequences.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        // Build histories and encode all positions.
        let mut histories: Vec<Vec<shakmaty::Chess>> = Vec::with_capacity(n);
        let mut flat_input = Vec::with_capacity(n * INPUT_PLANES * 64);
        for &move_seq in move_sequences {
            let history = build_history(move_seq);
            let encoded = encode_position(&history);
            flat_input.extend_from_slice(&encoded);
            histories.push(history);
        }

        let input_engine = Tensor::<f32>::from_array(([n, INPUT_PLANES, 8, 8], flat_input.clone()))?;
        let input_maia = Tensor::<f32>::from_array(([n, INPUT_PLANES, 8, 8], flat_input))?;

        // One inference call per network for the whole batch.
        let engine_outputs = self.engine_session.run(ort::inputs![input_engine])?;
        let maia_outputs = self.maia_session.run(ort::inputs![input_maia])?;

        let engine_policy_all = extract_policy(&engine_outputs)?;
        let engine_wdl_all = extract_wdl_batch(&engine_outputs, n)?;
        let maia_policy_all = extract_policy(&maia_outputs)?;

        let policy_size = engine_policy_all.len() / n;
        let maia_policy_size = maia_policy_all.len() / n;

        let mut results = Vec::with_capacity(n);
        for i in 0..n {
            let current = &histories[i][0];
            let is_black = current.turn() == shakmaty::Color::Black;
            let legal_moves = legal_uci_moves(current);

            let engine_logits = &engine_policy_all[i * policy_size..(i + 1) * policy_size];
            let engine_policy = decode_policy(engine_logits, &legal_moves, is_black, 0.0);
            let wdl = engine_wdl_all[i];

            let maia_logits = &maia_policy_all[i * maia_policy_size..(i + 1) * maia_policy_size];
            let maia_policy = decode_policy(maia_logits, &legal_moves, is_black, 0.0);

            results.push((
                EngineEval {
                    wdl,
                    policy: engine_policy,
                    q_values: HashMap::new(),
                },
                maia_policy,
            ));
        }

        Ok(results)
    }
}

/// Extract policy logits from model outputs.
fn extract_policy(outputs: &ort::session::SessionOutputs) -> Result<Vec<f32>, NNError> {
    for name in &["output_policy", "/output/policy"] {
        if let Some(tensor) = outputs.get(*name) {
            let (_, data) = tensor
                .try_extract_tensor::<f32>()
                .map_err(|e| NNError::OrtError(e.to_string()))?;
            return Ok(data.to_vec());
        }
    }
    let (_, tensor) = outputs
        .iter()
        .next()
        .ok_or_else(|| NNError::NoOutput("policy".into()))?;
    let (_, data) = tensor
        .try_extract_tensor::<f32>()
        .map_err(|e| NNError::OrtError(e.to_string()))?;
    Ok(data.to_vec())
}

/// Extract WDL from model outputs. Falls back to converting a scalar value head.
fn extract_wdl(outputs: &ort::session::SessionOutputs) -> Result<(u32, u32, u32), NNError> {
    for name in &["output_wdl", "/output/wdl"] {
        if let Some(tensor) = outputs.get(*name) {
            if let Ok((_, data)) = tensor.try_extract_tensor::<f32>() {
                if data.len() >= 3 {
                    return Ok(softmax_wdl(data[0], data[1], data[2]));
                }
            }
        }
    }

    for name in &["output_value", "/output/value"] {
        if let Some(tensor) = outputs.get(*name) {
            if let Ok((_, data)) = tensor.try_extract_tensor::<f32>() {
                if data.len() >= 3 {
                    return Ok(softmax_wdl(data[0], data[1], data[2]));
                }
                if !data.is_empty() {
                    let v = data[0].clamp(-1.0, 1.0);
                    let w = ((v + 1.0) / 2.0 * 1000.0).round() as u32;
                    let l = 1000 - w;
                    return Ok((w, 0, l));
                }
            }
        }
    }

    let (_, tensor) = outputs
        .iter()
        .nth(1)
        .ok_or_else(|| NNError::NoOutput("value/wdl".into()))?;
    let (_, data) = tensor
        .try_extract_tensor::<f32>()
        .map_err(|e| NNError::OrtError(e.to_string()))?;
    if data.len() >= 3 {
        Ok(softmax_wdl(data[0], data[1], data[2]))
    } else if !data.is_empty() {
        let v = data[0].clamp(-1.0, 1.0);
        let w = ((v + 1.0) / 2.0 * 1000.0).round() as u32;
        let l = 1000 - w;
        Ok((w, 0, l))
    } else {
        Err(NNError::NoOutput("value/wdl".into()))
    }
}

/// Extract WDL values for a batch of N positions.
fn extract_wdl_batch(outputs: &ort::session::SessionOutputs, n: usize) -> Result<Vec<(u32, u32, u32)>, NNError> {
    // Find the WDL tensor.
    let data = find_wdl_data(outputs)?;

    if data.len() >= n * 3 {
        // Shape [N, 3] — WDL logits per position.
        Ok((0..n).map(|i| softmax_wdl(data[i * 3], data[i * 3 + 1], data[i * 3 + 2])).collect())
    } else if data.len() >= n {
        // Shape [N, 1] — scalar value head.
        Ok((0..n).map(|i| {
            let v = data[i].clamp(-1.0, 1.0);
            let w = ((v + 1.0) / 2.0 * 1000.0).round() as u32;
            (w, 0, 1000 - w)
        }).collect())
    } else {
        Err(NNError::ShapeError(format!(
            "WDL output has {} elements, expected {} for batch of {n}", data.len(), n * 3
        )))
    }
}

/// Find WDL/value data in model outputs, trying known names.
fn find_wdl_data(outputs: &ort::session::SessionOutputs) -> Result<Vec<f32>, NNError> {
    for name in &["output_wdl", "/output/wdl", "output_value", "/output/value"] {
        if let Some(tensor) = outputs.get(*name) {
            if let Ok((_, data)) = tensor.try_extract_tensor::<f32>() {
                return Ok(data.to_vec());
            }
        }
    }
    // Fallback: second output.
    let (_, tensor) = outputs.iter().nth(1)
        .ok_or_else(|| NNError::NoOutput("value/wdl".into()))?;
    let (_, data) = tensor.try_extract_tensor::<f32>()
        .map_err(|e| NNError::OrtError(e.to_string()))?;
    Ok(data.to_vec())
}

/// Apply softmax to raw WDL logits, return as (W, D, L) summing to ~1000.
fn softmax_wdl(w: f32, d: f32, l: f32) -> (u32, u32, u32) {
    let max_v = w.max(d).max(l);
    let ew = (w - max_v).exp();
    let ed = (d - max_v).exp();
    let el = (l - max_v).exp();
    let sum = ew + ed + el;
    (
        (ew / sum * 1000.0).round() as u32,
        (ed / sum * 1000.0).round() as u32,
        (el / sum * 1000.0).round() as u32,
    )
}

/// Get all legal UCI move strings for a position.
fn legal_uci_moves(chess: &shakmaty::Chess) -> Vec<String> {
    let legals = chess.legal_moves();
    legals
        .iter()
        .map(|m| {
            let uci = shakmaty::uci::UciMove::from_standard(m.clone());
            uci.to_string()
        })
        .collect()
}
