# Implementation Plan: Dungeon UTC+7 Fail-Closed and Test Hardening

**Task:** `DUNGEON_DAILY_UTC7_R1_1_FAIL_CLOSED_AND_TEST_HARDENING`  
**Base Head:** `d2a128f9a70353e208d9df31b59aa73458f93f13`  
**Branch:** `feature/dungeon-daily-utc7-window`  

---

## 1. Java Hardening (`mod/zeus/src/Zeus.java`)
- Add `isDungeonUnscheduled()`: `dungeonStartMin == -1 && dungeonEndMin == -1`.
- Retain `isDungeonScheduled()`: `dungeonStartMin >= 0 && dungeonEndMin <= 1439 && dungeonStartMin < dungeonEndMin`.
- Update `dungeonScheduleDue`:
  - Overload `dungeonScheduleDue(int min, int dateKey)` for pure deterministic evaluation.
  - Fail closed if neither unscheduled nor scheduled.
  - Fail closed if `min < 0` or `dateKey < 0`.
- Update `dungeonIdle`:
  - Extract `dungeonIdle(int min, int dateKey)` and `dungeonIdle(long epochMillis)`.
  - In `dungeonIdle(min, dateKey)`:
    - If `isDungeonUnscheduled()`: run bounded by `dungeonMaxRuns`, stopping with reason 4 when reaching limit.
    - If `isDungeonScheduled()`: reset daily quota when entering window on new day; stay idle if outside window or quota reached.
    - If neither (invalid schedule): fail closed (stay in `DN_IDLE`, never start run).
  - Production `dungeonIdle()` delegates to `dungeonIdle(System.currentTimeMillis())`.
- Update `acceptControl`:
  - Reject `dungeon.max == 0`: domain is strictly `-1` or `1..=10`.
- Update `DN_FAILURE` and `dungeonDone`:
  - Ensure only `isDungeonUnscheduled()` stops with reason 4 on limit reached; scheduled mode remains in `DN_IDLE`.

---

## 2. Rust Hardening
### `tool/crates/zeus-core/src/control.rs`
- In `ControlSettings::clamped()`:
  - Do not silently normalize or repair `dungeon_start_min` and `dungeon_end_min`.
  - Preserve valid windows and exact `-1, -1` as-is. Leave invalid programmatic values unchanged so serialization fails closed downstream.
  - In `dungeon_max`: clamp to `1..=DUNGEON_RUNS_MAX` if not `-1`.
- In `parse_settings`:
  - Reject `dungeon_max == 0`. Valid domain is `-1` or `1..=DUNGEON_RUNS_MAX`.
  - Add explicit unit tests for invalid schedule rejection, `dungeon_max == 0` rejection, and `clamped()` preservation.

### `tool/crates/zeus-ui/src/model.rs`
- In `UiControl::clamped()`:
  - Remove silent normalization/repair of `dungeon_start_min` and `dungeon_end_min`.
  - Preserve exact values unchanged.
  - Add unit test verifying `UiControl::clamped()` does not normalize invalid schedule windows.

---

## 3. Java Test Hardening (`mod/zeus/tools/DungeonStateMachineTest.java`)
- Update Test 78:
  - Prove actual `dungeonIdle` execution with Day 1 quota exhaustion (stays `DN_IDLE`).
  - At Day 2 20:00 start boundary, execute `dungeonIdle(1200, 2026278)`:
    - `dungeonRuns` resets to 0.
    - `dungeonConsecutiveFails` resets to 0.
    - `dungeonScheduleDateKey` advances to Day 2.
    - Transitions from `DN_IDLE` to `DN_ROUTING` without toggling `dungeon.on`.
  - After `endMin` on Day 2, invoke `dungeonIdle(1215, 2026278)`: remains `DN_IDLE`, no new run begins.
  - Active run in `DN_COMBAT`/`DN_ROUTING` after `endMin` is not cancelled.
- Correct Test 83 (Clock / Conversion Failure):
  - Remove claim that `epochMillis=-1` is clock failure.
  - Test `dungeonScheduleDue(-1, 2026278)` and `dungeonScheduleDue(1200, -1)` fail closed.
  - Test `dungeonIdle(-1, 2026278)` and `dungeonIdle(1200, -1)` fail closed.
- Add Regression Tests:
  - Test exact `-1/-1` unscheduled is immediate.
  - Test invalid `-1/500`, `500/-1`, `start >= end`, out-of-range minute pairs in `dungeonScheduleDue` and `dungeonIdle` fail closed.
  - Test `acceptControl` rejects `dungeon.max == 0`.

---

## 4. Rebuild Runtime JAR & Runtime Pins
- Run `mod/zeus/sync-build-merged.sh` in WSL.
- Compute new JAR SHA256 and size.
- Update `vendor/game/zeus-jar.json`.
- Update `Dockerfile` pinned SHA.
- Update `AUTO_DUNGEON_COMPATIBLE_JAR_SHA256` in `tool/crates/zeus-agent/src/supabase_rest.rs`.

---

## 5. Verification & Git
- Run all test suites:
  - Java: `DungeonStateMachineTest`, `VisualQoLTest`, `DialogGateTest`, `CharacterSlotTest`, `ReconnectTravelTest`.
  - Rust: `cargo test -p zeus-core control`, `cargo test -p zeus-ui`, `cargo test -p zeus-agent`.
- Run `git diff --check`.
- Explicit git staging.
- Single commit: `fix(dungeon): harden UTC+7 schedule fail-closed semantics`.
- Push branch with upstream tracking.
- Output final single JSON object.
