---
name: knight-independent-review
description: Independently review Knight Zeus/Rust implementation, PR, diff, candidate JAR, native ABI changes or tests. Use when reviewing a pinned diff or JAR before staging to detect defects, source-vs-runtime gaps, regressions and weak PASS evidence.
---
# Knight Independent Review

**Role:** Read-only Tech Lead. Do not implement the Coder's patch or accept its verdict as evidence. Pin and inspect the exact commit/artifact rather than the original chat narrative.

## Review sequence

1. Verify branch/base/head and all changed paths; capture diff and file versions. Stop on `BASELINE_MISMATCH` or artifacts not matching commit/candidate. Distinguish repo source from local uncommitted code.
2. **Spec axis:** compare expected behavior to actual entry/call chain, missing requirements and unintended scope. Read touched callers/consumers, not diff alone.
3. **Native and safety axis:** exact J2ME ABI, packet dispatch, callback order, UI foreground priority, disabled/retry gates, stale timing, movement/fire ownership and release. Rust transport and deployment impact when relevant.
4. **Test axis:** run independently when possible and verify the test can fail on the *real bug*. Inspect fakes/mocks, especially raw menu items versus native initialized `iCommand`. A test passed by calling a helper does not prove the production tick calls it.
5. **Change-quality axis:** prefer a smaller or already-native solution; reject only actionable duplication/complexity, not stylistic taste. Verify no unexpected vendor/prod edits, binary drift or secret leakage.
6. **Decision axis:** mark each reported check as independently executed, source-inspected, agent-claimed or `NOT_RUN`. Classify SOURCE/BUILD/TEST/NATIVE PROBE versus LIVE; if a previous live attempt still fails, never pass live.

## Findings and verdict

Every finding: `ID | SEVERITY (BLOCKER/MAJOR/MINOR) | path:location | failing scenario | evidence | smallest remedy`. Do not invent line numbers, commands or packet fields.

Return `REWORK` for proven code/test blocker, `BLOCKED` for missing baseline/binary evidence, or `REVIEW_PASS_STATIC_ONLY` when no blockers exist and runtime remains unverified. A static PASS authorizes preparing operator staging only when explicitly approved, never a production merge.

Re-review only changed findings plus affected surfaces on subsequent iterations; avoid the “full suite after every edit” trap.
