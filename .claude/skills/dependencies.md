---
name: Dependencies
description: Policy on adding and managing crate dependencies
globs: Cargo.toml
---

# Dependencies

## Philosophy

Minimal — avoid dependencies unless they save significant effort. Prefer writing small utilities over pulling in a crate.

## Vetting Criteria

Before adding a crate, it must be:

- Widely used (high download count, active community)
- Actively maintained (recent commits, responsive issues)

## Approved Dependencies

- `iced` — UI framework
- `shakmaty` — chess position logic, move generation, game-over detection
- `rusqlite` — SQLite for caching engine evals, Maia policies, and MCTS tree persistence
- `serde` + `serde_json` — serialization for cache storage and config
- `rand` — weighted random sampling for Maia distributions at CHANCE nodes
- `criterion` — benchmarking (dev-dependency)
- `insta` — snapshot testing (dev-dependency)

All other dependencies require explicit justification.

## Dependency Updates

Pin exact versions. Update manually and deliberately — review changelogs before bumping.
