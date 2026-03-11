# Known Issues and Weaknesses

## 1. Maia accuracy limitations

Maia predicts the single most likely human move with ~52% accuracy. The distribution over all moves is less well-calibrated. If Maia systematically mispredicts for certain position types (e.g., complex tactical positions), our tree will explore the wrong opponent responses.

**Mitigation:** The exploration floor (epsilon) ensures we see some "surprising" responses. Monitor cases where the engine eval changes dramatically after the opponent's actual move vs. what Maia predicted.

## 2. Horizon effect at shallow depths

If our tree only reaches depth 6-8 (3-4 full moves), we may miss that a "human-exploiting" line leads to a position that's actually bad for us once the opponent gets past the tricky phase. The engine eval at the leaf captures some of this, but deeper tactics may be missed.

**Mitigation:** Ensure sufficient iteration budget. Consider iterative deepening: first do a shallow search (1000 iterations), identify the top 3-5 candidate moves, then do a focused deep search on those.

## 3. Overconfidence against weak play

If Maia predicts the opponent will make mistakes, our search will find that we're "winning" in most lines. This is correct *in expectation*, but:
- Variance is high: the opponent might not make the predicted mistakes.
- We might choose a line that's great if they blunder but terrible if they don't.

**Mitigation:** The safety parameter blends expected value with worst-case value among likely opponent responses:

```
PracticalScore(a) = (1 - safety) * Q(a) + safety * Q_worst_likely(a)
```

Where `Q_worst_likely(a)` is our value when the opponent plays their best response among moves with Maia probability > 10%.

## 4. Transpositions

Chess has transpositions: different move orders reaching the same position. Our tree is a tree, not a DAG. The same position may appear multiple times with different move histories.

**Issues:**
- Wasted computation evaluating the same position twice.
- Maia may give different predictions for the same position reached via different move orders (this is a feature, not a bug -- move-order effects are real in human play).
- Engine eval should be the same (position-dependent only).

**Mitigation:** Cache engine evals by EPD. Cache Maia by move path. Accept tree duplication -- converting to a DAG creates complex backpropagation issues and is not worth the complexity.

## 5. Opening book bias

Starting positions (after 5-15 moves) are in well-known opening theory. Maia's predictions here will cluster on known book moves, which are usually good. The "exploit human mistakes" strategy works less well in known territory.

**Mitigation:** The system is most valuable in positions slightly out of book, where the opponent must think for themselves. Consider starting the search from positions just past the opening.

## 6. Computational cost

Even with nodes=1 evals (~15ms per leaf), large searches take minutes.

**Mitigation:**
- Parallelize Maia and engine eval (two lc0 processes).
- Use engine eval caching aggressively (same position via different paths).
- Prune root children with very low visit counts and very low Q early (smart pruning).

## 7. Draw handling

Many positions are drawn with best play but have non-zero winning chances against humans. A pure WDL model might evaluate a position as "90% draw" and treat it as Q=0.5, missing that the 8% win / 2% loss split makes it a great practical choice.

**Mitigation:** The contempt parameter (`V = W/1000 + contempt * D/1000`) addresses this. With contempt=0.6, drawn positions are scored slightly in our favor, reflecting that humans make more mistakes in drawn positions.

## References

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
