# lc0 Integration

How chess-meta communicates with lc0 for both engine evaluation and Maia human-prediction.

## Process Architecture

Two persistent lc0 processes run simultaneously:

1. **Engine process** — loaded with standard weights, provides position evaluation (WDL + policy)
2. **Maia process** — loaded with Maia weights, provides human-play probability distributions

Both are spawned once and reused across all evaluations. Each communicates via UCI protocol over stdin/stdout pipes.

## UCI Protocol

### Initialization

On process spawn, send UCI init and options. The two processes use slightly different options:

**Engine process:**
```
uci
setoption name WeightsFile value {weights_path}
setoption name VerboseMoveStats value true
setoption name UCI_ShowWDL value true
setoption name MultiPV value 500
setoption name SmartPruningFactor value 0
setoption name NNCacheSizeMb value 512
isready
```

**Maia process:**
```
uci
setoption name WeightsFile value {maia_weights_path}
setoption name VerboseMoveStats value true
setoption name MultiPV value 500
setoption name SmartPruningFactor value 0
isready
```

Maia doesn't need `UCI_ShowWDL` (we only use its policy) or `NNCacheSizeMb` (its nodes=1 queries don't accumulate cache pressure).

Wait for `readyok` before sending queries.

### Querying a position

**Engine eval:**
```
position startpos moves e2e4 e7e5 g1f3
go nodes 1
```
Parse `info` lines for WDL, policy, and Q values. Wait for `bestmove` to signal completion.

**Maia eval:**
```
position startpos moves e2e4 e7e5 g1f3
go nodes 1
```
Parse `info string` lines for verbose move stats (policy percentages). Wait for `bestmove`.

### Key constraint: Maia requires move sequences

Maia produces accurate human-prediction distributions only when given the full move sequence from the starting position. Sending a FEN/EPD alone will not produce correct results.

The engine eval does not have this restriction — the same position via different move orders produces the same evaluation.

### Sequential protocol

Only one `go` command at a time per process. We must wait for `bestmove` before sending the next query. This provides natural backpressure and prevents pipe flooding.

## Caching Strategy

### Engine cache (EPD-keyed)

Since engine evals depend only on the position (not move history), cache by EPD string. The same position reached via different move orders shares a single cached eval.

```sql
CREATE TABLE engine_cache (
    epd TEXT PRIMARY KEY,
    wdl_w INTEGER NOT NULL,
    wdl_d INTEGER NOT NULL,
    wdl_l INTEGER NOT NULL,
    policy_json TEXT NOT NULL,
    q_values_json TEXT NOT NULL
);
```

WDL is stored as separate integer columns for direct access. Policy and Q values are stored as JSON maps (`{"e2e4": 45.2, "d2d4": 30.1, ...}`).

### Maia cache (move-sequence-keyed)

Since Maia depends on move history, cache by the full move sequence.

```sql
CREATE TABLE maia_cache (
    move_sequence TEXT PRIMARY KEY,
    policy_json TEXT NOT NULL
);
```

## Output Parsing

### Engine output

From `info` lines, extract:
- `score cp` / `score mate` — centipawn or mate score
- `wdl` — win/draw/loss in permille (0-1000 each)
- `multipv` — ranking of each move (1 = best)
- Q value from verbose move stats

From `info string` lines with `VerboseMoveStats true`:
- Per-move policy percentages
- Per-move Q values

### Maia output

From `info string` lines:
- Per-move policy percentages (this is the human-prediction distribution)
- Castling notation: lc0 verbose stats use king-rook notation (e1h1), PV uses king-destination (e1g1) — handle both

## Stability Under Rapid-Fire Queries

With `nodes=1` evals, we send thousands of sequential queries in quick succession.

### Known concerns

- **Memory leaks:** Some lc0 versions leak memory across searches ([lc0 #829](https://github.com/LeelaChessZero/lc0/issues/829)). The NNCache accumulates entries (~350 bytes each) across queries.
- **Internal tree management:** lc0 attempts to reuse/reroot its internal search tree between `go` commands. With rapid position changes, this adds unnecessary overhead.
- **TCEC precedent:** Past tournaments had issues with many consecutive fast searches causing RAM exhaustion.

### Required mitigations

1. **Cap NNCache:** Set `NNCacheSizeMb` explicitly (e.g., 512) to bound memory growth.
2. **Periodic reset:** Send `ucinewgame` every ~500 queries to force lc0 to clear its internal tree and NNCache.
3. **Memory monitoring:** Track lc0 process memory usage. If it exceeds a threshold, restart the process (clear cached engine handle, recreate on next request).
4. **Crash recovery:** On any communication error, clear the cached engine process. The next query will spawn a fresh process. This matches the pattern from rust-chess.

## Process Lifecycle

```
spawn lc0 → send UCI init → send options → isready/readyok
    ↓
[query loop]
    position ... → go nodes 1 → read info lines → bestmove
    (repeat)
    every 500 queries: ucinewgame → isready/readyok
    ↓
on error: kill process, clear handle, recreate on next query
    ↓
on shutdown: send "quit", then kill() as fallback, then wait()
```

## lc0 UCI Options Reference

| Option | Value | Purpose |
|--------|-------|---------|
| WeightsFile | path | Neural network weights file |
| VerboseMoveStats | true | Required for policy extraction from `info string` lines |
| UCI_ShowWDL | true | Enables win/draw/loss output |
| MultiPV | 500 | Analyze all legal moves, not just the best |
| SmartPruningFactor | 0 | Disable pruning to see full move list |
| NNCacheSizeMb | 512 | Cap neural network cache memory |
