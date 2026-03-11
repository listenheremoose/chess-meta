# UI Design — Dashboard

Single-window iced application. All panels visible simultaneously. The MCTS search runs in a
background thread; the UI polls for updates and redraws.

## Implementation Status

**Implemented:**
- Four-panel layout (top bar, left move table, right tree view, bottom progress strip)
- Position input, Start/Pause/Reset controls, live stats
- Move comparison table with Engine Q, Practical Q, Delta, Visits columns
- Move detail view (Q white, worst-case, WDL breakdown)
- Canvas tree visualization with visit-based sizing and Q-based coloring
- Best move timeline and Q convergence sparkline
- Dark theme with color palette

**Not yet implemented:**
- Saved positions dropdown
- Settings panel (gear icon) — config is file-based via settings.toml for now
- Column sorting (click headers)
- Row highlighting for highest-delta move
- SAN notation (currently UCI)
- Inline bar visualization in table
- Principal variation display in move detail
- Most likely opponent response in move detail
- Tree zoom/pan, hover tooltips, click-to-subtree navigation, breadcrumb trail
- Collapsed branch `+N` indicators
- Sync between tree click and move table selection

## Layout

```
┌─────────────────────────────────────────────────────────────────────┐
│  TOP BAR — Controls & Status                                        │
├───────────────────────────────────┬─────────────────────────────────┤
│                                   │                                 │
│  LEFT — Move Comparison           │  RIGHT — Search Tree            │
│                                   │                                 │
├───────────────────────────────────┴─────────────────────────────────┤
│  BOTTOM — Search Progress                                           │
└─────────────────────────────────────────────────────────────────────┘
```

## Top Bar — Controls & Status

**Position input:**
- Text field accepting a space-joined UCI move sequence (e.g., `e2e4 e7e5 g1f3`)
- Dropdown to select from previously searched positions (stored in SQLite)

**Search controls:**
- Start button — begins MCTS from the entered position
- Pause button — suspends iteration loop, tree state preserved
- Reset button — clears current tree, starts fresh

**Live stats (read-only text):**
- `Iterations: 3,412 / 5,000`
- `Elapsed: 52s`
- `Best: Nf3 (stable 1,200 iters)` — shows how long the current best move has held
- `Nodes: 2,847` — total nodes in tree
- `Iter/s: 65.2`

**Settings (gear icon or expandable section):**
- Engine paths (lc0 binary, engine weights, Maia weights)
- Search parameters: max iterations, max time, contempt, safety

## Left Panel — Move Comparison

### Move table

Scrollable table of root candidate moves (top 3 engine + top 5 Maia, deduped).

| Column | Description |
|--------|-------------|
| Move | UCI notation (e.g., g1f3, d2d4) |
| Engine Q | Raw engine evaluation [0, 1] |
| Practical Q | MCTS result against Maia opponents [0, 1] |
| Delta | Practical - Engine (colored: green positive, red negative) |
| Visits | MCTS visit count (confidence indicator) |
| Bar | Inline horizontal bar showing practical Q, colored by delta |

**Sorting:** Click column headers. Default sort by practical Q descending.

**Row highlighting:** The move with the highest positive delta gets a subtle highlight — this is the
"human exploiter" the system found.

### Move detail (below table, shown when a move is selected)

Clicking a row expands a detail section:

- **Most likely opponent response:** `1...e5 (Maia 68%)` — the move humans play most often
- **Our Q after their top response:** Shows whether the advantage holds
- **WDL breakdown:** Stacked horizontal bar (green=win, gray=draw, red=loss)
- **Principal variation:** The most-visited line through the tree from this move
  `1. Nf3 e5 2. d4 exd4 3. Nxd4` (with Maia % at each opponent move)
- **Safety score:** Worst-case Q among opponent responses with >10% Maia probability

## Right Panel — Search Tree

Custom `Canvas` widget drawing the MCTS tree.

