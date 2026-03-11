# MCTS Design for Chess-Meta: Human-Exploiting Move Search

## Overview

Chess-meta uses a modified Monte Carlo Tree Search to find moves that maximize practical winning chances against human opponents. The tree has two types of internal nodes:

- **MAX nodes ("our" turn):** Select moves via UCB/PUCT (exploration/exploitation).
- **CHANCE nodes ("opponent" turn):** Sample moves proportional to Maia's predicted human-play distribution.

Leaf evaluation uses lc0's engine eval (not rollouts). This is essentially an **Expectimax-MCTS hybrid**: maximizing at our nodes, taking expectations at opponent nodes.

---

## 1. UCB Formula Specifics

### Recommended formula: Modified PUCT (AlphaZero-style)

At **MAX nodes** (our turn), select the child `a` that maximizes:

```
Score(s, a) = Q(s, a) + C(s) * P(s, a) * sqrt(N(s)) / (1 + N(s, a))
```

Where:
- `Q(s, a)` = mean value of all simulations through action `a` from state `s`, in range [0, 1] (our win probability)
- `P(s, a)` = prior probability for move `a` (see below)
- `N(s)` = total visit count of state `s` (sum of all child visits)
- `N(s, a)` = visit count of child reached by action `a`
- `C(s)` = exploration constant (see below)

### Exploration constant C

Use lc0's dynamic cpuct formula:

```
C(s) = cpuct_init + cpuct_factor * ln((N(s) + cpuct_base) / cpuct_base)
```

Recommended starting values (adapted from lc0 defaults, adjusted for [0,1] Q range):
- `cpuct_init = 1.5` (lc0 uses 3.0 on [-1,1] range; halved for [0,1])
- `cpuct_base = 19652`
- `cpuct_factor = 1.0` (lc0 uses 2.0 on [-1,1]; halved)

The logarithmic growth ensures that as a node accumulates thousands of visits, exploration gradually increases, preventing premature convergence.

### Prior P(s, a) at MAX nodes

Use a **blended prior** from both the engine policy head and the Maia policy:

```
P(s, a) = alpha * P_engine(s, a) + (1 - alpha) * P_maia(s, a)
```

With `alpha = 0.7` (favor engine policy, but let Maia influence us toward moves humans find hard to respond to).

**Rationale:** Pure engine policy would guide us toward objectively best moves. Pure Maia policy would guide us toward human-like moves. The blend guides us toward strong moves that also happen to arise in positions humans find difficult. The alpha parameter should be tunable; 0.7 is a starting point.

### Candidate move selection at MAX nodes

At our turns, the candidate set is:
- **Top 3 by engine Q-value** (objectively strongest moves)
- **Top 5 by Maia policy** (most human-natural moves)
- Deduplicated, so typically 5-8 candidates per node

This biases toward human-natural moves (since the goal is finding moves that perform well against humans) while still checking the objectively best options. Moves outside this candidate set are not explored at MAX nodes.

### At CHANCE nodes (opponent's turn)

CHANCE nodes do **not** use UCB at all. They are not decision nodes -- they model the stochastic environment of human play. Selection is by sampling from Maia's distribution (see Section 6 for details).

### Why PUCT over UCB1

UCB1 (`Q + C * sqrt(ln(N) / n_i)`) treats all unvisited moves equally. PUCT uses the neural network prior `P(s,a)` to focus early exploration on promising moves. Since we have strong priors from lc0's policy head, PUCT is strictly superior here -- it avoids wasting evaluations on clearly bad moves before visiting good ones.

---

## 2. Selection Phase Details

### Selection algorithm (one iteration)

```
function SELECT(node):
    if node is terminal:
        return node
    if node is leaf (unexpanded):
        return node
    if node.type == MAX:  // our turn
        child = argmax over children of Score(node, child)
        return SELECT(child)
    if node.type == CHANCE:  // opponent turn
        child = sample from Maia distribution (see Section 6)
        if child not yet in tree:
            expand child, return child
        return SELECT(child)
```

### Key design decisions

