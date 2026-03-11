# chess-meta

Automated chess analysis tool that finds moves scoring better against real human opponents,
even if they aren't the absolute best engine moves.

## Project overview

Uses lc0 (Leela Chess Zero) with two neural networks:
- **Standard engine weights** — for position evaluation (value + policy)
- **Maia neural net** — predicts what moves humans actually play at various rating levels

Runs a modified MCTS (Monte Carlo Tree Search) where:
- **Our turns (MAX nodes):** PUCT selection explores candidate moves (top 3 engine + top 5 Maia, deduped)
- **Opponent turns (CHANCE nodes):** Moves sampled proportional to Maia's human-prediction distribution
- **Leaf evaluation:** lc0 engine eval at `go nodes 1` (raw NN forward pass)

The result: discover moves that may be slightly suboptimal objectively but lead to positions
where humans consistently struggle, yielding higher practical win rates.

Design docs:
- [docs/mcts-algorithm.md](docs/mcts-algorithm.md) — PUCT/CHANCE logic, selection, expansion, backprop, pseudocode
- [docs/mcts-parameters.md](docs/mcts-parameters.md) — all tunable constants with defaults, ranges, rationale
- [docs/lc0-integration.md](docs/lc0-integration.md) — UCI protocol, process management, caching, stability mitigations
- [docs/known-issues.md](docs/known-issues.md) — weaknesses, mitigations, references
- [docs/ui-design.md](docs/ui-design.md) — dashboard layout, panels, data flow, color palette

## Architecture

```
src/
  main.rs           -- Entry point, launches iced app
  app.rs            -- Main iced Application: state, update, view, subscriptions
  coordinator.rs    -- MCTS loop: select → expand → evaluate → backprop (runs in background thread)
  engine.rs         -- lc0 UCI integration (persistent process, nodes=1 evals)
  maia.rs           -- Maia policy extraction (persistent process, nodes=1)
  search.rs         -- PUCT selection, CHANCE sampling, backpropagation
  cache.rs          -- SQLite: engine cache (EPD-keyed), Maia cache (move-sequence-keyed),
                       MCTS tree persistence (resumable)
  position.rs       -- Position representation via shakmaty, move application, EPD/move-path tracking
  config.rs         -- Settings: engine paths, search parameters, all tunable constants
  ui/
    mod.rs          -- UI module root
    controls.rs     -- Top bar: position input, start/pause/reset, live stats
    move_table.rs   -- Left panel: move comparison table + detail view
    tree_view.rs    -- Right panel: search tree canvas visualization
    progress.rs     -- Bottom strip: convergence sparkline, best-move timeline, iter/sec
```

## Key technical decisions

### Dual cache keys
- **Engine eval** cached by EPD (transposition-safe — same position via different move orders shares eval)
- **Maia policy** cached by full move sequence from game start (Maia requires move history for accurate predictions)
- The search tree stores both EPD and move sequence per node

### lc0 process management
- Two persistent lc0 processes (engine + Maia), reused across all evaluations
- Sequential UCI protocol — one `go` command at a time per process, wait for `bestmove`
- Set `NNCacheSizeMb` explicitly to cap memory (e.g., 512 MB)
- Periodically send `ucinewgame` (every ~500 queries) to clear lc0's internal tree
- Monitor process memory, restart on crash or excessive growth (clear cached process, recreate on next request)
- lc0 UCI options: `VerboseMoveStats true`, `UCI_ShowWDL true`, `MultiPV 500`, `SmartPruningFactor 0`

### Values and perspectives
- All values stored as expected score from **White's perspective** in [0, 1]
- Convert to side-to-move perspective only during PUCT selection
- Value formula: `V = W/1000 + contempt * D/1000` (contempt defaults to 0.6)

### MCTS specifics
- PUCT with dynamic cpuct (init=1.5, base=19652, factor=1.0)
- Blended priors at MAX nodes: 70% engine policy + 30% Maia policy
- FPU reduction: 0.3 for unvisited children
- CHANCE nodes: Maia distribution with temperature smoothing (T=1.0) and exploration floor (epsilon=0.01)
- Filter opponent moves below 0.1% Maia probability
- Persistent tree in SQLite, in-memory working set, periodic flush
- Safety parameter for final move selection: blend expected score with worst-case against likely responses

## UI — Dashboard

Single-window iced dashboard with four panels. See [docs/ui-design.md](docs/ui-design.md) for full spec and color palette.

### Top bar — Controls & Status
- Position input (paste move sequence or select from saved)
- Start / Pause / Reset buttons
- Live stats: iteration count, elapsed time, current best move, stability indicator

### Left panel — Move Comparison
- Table of root candidate moves with columns: move, engine Q, practical Q, visits, delta
- Inline bar visualization colored by delta (positive delta = "human exploiter")
- Sortable by any column (default: practical Q)
- Click a move for details: most likely opponent response + Maia %, our Q after that response, WDL stacked bar, principal variation

### Right panel — Search Tree
- Pruned tree showing only nodes above a configurable visit threshold
- Nodes sized by visit count, colored by practical Q
- Opponent (CHANCE) nodes visually distinct from our (MAX) nodes
- Click a node to navigate the move comparison to that subtree
- Depth limit slider, min-visits filter
- Selecting a move in the left panel highlights its branch

### Bottom strip — Search Progress
- Best move over time (horizontal band showing stability/changes)
- Q convergence sparkline
- Iterations per second

## Reference project

`c:\code\rust-chess` — interactive chess training tool using the same lc0 + Maia stack.
Key files for reference:
- `engine.rs` — UCI protocol, process lifecycle, output parsing
- `maia.rs` — Maia policy extraction, verbose move stats parsing
- `analysis.rs` — Threading, cancellation, cache coordination
- `drill.rs` — Maia-weighted move selection, filtering logic

Copy patterns from rust-chess as needed, but this project has different concerns:
- Dashboard GUI for visualization (not an interactive chess-playing UI)
- High-throughput `nodes=1` evals instead of deep single-position searches
- Tree search instead of single-position analysis
- EPD-based engine cache instead of move-sequence-only

## Code style

- Rust 2024 edition
- GUI framework: `iced` for the dashboard UI
- Chess logic: `shakmaty` crate (same as rust-chess)
- Database: `rusqlite` for SQLite
- Error handling: Custom error enums per module (see error-handling skill)
- Keep modules focused — one concern per file
