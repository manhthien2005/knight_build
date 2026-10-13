# Repo map and baseline discipline

This document maps verified **remote main** landmarks, not a claim about the operator's uncommitted Windows worktree.

## Remote main snapshot inspected for this skill pack

- Repository: https://github.com/manhthien2005/knight_build
- Inspected starting main HEAD: `18ae65c2d95653d299d1c505db98c22d52af46cc`.
- `mod/zeus/src/Zeus.java`: Java client extension; `mod/zeus/tools/PatchZeus.java`: entry-point bytecode injection; `mod/zeus/build-zeus-jar.sh`: Java 6-targeted build recipe; `mod/zeus/tools/*Test.java`: relevant test harnesses.
- `tool/Cargo.toml`: Rust 2024 / Rust 1.98 workspace with `zeus-core`, `zeus-ui`, `zeus-agent`.
- `vendor/game/Zeus_Knight.jar` and `vendor/game/zeus-jar.json`: runtime artifact metadata. Treat as protected production artifacts, not scratch build destinations.
- `bin/`, `Dockerfile`, `railway.json`: Railway/MicroEmulator runtime; existing root README describes Docker/noVNC deployment.
- `mod/potato/`: separate module. Do not assume part of the Zeus change surface.

## Important baseline warning

Local worktree `feature/minh-boss-r2-3a-canonical` and local candidate R4.3.x were reported by the operator, but have NOT been confirmed on the remote main snapshot above. In particular, R4.3.7 passed source/probe tests but the operator later observed a LIVE failure at the Alisama foreground 'Mua bán' dialog; R4.3.9 was assigned for investigation, not independently accepted here.

Therefore:
- Do not call the remote main snapshot “latest Boss Hunt”; do not implement an R4.3.x follow-up directly on it without reconciling local changes.
- Before coding, capture exact local HEAD, dirty status, candidate checksum, current source tree, and remote ref. Avoid `reset` or `clean`; preserve backup/uncommitted work.
- If sources disagree, write `BASELINE_MISMATCH` and request reconciliation. A Docs/Skills branch can be reviewed independently, but its existence does not sync the local Boss Hunt implementation.
- Re-check these landmarks and replace the pinned baseline record after deliberate sync; do not silently advance the recorded SHA.

## Domain vocabulary

`NATIVE`: behavior emitted by real vanilla/ModByTminh client objects, not a mock. `ABI`: exact types, method pointers, call order and packet fields. `OWNER`: component that acquired movement/fire/focus and must release it. `LIVE`: human-observed client behavior with exact JAR/config/profile and time. `PROBE`: targeted executable check that may still omit UI lifecycle/context.