**Opponent nodes create children on-demand.** Since we sample from Maia's distribution, we may sample a move whose child node doesn't exist yet. In that case, we create it and treat it as the leaf for this iteration. This means CHANCE nodes grow organically based on which moves Maia predicts humans will play most often.

**No UCB at opponent nodes -- and this is correct.** The theoretical justification: we are not trying to find the opponent's best move. We are modeling the opponent as a stochastic process with a known distribution (Maia). This makes opponent nodes equivalent to "chance nodes" in expectimax / stochastic game trees. The MCTS literature on stochastic games (e.g., backgammon dice rolls) handles this identically: sample from the known distribution, do not apply UCB.

**Potential issue: rare but important opponent responses.** If Maia gives 0.1% to a refutation of our move, we'll almost never explore it. This is actually *correct behavior* for our use case -- we're optimizing against the human distribution, not against perfect play. If a human would play the refutation only 0.1% of the time, it contributes only 0.1% to our expected score. However, see Section 6 for optional safeguards.

**Virtual loss for parallelism.** Not needed initially since lc0 is sequential, but if we ever batch evaluations, standard virtual loss (temporarily decrement Q during in-flight evaluations) would apply.

---

## 3. Expansion Strategy

### Single-child expansion

When selection reaches a leaf node, expand **one child only** (the selected move). This is standard MCTS practice and is especially important here because evaluation is expensive (~100ms).

### Expansion procedure

