---
name: knight-runtime-acceptance
description: Safely stage and manually validate a pinned Knight Zeus candidate on an isolated Windows MicroEmulator profile. Use when staging a JAR with SHA-256 guard and launcher DryRun or checking NPC dialogs, Boss Hunt, movement, combat, snapshots and operator emergency stop.
---
# Knight Runtime Acceptance

**Role:** Runtime evidence collector; the human operator controls game/account actions. This skill is **not** authorization to launch, log in, interact, buy, move, or send packets.

## Safety preflight before staging

1. Pin approved candidate, source commit, Java 6 major 50, expected SHA-256 and *exact* isolated profile. Confirm no ambiguous local/remote baseline.
2. Verify the previous candidate is backed up. Confirm client is **not running** and auto modules are disabled/removed before replacement; if not, return `BLOCKED` without killing processes.
3. Stage only in the approved isolated profile, update checksum launcher guard and run PowerShell syntax/DryRun where supported. Production JAR, Railway deploy, account RMS and credentials stay untouched.
4. Operator alone manually launches/logs in. Do not infer a successful startup from a DryRun.

## Live execution

- Define **one** target action boundary, expected status transition and stop condition (e.g. “Alisama foreground dialog -> native Boss menu”; do not bundle full patrol/combat in the same acceptance).
- Before enabled controls, show operator an emergency-stop method for that *exact* profile/config and explain that closing the isolated client is the fallback.
- Record prestate, live UI observation, status JSON, timestamps, JAR hash and command/action count if available. Redact user/account information.
- On unexpected dialogs, transaction surfaces, wrong actor/NPC, stale schedule, repeated command or control ownership leak: immediately instruct the operator to stop; do not manually click on behalf of the bot to hide a gap.
- If failed, return to `knight-source-trace` with the smallest discriminating observation. Do not perform blind second/third retest without a changed hypothesis or new diagnostic.

## Acceptance output

```text
CANDIDATE_COMMIT / JAR_SHA / PROFILE / CONFIG
PREFLIGHT / DRYRUN / OPERATOR_LAUNCH
TARGET_BOUNDARY / EXPECTED_RESULT / OBSERVED_RESULT
STATUS_PRE / STATUS_POST / timestamp
STOP_ACTION / recovery state
VERDICT: LIVE_PASS | LIVE_FAIL | NOT_RUN | BLOCKED
NEXT: reviewer acceptance or exactly one targeted trace
```

Never label the overall feature “DONE” until every required live behavior has operator-confirmed evidence. Static/CI proofs remain useful but distinct. Production changes require separate explicit authorization.