### Node rendering

- **Shape:** Rectangles for MAX nodes (our turn), circles for CHANCE nodes (opponent)
- **Size:** Width proportional to visit count (relative to parent's total visits)
- **Color:** Gradient based on practical Q. Green = good for us, red = bad. Intensity = confidence (more visits = more saturated)
- **Label:** SAN move + visit count. For nodes with enough space, also show Q.

### Layout

Top-down vertical tree. Root at top, branches downward.

- Only nodes above the **min-visits threshold** are shown (adjustable via slider)
- Default threshold: 5% of root visits (e.g., if root has 5000 visits, show nodes with 250+)
- Collapsed branches show a `+N` indicator for hidden children

### Controls (below or overlaid on tree)

- **Min visits slider:** Filter noise by hiding low-visit nodes
- **Max depth slider:** Limit how deep the tree renders
- **Zoom:** Scroll to zoom, drag to pan

### Interaction

- **Hover:** Tooltip with full stats (Q, visits, Maia %, engine rank, WDL)
- **Click:** Selects the node. The move comparison table updates to show children of this node as if it were the root. Breadcrumb trail shows navigation path.
- **Sync with left panel:** Selecting a move in the table highlights the corresponding branch in the tree. Selecting a node in the tree selects the corresponding move in the table.

## Bottom Strip — Search Progress

Horizontal strip showing search progress over time. Uses `Canvas` for custom drawing.

### Best move timeline

Horizontal band divided into colored segments. Each segment = a period where a particular move
was the most-visited. Color-coded by move. Shows at a glance whether the search settled early
or oscillated.

```
[  Nf3  |  d4  |      Nf3                              ]
 0     200    500    3412 iterations
```

### Q convergence sparkline

Small line chart showing the Q-value of the current best move over iterations. Should plateau
when converged. If it's still moving, the search needs more time.

### Iterations per second

Simple text or small bar showing current throughput. Useful for detecting lc0 slowdowns (memory
issues, etc.).

## Data Flow

```
Background thread                    UI thread (iced)
─────────────────                    ────────────────
MCTS loop runs                       iced subscription polls every 100ms
  → every 50 iterations, sends       → coordinator.poll() drains mpsc channel
    SearchSnapshot via mpsc channel     → keeps latest snapshot
                                        → clears canvas caches
                                        → triggers view redraw
```

Communication uses an `mpsc` channel (not `Arc<Mutex<>>`). The background thread owns the full MCTS tree and engines. It sends periodic `SearchSnapshot` structs containing:
- Root candidate moves with all stats (engine Q, practical Q, delta, visits, WDL)
- Flattened tree nodes (pre-pruned by min-visit threshold)
- Iteration count, elapsed time, iterations/sec
- Best move history (for the timeline)
- Q convergence history (for the sparkline)

The UI never touches the full MCTS tree directly — it reads snapshots that the
coordinator sends every 50 iterations.

## Color Palette

| Element | Color | Usage |
|---------|-------|-------|
| Positive delta | Green (#4CAF50) | Moves that beat engine expectation |
| Negative delta | Red (#F44336) | Moves that underperform |
| Neutral | Gray (#9E9E9E) | No significant delta |
| MAX node | Blue family (#2196F3) | Our decision nodes in tree |
| CHANCE node | Orange family (#FF9800) | Opponent nodes in tree |
| Win | Green (#66BB6A) | WDL bar segment |
| Draw | Gray (#BDBDBD) | WDL bar segment |
| Loss | Red (#EF5350) | WDL bar segment |
| Background | Dark (#1E1E1E) | Main window background |
| Panel background | Slightly lighter (#2D2D2D) | Panel backgrounds |
| Text | Light (#E0E0E0) | Primary text |
| Secondary text | Dimmer (#9E9E9E) | Labels, less important info |

Dark theme by default — matches typical chess analysis tools and reduces eye strain for
long-running analysis sessions.