1. Selection reaches leaf node `L`.
2. **Before expansion**, evaluate `L` using lc0 engine eval to get `V(L)` (the value) and `P_engine(L, *)` (the policy over L's legal moves).
3. Also query Maia for `P_maia(L, *)` (human prediction distribution over L's legal moves).
4. Store these on node `L`: the blended prior for each legal move, the Maia distribution (for use if L is a CHANCE node), and the eval value.
5. Backpropagate `V(L)` up the tree.
6. `L` is now an internal node. Its children are **not yet created** -- they will be created lazily when selected/sampled in future iterations.

### Batching Maia and engine evals

Since we need both Maia and engine evals at each leaf, and they use different lc0 instances:

- **Query Maia first** (~10ms) to get the human-prediction distribution.
- **Then query the engine** (~50-100ms) for value and policy.
- Total per-expansion: ~60-110ms.

If running two separate lc0 processes (one for engine, one for Maia), these can be parallelized to ~100ms total.

### First Play Urgency (FPU)

When a MAX node has children that have never been visited, we need a Q estimate for them. Use **FPU reduction** (as lc0 does):

```
Q_fpu(s, a) = Q(s) - fpu_reduction
```

Where `Q(s)` is the parent's mean value and `fpu_reduction = 0.3` (lc0 uses 1.2 on [-1,1], so ~0.6 scaled to [0,1]; we use a smaller value because our evaluations are more reliable than lc0's internal ones at low visit counts). This ensures unvisited children are assumed slightly worse than the current average, encouraging depth-first behavior while still allowing exploration via the prior.

---

## 4. Evaluation

### Engine eval budget

Use a **fixed node budget of 1 node** (just the neural network forward pass, ~5-10ms) for most leaf evaluations, NOT 10k nodes. Rationale:

- At 100ms per eval with 10k nodes, running 1000 MCTS iterations would take 100 seconds. That's too slow for interactive use.
- lc0's raw neural network output (nodes=1) is already a strong evaluation -- it's what AlphaZero-style MCTS was designed for.
- Our MCTS *is* the search. We don't need lc0 to also search internally.
- Use `go nodes 1` in UCI to get just the NN eval.

**With nodes=1:** both engine and Maia evals take ~5-10ms each. Total per leaf: ~10-20ms. 1000 iterations = 10-20 seconds. 5000 iterations = 50-100 seconds. This is much more practical.

**Optional: deeper eval for root children.** For the immediate children of the root (our candidate moves), consider a deeper eval (e.g., 1000 nodes, ~10ms) to get more accurate priors. This front-loaded cost pays for itself by improving the quality of the initial policy.

### Output format: WDL

Configure lc0 with `UCI_ShowWDL true`. Extract:
- `W` = win probability (0-1000 permille)
- `D` = draw probability
- `L` = loss probability

Convert to a value for backpropagation:

```
V = W/1000 + 0.5 * D/1000
```

This gives V in [0, 1] representing expected score from our perspective (1.0 = certain win, 0.5 = draw, 0.0 = certain loss).

### Why WDL over raw Q

WDL is superior because:
- It handles drawn positions correctly (a drawn position is not "half as good" as a win in all contexts, but for expected-score calculations, this is the standard treatment).
- The WDL head is better calibrated than the raw value head in modern lc0 networks.
- It provides richer information for future extensions (e.g., preferring "low-risk" lines with high W+D vs. "swindle" lines with high W+L).

### Maia output

Maia is queried with `go nodes 1`. The output we need is the **policy distribution** (move probabilities), not the value. Extract the policy from the `info` lines. Use the `multipv` option or parse the policy output directly.

**Critical: Maia requires the full move sequence from game start.** When querying Maia, always send `position startpos moves e2e4 e7e5 ...` with the full move history, not just a FEN. This is because Maia's predictions depend on the game trajectory, not just the current position.

---

## 5. Backpropagation

### Value representation

All values are stored as **expected score from White's perspective** in [0, 1]. This avoids sign-flip confusion.

At each node, store:
- `N` = visit count
- `W_sum` = sum of all backpropagated values (from White's perspective)
- `Q = W_sum / N` = mean value (from White's perspective)

### Backpropagation procedure

After evaluating leaf `L` and obtaining value `V_white` (expected score for White):

```
function BACKPROPAGATE(path, V_white):
    for each node in path (leaf to root):
        node.N += 1
        node.W_sum += V_white
        node.Q = node.W_sum / node.N
```

### Perspective handling during selection

When selecting at a MAX node, we need Q from the side-to-move's perspective:

```
Q_stm(s, a) = if white_to_move(s) then Q(child) else 1 - Q(child)
```

This way, Q_stm is always "how good is this for the player choosing." The PUCT formula uses Q_stm.

### Why not negamax-style [-1, 1]

Storing values from a fixed perspective (White) and converting during selection is cleaner for persistent storage in SQLite. It avoids needing to track whose perspective each stored Q represents, which is error-prone when deserializing trees.

---

## 6. Opponent Node (CHANCE Node) Handling

### Maia distribution processing

When we reach an opponent CHANCE node during selection, sample a move from Maia's predicted distribution. The raw Maia output gives `P_maia(s, a)` for each legal move.

### Filtering and smoothing

Apply the following pipeline to Maia's raw probabilities:

**Step 1: Drop negligible moves.**
- Remove moves with P_maia < 0.001 (0.1%). These are moves Maia considers essentially impossible for a human to play.

**Step 2: Apply temperature smoothing.**
Rather than hard caps, use a temperature parameter to control the distribution's sharpness:

```
P_adjusted(a) = P_maia(a)^(1/T) / sum_b(P_maia(b)^(1/T))
```

With `T = 1.0` (no smoothing) as the default. If the distribution is too peaked:
- `T = 1.2` gently spreads probability mass to less-likely moves.
- `T = 0.8` sharpens the distribution further.

**Step 3: Optional exploration floor.**
Ensure every remaining move has at least `epsilon = 0.01` probability (1%), then renormalize. This prevents complete blindness to surprising opponent moves.

```
P_final(a) = max(P_adjusted(a), epsilon)
// then renormalize so sum = 1
```

### When Maia gives 80% to one move

This is **correct behavior** -- if a human plays one move 80% of the time, our expected score should weight that branch 80%. The tree will naturally explore that branch most, giving us the most accurate value estimate where it matters most.

However, the exploration floor (epsilon) ensures we don't have zero information about other responses. With epsilon=0.01 and say 20 legal moves, the dominant move goes from 80% to ~62%, and each rare move gets at least 1%. After 1000 iterations, the dominant branch gets ~620 visits and rare branches get ~10 each -- enough to have a rough value estimate.

### Caching Maia distributions

Store the Maia distribution on the CHANCE node when it's first expanded. This avoids re-querying Maia on every visit. Since CHANCE nodes don't change their distribution, this is safe.

---

## 7. Convergence and Stopping

### Iterations needed

**Rule of thumb:** For a position with B candidate moves at our turn, we need roughly:
- **100 * B iterations** for a rough ordering of moves
- **500 * B iterations** for a reliable ordering
- **2000 * B iterations** for convergence

For a typical chess position with ~30 legal moves: ~3000 for rough, ~15000 for reliable, ~60000 for convergence.

At ~15ms per iteration, this means:
- Rough: ~45 seconds
- Reliable: ~225 seconds (~4 minutes)
- Converged: ~900 seconds (~15 minutes)

### Convergence metrics

Track the following metrics and stop when they stabilize:

1. **Best move stability:** The move with the highest visit count at the root. If the best move hasn't changed in the last 20% of iterations, likely converged.

2. **Q-value stability:** Track `max_Q - second_Q` at the root. When this gap stabilizes (changes by less than 0.01 over 500 iterations), the ranking is stable.

3. **Visit distribution entropy:** `H = -sum(p_i * ln(p_i))` where `p_i = N_i / N_total` for root children. Decreasing entropy means the search is concentrating. Very low entropy means one move dominates.

4. **Running best-move Q-value:** Plot Q of the most-visited root child over time. When it plateaus, stop.

### Practical stopping criteria

```
STOP when ANY of:
  - Iteration count >= max_iterations (user-configurable, default 5000)
  - Wall clock time >= max_time (user-configurable)
  - Best move unchanged for last 30% of iterations AND Q gap > 0.03
  - User requests pause
```

---

## 8. Tree Reuse

### Subtree reuse after a move

If we searched position A and then move `m` is played (reaching position B = A + m):

1. Find the child node of A corresponding to move `m`.
2. If it exists and has been expanded, **re-root the tree** at that node.
3. All statistics (N, Q, children) remain valid.
4. Prune all other branches from the database.

This is fully sound because:
- All values are stored from White's perspective (no sign confusion).
- The Maia distributions stored on opponent nodes remain valid (they depend on move history, which hasn't changed for the reused subtree).
- Engine eval values (position-dependent, not history-dependent) remain valid.

### Two moves ahead (our move + opponent response)

After we play move `m1` and opponent plays `m2`:
1. Navigate root -> m1 -> m2.
2. Re-root there.
3. The subtree under m2 is fully valid.

### Invalidation

The only case where reuse is invalid:
- If Maia's model is updated (e.g., different rating target). In this case, all CHANCE node distributions must be recalculated. For simplicity, discard the tree.

---

## 9. Practical Considerations

### Memory usage

Each node stores:
- Position hash (8 bytes)
- Side to move (1 byte)
- N visit count (4 bytes)
- W_sum value sum (8 bytes, f64)
- Q cached mean (4 bytes, f32)
- Node type: MAX or CHANCE (1 byte)
- Parent pointer (8 bytes)
- Move that led here (2 bytes, encoded)
- Policy prior for each legal move (~30 moves * 6 bytes = ~180 bytes)
- Maia distribution if CHANCE node (~180 bytes)
- Number of children created (4 bytes)

**Per node: ~400 bytes average.**

For 100,000 nodes: ~40 MB. For 1,000,000 nodes: ~400 MB. This is manageable.

### SQLite schema for persistent tree

```sql
CREATE TABLE nodes (
    id          INTEGER PRIMARY KEY,
    hash        BLOB NOT NULL,        -- 8-byte position hash
    epd         TEXT,                  -- EPD string for debugging/engine queries
    move_path   TEXT,                  -- full move sequence from game start (for Maia)
    node_type   INTEGER NOT NULL,      -- 0 = MAX (our turn), 1 = CHANCE (opponent)
    side_to_move INTEGER NOT NULL,     -- 0 = white, 1 = black
    visit_count INTEGER NOT NULL DEFAULT 0,
    value_sum   REAL NOT NULL DEFAULT 0.0,  -- sum of backpropagated V (White perspective)
    q_value     REAL,                  -- cached W_sum / N
    is_terminal INTEGER NOT NULL DEFAULT 0,
    terminal_value REAL,              -- if terminal: 1.0, 0.5, or 0.0
    parent_id   INTEGER REFERENCES nodes(id),
    move        TEXT,                  -- UCI move string (e.g. "e2e4") from parent
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE node_children (
    node_id     INTEGER NOT NULL REFERENCES nodes(id),
    move        TEXT NOT NULL,         -- UCI move string
    child_id    INTEGER REFERENCES nodes(id),  -- NULL if not yet expanded
    prior       REAL NOT NULL,         -- blended prior P(s,a) for MAX; Maia P for CHANCE
    PRIMARY KEY (node_id, move)
);

CREATE TABLE search_sessions (
    id          INTEGER PRIMARY KEY,
    root_node_id INTEGER NOT NULL REFERENCES nodes(id),
    total_iterations INTEGER NOT NULL DEFAULT 0,
    start_position TEXT NOT NULL,      -- starting FEN or move sequence
    config      TEXT,                  -- JSON blob of search parameters
    created_at  TEXT DEFAULT CURRENT_TIMESTAMP,
    updated_at  TEXT DEFAULT CURRENT_TIMESTAMP
);

-- Indexes for efficient traversal
CREATE INDEX idx_nodes_hash ON nodes(hash);
CREATE INDEX idx_nodes_parent ON nodes(parent_id);
CREATE INDEX idx_children_child ON node_children(child_id);
```

### Resume procedure

1. Load the root node from the `search_sessions` table.
2. The tree is already in SQLite. Selection/expansion/backprop all operate on the database.
3. For performance, maintain an **in-memory cache** of hot nodes (recently visited). Write-back to SQLite periodically (every 100 iterations or on pause).
4. On resume, load the root and its immediate children into cache. Deeper nodes are loaded on-demand during selection.

### Cold start (zero visits)

When a node has zero visits:
- **At MAX nodes:** FPU reduction handles this (see Section 3). Unvisited children are assumed to have Q = parent_Q - fpu_reduction.
- **At CHANCE nodes:** Cold start isn't an issue because we sample from Maia's distribution regardless of visit counts. If the sampled child doesn't exist, we create and evaluate it.
- **Root node with zero visits:** The very first iteration will select the move with the highest prior, expand it, evaluate it, and backpropagate. After 1 iteration we have a value for the root and one child.

### Terminal nodes

Handle checkmate, stalemate, and draw claims:

```
if position is checkmate:
    V = 0.0 if side_to_move is White, else 1.0  (loser's perspective: the side in checkmate lost)
if position is stalemate:
    V = 0.5
if position is draw by repetition / 50-move / insufficient material:
    V = 0.5
```

Terminal nodes are never expanded. Their value is exact and fixed. Mark them with `is_terminal = 1` and `terminal_value = V`. They still accumulate visit counts for proper UCB calculation at parent nodes.

**Terminal node detection:** Check for terminal conditions before querying lc0. This saves an engine eval.

---

## 10. Known Issues and Weaknesses

### 1. Maia accuracy limitations

Maia predicts the single most likely human move with ~52% accuracy. The distribution over all moves is less well-calibrated. If Maia systematically mispredicts for certain position types (e.g., complex tactical positions), our tree will explore the wrong opponent responses.

**Mitigation:** The exploration floor (epsilon) ensures we see some "surprising" responses. Monitor cases where the engine eval of the opponent's position changes dramatically after the opponent's actual move vs. what Maia predicted.

### 2. Horizon effect at shallow depths

If our tree only reaches depth 6-8 (3-4 full moves), we may miss that a "human-exploiting" line leads to a position that's actually bad for us once the opponent gets past the tricky phase. The engine eval at the leaf captures some of this, but deeper tactics may be missed.

**Mitigation:** Ensure sufficient iteration budget. Consider iterative deepening: first do a shallow search (1000 iterations), identify the top 3-5 candidate moves, then do a focused deep search on those.

### 3. Overconfidence against weak play

If Maia predicts the opponent will make mistakes, our search will find that we're "winning" in most lines. This is correct *in expectation*, but:
- Variance is high: the opponent might not make the predicted mistakes.
- We might choose a line that's great if they blunder but terrible if they don't.

**Mitigation:** Track not just expected value but also **worst-case value** among high-probability opponent responses. Consider a "safety margin": if our best move has Q=0.7 but the opponent's top Maia move leads to Q=0.4 for us (they found the right response), while our second-best move has Q=0.65 but the opponent's top response still leaves us at Q=0.6, prefer the second move. This could be formalized as:

```
PracticalScore(a) = (1 - safety) * Q(a) + safety * Q_worst_likely(a)
```

Where `Q_worst_likely(a)` is our value when the opponent plays their best response among moves with Maia probability > 10%. `safety = 0.2` as a starting point.

### 4. Position hash collisions (transpositions)

Chess has transpositions: different move orders reaching the same position. Our tree is a tree, not a DAG. The same position may appear multiple times with different move histories.

**Issues:**
- Wasted computation evaluating the same position twice.
- Maia may give different predictions for the same position reached via different move orders (this is a feature, not a bug -- move-order effects are real in human play).
- Engine eval should be the same (position-dependent only).

**Mitigation:** For engine eval, cache results by EPD (position hash). For Maia, always use the actual move path. Accept some duplication in the tree -- converting to a DAG creates complex backpropagation issues and is not worth the complexity.

### 5. Opening book bias

Starting positions (after 5-15 moves) are in well-known opening theory. Maia's predictions here will cluster on known book moves, which are usually good. The "exploit human mistakes" strategy works less well in known territory.

**Mitigation:** The system is most valuable in positions that are slightly out of book, where the opponent must think for themselves. Consider starting the search from positions just past the opening, or weighting results more heavily from deeper positions.

### 6. lc0 stability under rapid-fire queries

With `nodes=1` evals, we send thousands of sequential UCI queries in quick succession. Known concerns:

- **Memory leaks:** Some lc0 versions leak memory across searches (documented in [lc0 #829](https://github.com/LeelaChessZero/lc0/issues/829)). Even though `nodes=1` creates a tiny search tree, the NNCache accumulates entries (~350 bytes each) across queries.
- **Internal tree management:** lc0 attempts to reuse/reroot its internal search tree between `go` commands. With rapid position changes, this adds unnecessary overhead.
- **TCEC precedent:** Past tournaments had issues with many consecutive fast searches causing RAM exhaustion.

**Mitigations (required):**
1. Set `NNCacheSizeMb` explicitly (e.g., 512) to cap memory growth
2. Send `ucinewgame` periodically (~every 500 queries) to force lc0 to clear its internal tree and NNCache
3. Monitor lc0 process memory usage; restart the process if it exceeds a threshold (reuse the crash-recovery pattern from rust-chess: clear cached engine, recreate on next request)
4. The synchronous UCI protocol provides natural backpressure (we wait for `bestmove` before the next query), so pipe flooding is not a concern

### 7. Computational cost

Even with nodes=1 evals (~15ms per leaf), large searches take minutes. For analysis this is fine, but for real-time play it limits depth.

**Mitigation:**
- Parallelize Maia and engine eval (two lc0 processes).
- Use engine eval caching aggressively (same position via different paths).
- Consider GPU batching if lc0 supports it.
- Prune root children with very low visit counts and very low Q early (smart pruning).

### 8. Draw handling

In practice, many positions are drawn with best play but have non-zero winning chances against humans. A pure WDL model might evaluate a position as "90% draw" and treat it as Q=0.5, missing that the 8% win / 2% loss split makes it a great practical choice.

**Mitigation:** Consider scoring draws partially based on material/positional advantage. Or use a "contempt" adjustment: `V = W/1000 + contempt * D/1000` where `contempt > 0.5` means we believe we can win drawn positions, and `contempt < 0.5` means we prefer to avoid them. For human-exploiting play, `contempt = 0.6` (we believe draws favor us slightly since humans make more mistakes) is a reasonable starting point.

---

## Summary of Key Parameters

| Parameter | Default | Range | Description |
|-----------|---------|-------|-------------|
| cpuct_init | 1.5 | 0.5 - 5.0 | Base exploration constant |
| cpuct_base | 19652 | 1000 - 100000 | Logarithmic growth base |
| cpuct_factor | 1.0 | 0.0 - 5.0 | Logarithmic growth multiplier |
| fpu_reduction | 0.3 | 0.0 - 1.0 | FPU penalty for unvisited children |
| alpha (prior blend) | 0.7 | 0.0 - 1.0 | Engine vs Maia prior weight |
| maia_temperature | 1.0 | 0.5 - 2.0 | Maia distribution temperature |
| maia_floor (epsilon) | 0.01 | 0.0 - 0.05 | Minimum probability per opponent move |
| maia_min_prob | 0.001 | 0.0 - 0.01 | Drop moves below this threshold |
| contempt | 0.6 | 0.0 - 1.0 | Draw scoring (0.5 = neutral) |
| safety | 0.2 | 0.0 - 1.0 | Worst-case weighting in final move selection |
| max_iterations | 5000 | 100 - 100000 | Iteration budget |
| engine_nodes | 1 | 1 - 10000 | lc0 node budget per leaf eval |

---

## Pseudocode: Complete MCTS Iteration

```
function MCTS_ITERATION(root):
    // 1. SELECTION
    path = [root]
    node = root
    while node is expanded and not terminal:
        if node.type == MAX:
            move = argmax_a(Q_stm(node, a) + C(node) * P(node, a) * sqrt(N(node)) / (1 + N(node, a)))
            // For unvisited children: Q_stm = parent_Q_stm - fpu_reduction
        else:  // CHANCE
            move = sample from node.maia_distribution

        if move leads to existing child:
            node = child(node, move)
            path.append(node)
        else:
            // Child doesn't exist yet -- create and expand it
            new_node = create_child(node, move)
            path.append(new_node)
            node = new_node
            break

    // 2. EVALUATION
    if node is terminal:
        V_white = terminal_value(node)
    else:
        // Query both engines
        maia_dist = query_maia(node.move_path)  // full move history
        engine_result = query_engine(node.epd)   // position only, nodes=1

        V_white = engine_result.W/1000 + contempt * engine_result.D/1000

        // Store on node
        if node.type == MAX:
            for each legal move m:
                P(node, m) = alpha * engine_result.policy(m) + (1-alpha) * maia_dist(m)
        else:  // CHANCE
            node.maia_distribution = apply_temperature_and_floor(maia_dist)
            for each legal move m:
                P(node, m) = node.maia_distribution(m)

    // 3. BACKPROPAGATION
    for each node in path:
        node.N += 1
        node.W_sum += V_white
        node.Q = node.W_sum / node.N

    // 4. PERSIST (periodically)
    if iteration_count % 100 == 0:
        flush_cache_to_sqlite()
```

---

## References and Further Reading

- [AlphaZero PUCT formula and parameter tuning](https://medium.com/oracledevs/lessons-from-alphazero-part-3-parameter-tweaking-4dceb78ed1e5)
- [Lc0 PUCT and cpuct discussion](https://github.com/LeelaChessZero/lc0/issues/694)
- [Lc0 search options and defaults](https://lczero.org/dev/wiki/lc0-options/)
- [Lc0 technical explanation](https://lczero.org/dev/wiki/technical-explanation-of-leela-chess-zero/)
- [Lc0 WDL evaluation](https://lczero.org/blog/2020/04/wdl-head/)
- [Maia Chess: human-like neural network chess engine](https://www.maiachess.com/)
- [Maia-2: Unified Model for Human-AI Alignment in Chess](https://arxiv.org/html/2409.20553v1)
- [Aligning Superhuman AI with Human Behavior (Maia KDD 2020)](https://www.cs.toronto.edu/~ashton/pubs/maia-kdd2020.pdf)
- [Combining Prediction of Human Decisions with ISMCTS](https://arxiv.org/abs/1709.09451)
- [MCTS with opponent modeling (thesis)](https://www.ai.rug.nl/~mwiering/Tom_van_der_Kleij_Thesis.pdf)
- [Monte Carlo Tree Search: review of modifications and applications](https://link.springer.com/article/10.1007/s10462-022-10228-y)
- [MCTS Wikipedia](https://en.wikipedia.org/wiki/Monte_Carlo_tree_search)
- [Chessprogramming: Leela Chess Zero](https://www.chessprogramming.org/Leela_Chess_Zero)
