# MCTS Parameters

All tunable constants with defaults, valid ranges, and rationale.

## PUCT / Exploration

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| cpuct_init | 1.5 | 0.5 - 5.0 | Base exploration constant. Adapted from lc0's 3.0 on [-1,1] range, halved for [0,1]. |
| cpuct_base | 19652 | 1000 - 100000 | Logarithmic growth base for dynamic cpuct. Matches lc0 default. |
| cpuct_factor | 1.0 | 0.0 - 5.0 | Logarithmic growth multiplier. Adapted from lc0's 2.0, halved for [0,1]. |
| cpuct_depth_decay | 1.0 | 0.0 - 1.0 | Exponential decay on cpuct by tree depth: `C *= decay^depth`. 1.0 = no decay (lc0 default behavior). Values like 0.85-0.95 give broad exploration near the root and exploitation in deep lines. |
| fpu_reduction | 0.3 | 0.0 - 1.0 | FPU penalty for unvisited children. Smaller than lc0's 0.6 (scaled) because our evals are more reliable at low visit counts. |
| alpha (prior blend) | 0.7 | 0.0 - 1.0 | Weight of engine policy vs Maia policy in blended prior at MAX nodes. 1.0 = pure engine, 0.0 = pure Maia. |

## Maia / Opponent Modeling

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| maia_temperature | 1.0 | 0.5 - 2.0 | Temperature for Maia distribution. >1 spreads mass to rare moves, <1 sharpens toward top move. |
| maia_floor (epsilon) | 0.01 | 0.0 - 0.05 | Minimum probability per opponent move after filtering. Prevents complete blindness to surprising responses. |
| maia_min_prob | 0.001 | 0.0 - 0.01 | Drop moves below this Maia probability threshold. 0.001 = 0.1%. |

## Evaluation

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| engine_nodes | 1 | 1 - 10000 | lc0 node budget per leaf eval. 1 = raw NN forward pass (~5-10ms). Higher = more accurate but slower. |
| contempt | 0.6 | 0.0 - 1.0 | Draw scoring: `V = W/1000 + contempt * D/1000`. 0.5 = neutral, >0.5 = believe we can win drawn positions against humans. |

## Final Move Selection

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| safety | 0.2 | 0.0 - 1.0 | Worst-case weighting: `PracticalScore = (1-safety)*Q + safety*Q_worst_likely`. Higher = more conservative, avoids moves that collapse if opponent finds the right response. |

## Search Budget

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| max_nodes | 150000 | 1000 - 1000000 | Maximum tree nodes before stopping search. |
| max_time | none | 1s - ∞ | Wall clock time limit (optional). |

## Candidate Selection

| Parameter | Default | Description |
|-----------|---------|-------------|
| engine_top_n | 3 | Number of top engine moves to include as candidates at MAX nodes. |
| maia_top_n | 5 | Number of top Maia moves to include as candidates at MAX nodes. |

These are deduplicated, yielding typically 5-8 candidates per MAX node.

## lc0 Process Management

| Parameter | Default | Description |
|-----------|---------|-------------|
| nn_cache_size_mb | 512 | NNCacheSizeMb sent to lc0 to cap memory growth. |
| ucinewgame_interval | 500 | Send `ucinewgame` every N queries to clear lc0's internal state. |

## Persistence

| Parameter | Default | Description |
|-----------|---------|-------------|
| flush_interval | 100 | Write in-memory tree state to SQLite every N iterations. |
