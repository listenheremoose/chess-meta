# MCTS Algorithm

Chess-meta uses a modified Monte Carlo Tree Search to find moves that maximize practical winning chances against human opponents. The tree has two types of internal nodes:

- **MAX nodes ("our" turn):** Select moves via UCB/PUCT (exploration/exploitation).
- **CHANCE nodes ("opponent" turn):** Sample moves proportional to Maia's predicted human-play distribution.

Leaf evaluation uses lc0's engine eval (not rollouts). This is essentially an **Expectimax-MCTS hybrid**: maximizing at our nodes, taking expectations at opponent nodes.

---

## 1. PUCT Formula (MAX Nodes)

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

### Why PUCT over UCB1

UCB1 (`Q + C * sqrt(ln(N) / n_i)`) treats all unvisited moves equally. PUCT uses the neural network prior `P(s,a)` to focus early exploration on promising moves. Since we have strong priors from lc0's policy head, PUCT is strictly superior here -- it avoids wasting evaluations on clearly bad moves before visiting good ones.

---

## 2. CHANCE Nodes (Opponent's Turn)

CHANCE nodes do **not** use UCB at all. They are not decision nodes -- they model the stochastic environment of human play. Selection is by sampling from Maia's distribution.

### Maia distribution processing

Apply the following pipeline to Maia's raw probabilities:

**Step 1: Drop negligible moves.**
- Remove moves with P_maia < 0.001 (0.1%).

**Step 2: Apply temperature smoothing.**

```
P_adjusted(a) = P_maia(a)^(1/T) / sum_b(P_maia(b)^(1/T))
```

With `T = 1.0` (no smoothing) as the default.

**Step 3: Optional exploration floor.**
Ensure every remaining move has at least `epsilon = 0.01` probability (1%), then renormalize.

```
P_final(a) = max(P_adjusted(a), epsilon)
// then renormalize so sum = 1
```

### When Maia gives 80% to one move

This is **correct behavior** -- if a human plays one move 80% of the time, our expected score should weight that branch 80%. The exploration floor (epsilon) ensures we don't have zero information about other responses.

### Caching Maia distributions

Store the Maia distribution on the CHANCE node when it's first expanded. This avoids re-querying Maia on every visit. Since CHANCE nodes don't change their distribution, this is safe.

---

## 3. Selection

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
        child = sample from Maia distribution
        if child not yet in tree:
            expand child, return child
        return SELECT(child)
```

**Opponent nodes create children on-demand.** Since we sample from Maia's distribution, we may sample a move whose child node doesn't exist yet. In that case, we create it and treat it as the leaf for this iteration.

**No UCB at opponent nodes -- and this is correct.** We are modeling the opponent as a stochastic process with a known distribution (Maia). This makes opponent nodes equivalent to "chance nodes" in expectimax / stochastic game trees.

---

## 4. Expansion

When selection reaches a leaf node, expand **one child only** (the selected move).

### Expansion procedure

1. Selection reaches leaf node `L`.
2. Evaluate `L` using lc0 engine eval to get `V(L)` (the value) and `P_engine(L, *)` (the policy).
3. Query Maia for `P_maia(L, *)` (human prediction distribution).
4. Store on node `L`: the blended prior for each legal move, the Maia distribution, and the eval value.
5. Backpropagate `V(L)` up the tree.
6. `L` is now an internal node. Its children are created lazily when selected/sampled in future iterations.

### First Play Urgency (FPU)

When a MAX node has unvisited children, use **FPU reduction**:

```
Q_fpu(s, a) = Q(s) - fpu_reduction
```

Where `Q(s)` is the parent's mean value. This ensures unvisited children are assumed slightly worse than the current average, encouraging depth-first behavior while still allowing exploration via the prior.

---

## 5. Evaluation

### Engine eval budget

Use `go nodes 1` (just the neural network forward pass, ~5-10ms). Our MCTS *is* the search — we don't need lc0 to also search internally.

Both engine and Maia evals take ~5-10ms each. Total per leaf: ~10-20ms. These can be parallelized across two lc0 processes.

### Output format: WDL

Configure lc0 with `UCI_ShowWDL true`. Convert to a value for backpropagation:

```
V = W/1000 + contempt * D/1000
```

### Maia output

Maia is queried with `go nodes 1`. We need the **policy distribution** (move probabilities), not the value.

**Critical: Maia requires the full move sequence from game start.** Always send `position startpos moves e2e4 e7e5 ...` with the full move history.

---

## 6. Backpropagation

### Value representation

All values are stored as **expected score from White's perspective** in [0, 1]. This avoids sign-flip confusion.

At each node, store:
- `N` = visit count
- `W_sum` = sum of all backpropagated values (from White's perspective)
- `Q = W_sum / N` = mean value (from White's perspective)

### Backpropagation procedure

```
function BACKPROPAGATE(path, V_white):
    for each node in path (leaf to root):
        node.N += 1
        node.W_sum += V_white
        node.Q = node.W_sum / node.N
```

### Perspective handling during selection

When selecting at a MAX node, convert Q to side-to-move perspective:

```
Q_stm(s, a) = if white_to_move(s) then Q(child) else 1 - Q(child)
```

Storing values from a fixed perspective (White) and converting during selection is cleaner for persistent storage in SQLite.

---

## 7. Terminal Nodes

```
if position is checkmate:
    V = 0.0 if side_to_move is White, else 1.0
if position is stalemate:
    V = 0.5
if position is draw by repetition / 50-move / insufficient material:
    V = 0.5
```

Terminal nodes are never expanded. Their value is exact and fixed. They still accumulate visit counts for proper UCB calculation at parent nodes.

**Terminal node detection:** Check for terminal conditions before querying lc0 to save an engine eval.

---

## 8. Convergence and Stopping

**Rule of thumb:** For a position with B candidate moves at our turn:
- **100 * B iterations** for a rough ordering
- **500 * B iterations** for a reliable ordering
- **2000 * B iterations** for convergence

### Convergence metrics

1. **Best move stability:** If the best move hasn't changed in the last 20% of iterations, likely converged.
2. **Q-value stability:** When `max_Q - second_Q` changes by less than 0.01 over 500 iterations, the ranking is stable.
3. **Visit distribution entropy:** Decreasing entropy means the search is concentrating.
4. **Running best-move Q-value:** When it plateaus, stop.

### Stopping criteria

```
STOP when ANY of:
  - Iteration count >= max_iterations (default 5000)
  - Wall clock time >= max_time
  - Best move unchanged for last 30% of iterations AND Q gap > 0.03
  - User requests pause
```

---

## 9. Tree Reuse

If we searched position A and then want to search position B = A + move:

1. Find the child node corresponding to the move.
2. Re-root the tree at that node.
3. All statistics remain valid (values stored from White's perspective, Maia distributions depend on move history which hasn't changed).
4. Prune other branches from the database.

**Invalidation:** If Maia's model is updated (e.g., different rating target), discard the tree.

---

## Complete MCTS Iteration Pseudocode

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
            new_node = create_child(node, move)
            path.append(new_node)
            node = new_node
            break

    // 2. EVALUATION
    if node is terminal:
        V_white = terminal_value(node)
    else:
        maia_dist = query_maia(node.move_path)  // full move history
        engine_result = query_engine(node.epd)   // position only, nodes=1

        V_white = engine_result.W/1000 + contempt * engine_result.D/1000

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
