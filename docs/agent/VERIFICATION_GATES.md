# Verification gates: evidence before verdict

A change's success is limited to the strongest gate actually supported by evidence. No fixed test count proves a GUI feature by itself.

| Gate | Evidence needed | Example verdict |
| --- | --- | --- |
| G0 BASELINE | Exact repo/branch/head, local dirty state, artifact hash and expected native version | PASS / MISMATCH |
| G1 SOURCE | Traced entry, callers, UI lifecycle, native ABI and change impact | PASS / UNVERIFIED |
| G2 PATCH | Minimal scoped diff, compile result and focused red-capable regression | PASS / REWORK |
| G3 REVIEW | Independent spec + ABI + safety + tests + scope review against pinned commit | APPROVED / REWORK / BLOCKED |
| G4 STAGE | Isolated profile backup, SHA guard, no running client, feature disabled, launcher dry-run | READY / BLOCKED |
| G5 LIVE | Human confirms target UI/action/state in client using exact staged JAR; stop on unexpected behavior | PASS / FAIL / NOT_RUN |
| G6 MERGE/PROD | Explicit owner approval, protected main/production unchanged until then | APPROVED / BLOCKED |

## Regression quality rule

A test for a previously observed bug must exercise a boundary capable of producing **that exact failure**. If the failure was blocked by a foreground `MsgDialog`, assert behavior with foreground dialog and a stale/visible background menu through the actual production tick path; mocking a helper alone cannot prove it. For server-menu dispatch, verify native action/pointer setup, ID/row values, fail-closed paths and non-duplication. If a meaningful red run cannot be produced offline, say why and require G5 instead of inventing confidence.

## Minimum evidence envelope

```text
task_id:
repository / branch / base_sha / head_sha:
observed_goal_or_failure:
changed_paths:
source_and_ABI_evidence:
commands_run_and_outputs:
checks_not_run_and_reason:
jar_sha256 / classfile_major (if Java):
review_findings:
stage_profile / candidate_running / feature_enabled (if staging):
live_observation_and_status (if applicable):
verdict: SOURCE_ONLY | BUILD_PASS | TEST_PASS | REVIEW_PASS | LIVE_PASS | REWORK | BLOCKED
next_action_and_authority:
```

Keep findings concise: severity, path and location, failing scenario, evidence, smallest remedy. Separate tests **executed independently** from agent-reported tests. Never fabricate CI. A G3 approval does not authorize G4/G5 or G6.

## Runtime isolation

Before replacing an isolated JAR: confirm game client is closed, Boss Hunt disabled, previous JAR backed up, expected SHA matches, launcher guard updated and dry-run succeeded. Preserve production JAR, credentials, RMS and external services. G5 actions are user-operated and time bounded; capture only redacted status fields, never login material.

## Escalation

After repeated green tests with the same live symptom, stop widening tests. Identify the missing runtime seam and request one focused capture/probe. If root cause remains uncertain, output hypotheses with specific disconfirming evidence rather than claiming a fix.
