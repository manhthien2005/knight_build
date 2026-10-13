---
name: knight-implementation
description: Implement an authorized Knight Zeus Java 6 or Rust change on a pinned branch. Use when implementing focused bug fixes, J2ME native UI/packet or state-machine changes, Rust tool changes, tests and scoped refactoring after source tracing.
---
# Knight Implementation

**Goal:** Smallest complete working change at the verified seam. Preserve existing source, controls and operator state.

## Entry gate

Require approved goal/scope, actual base/HEAD ref, affected module rules and known-good behavior. For bug/ABI uncertainty, invoke `knight-source-trace` first. If the GitHub branch omits current Windows worktree changes, stop `BASELINE_MISMATCH`: do not rebuild a “latest” feature from remote main by guessing.

## One vertical slice

1. Identify producer/caller/native consumer and relevant tests; list the few files expected to change.
2. Create or identify one negative test **capable of failing on the reported symptom**, ideally through `bossHuntTick` / native UI lifecycle instead of a private helper. Demonstrate RED where feasible; if live-only, state the offline limitation.
3. Implement one minimal fix. Reuse client native controls and Rust data contracts; avoid packet invention, speculative framework layers or unrelated cleanup.
4. Check cancellation, UI priority, re-entrant ticks, stale menu IDs, exact command pointer, timeouts and ownership cleanup **only where they matter to this change**. Fail closed on uncertainty.
5. Run the focused affected test(s), native probe if applicable, relevant compile and bytecode-major validation. Don't claim a green test without the actual exit status/output. Expand regression only if the diff has cross-cutting effects.
6. Inspect diff for accidental production/vendor/deployment edits, unrelated changes, hardcoded accounts/credentials or mutated snapshots. Never run the game to “see if it works” as a coder.

## Deliverable to Reviewer

```text
TASK_ID / BASE_SHA / HEAD_SHA / dirty-state warning
ROOT_CAUSE / impacted native or Rust boundary
FILES_CHANGED and justification
TEST_RED evidence or NOT_RUN reason
BUILD and focused checks (exact command/results)
CANDIDATE_JAR_SHA256 + Java6 major 50 if built
UNVERIFIED assumptions, regression risks and operator steps
VERDICT: READY_FOR_INDEPENDENT_REVIEW | BLOCKED
```

A pushed feature branch/PR may be prepared only when specifically authorized. Do not merge or deploy. Never say “fixed live” from simulated assertions.
