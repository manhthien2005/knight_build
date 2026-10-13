---
name: knight-source-trace
description: Investigate Knight/Zeus source, ModByTminh v4.0.3 native ABI, J2ME Menu2/MsgDialog, packets, NPC flows, Rust-to-Java transport, state machine, or live bugs. Use before proposing a fix when code behavior or integration is uncertain.
---
# Knight Source Trace

**Goal:** Prove the smallest actual failing or requested call chain. Do not code in this mode unless a separate implementation task authorizes it.

## Trigger and scope

Use for bug reports, new feature feasibility, unclear methods, packet/UI mismatch, live failures after green tests, and compatibility changes. Do not use for docs-only edits with no source behavior impact. Read root `AGENTS.md` and the touched module's `AGENTS.md`.

## Investigation sequence

1. Pin actual source ref, candidate JAR hash, vanilla/ModByTminh reference and observed symptom. Distinguish local worktree, remote GitHub, and staged profile. If not aligned: `BASELINE_MISMATCH`.
2. Name the concrete entry point, event/tick thread, state gate, native method, UI/packet boundary, observer/status output, and exit/cleanup. Walk **caller → mutation → native consumer → observed result**. Mark any unavailable node `UNVERIFIED`.
3. Prioritize an executable, red-capable probe or recorded operator evidence. If no local game access, define a minimal capture request and do not claim to have run it.
4. Inspect native bytecode/class signatures **and order of initialization**, not just source names. For dialogs compare foreground `GameCanvas.currentDialog` versus `Menu2.isShowMenu`; inspect whether each `iCommand` actually has an action/pointer when `perform()` is called.
5. Rank falsifiable hypotheses only after separating proven facts from assumptions. Seek the smallest observation that discriminates them, avoiding broad logging or repeated full regression.
6. Identify fail-closed path, ownership release, debounce, cancellation, retries and expected changes to UI/packet status if relevant.

## Output

```text
BASELINE: ref/JAR/profile and any mismatch
OBSERVED: exact behavior and evidence
TRACE: entry -> guards -> native boundary -> outcome (locations)
PROVEN: facts from source/bytecode/runtime
HYPOTHESES: each with disconfirming check
ROOT_CAUSE: VERIFIED | PROBABLE | UNKNOWN
SMALLEST_REPRO: command or operator capture; NOT_RUN if unavailable
IMPACT: caller/test/artifact/rollback
NEXT: minimal implementation task OR one targeted diagnostic
```

Do not equate an example from generic Java with verified J2ME ABI. Never infer server schedule semantics from a guessed GMT+7 clock. See `docs/agent/VERIFICATION_GATES.md`.
