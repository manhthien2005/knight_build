# Daily UTC+7 Dungeon Time Window Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the legacy half-hour VM-local dungeon schedule (`dungeon.schedule`) with an exact daily UTC+7 time window (`dungeon.startMin` and `dungeon.endMin`), updating Java, Rust core, agent, Windows UI, protocol versions, and rebuilding the runtime JAR.

**Architecture:** 
- In Java (`Zeus.java`), replace `dungeonSchedule` with `dungeonStartMin` and `dungeonEndMin`. Provide deterministic UTC+7 minute-of-day and date key conversion helpers using `java.util.TimeZone.getTimeZone("GMT+07:00")` explicitly without depending on VM default timezone. Clock failures fail closed.
- Daily quota resets `dungeonRuns` and `dungeonConsecutiveFails` to 0 when entering a new eligible UTC+7 day/window, while reaching `dungeon.max` remains idle for the day instead of permanently stopping future days.
- In Rust (`zeus-core`, `zeus-agent`, `zeus-ui`), bump `CTL_VERSION` to 15, `CTL_KEY_COUNT` to 38, replace `dungeon.schedule` with signed minute fields (`i16`), update Windows UI with exact HH:mm edit fields with UTC+7 validation, rebuild `Zeus_Knight.jar`, and update runtime pins.

**Tech Stack:** Java (J2ME / Java 6 compatible), Rust 2021, Windows Win32 API.

---

## Global Constraints
- `ctl_version`: 15
- `ctl_key_count`: 38
- `snapshot_version`: 6 (unchanged)
- `snapshot_key_count`: 49 (unchanged)
- Timezone: `GMT+07:00` (UTC+7)
- Start minute: inclusive (`startMin <= min`)
- End minute: exclusive (`min < endMin`)
- Valid window: `0 <= startMin < endMin <= 1439`
- Unscheduled sentinel: `startMin == -1 && endMin == -1`
- Clock failure: `FAIL_CLOSED`
- Legacy / mixed v14 control files: `FAIL_CLOSED`
- Explicit staging only (`no git add .`, `no git add -A`)
- No rebase, no squash, no force push, do not merge

---

### Task 1: Update Java Control Schema, Timezone Conversion & State Machine in `Zeus.java`

**Files:**
- Modify: `mod/zeus/src/Zeus.java`
- Modify: `mod/zeus/tools/VisualQoLTest.java`

- [ ] **Step 1: Update CTL_KEYS, K_* indices, and version constants**
  - Change `CTL_VERSION` from 14 to 15.
  - In `CTL_KEYS`, replace `"dungeon.schedule"` with `"dungeon.startMin"`, `"dungeon.endMin"`.
  - Update `K_*` indices:
    - `K_DUNGEON_ON = 32;`
    - `K_DUNGEON_MAX = 33;`
    - `K_DUNGEON_START_MIN = 34;`
    - `K_DUNGEON_END_MIN = 35;`
    - `K_UI_EFFECTS = 36;`
    - `K_UI_HIDE_PLAYERS = 37;`
    - Remove `K_DUNGEON_SCHED`.

- [ ] **Step 2: Add UTC+7 deterministic clock helpers and schedule due logic**
  - Add `dungeonStartMin = -1;`, `dungeonEndMin = -1;`, `dungeonScheduleDateKey = -1;`.
  - Remove `dungeonSchedule` and `dungeonScheduleDay`.
  - Add `dungeonMinuteOfDayUtc7(long epochMillis)` and `dungeonDateKeyUtc7(long epochMillis)`.
  - Add `isDungeonScheduled()`.
  - Add `dungeonScheduleDue(long epochMillis)` and `dungeonScheduleDue()`.

- [ ] **Step 3: Update control parsing, validation, and state machine transitions**
  - In `acceptControl`: validate `startMin` and `endMin` (-1/-1 or 0 <= start < end <= 1439). Fail closed on any other combination.
  - In `dungeonIdle()`: evaluate UTC+7 dateKey and minute window. On new day/window, stamp dateKey and reset `dungeonRuns = 0`, `dungeonConsecutiveFails = 0`. If quota reached in scheduled mode, remain in `DN_IDLE`.
  - In `dungeonDone()`: if `dungeonRuns >= dungeonMaxRuns` in scheduled mode, stay in `DN_IDLE` instead of `DN_OFF`.
  - In `dungeonReset()`: reset `dungeonScheduleDateKey = -1;`.
  - In `VisualQoLTest.java`: update control text to v15 with `dungeon.startMin=-1` and `dungeon.endMin=-1`.

---

### Task 2: Expand `DungeonStateMachineTest.java` with Comprehensive Schedule Tests

**Files:**
- Modify: `mod/zeus/tools/DungeonStateMachineTest.java`

- [ ] **Step 1: Update existing tests to reflect new fields**
  - Replace any reflection calls targeting `dungeonSchedule` with `dungeonStartMin` / `dungeonEndMin`.

- [ ] **Step 2: Add Tests 76 through 90 covering all UTC+7 time window and quota requirements**
  - Exact start boundary: accepted.
  - Exact end boundary: rejected.
  - One minute before start: rejected.
  - One minute before end: accepted.
  - Hours after window: no late catch-up.
  - Next UTC+7 calendar day: eligible again.
  - Daily `dungeonRuns` reset on new scheduled day.
  - Configured max runs can execute again the following UTC+7 day without toggling `dungeon.on`.
  - Reaching max today waits in `DN_IDLE` rather than permanently disabling tomorrow.
  - Run active at `endMin` is not interrupted.
  - No new run begins after `endMin`.
  - -1/-1 preserves unscheduled behavior.
  - One-sided -1 configuration fails closed.
  - `start >= end` fails closed.
  - Clock failure fails closed.
  - UTC+7 result is identical when JVM default timezone is changed to UTC, GMT+8, etc.

- [ ] **Step 3: Compile and run test suite in WSL**
  - Run `DungeonStateMachineTest` and verify all tests pass.

---

### Task 3: Rebuild `Zeus_Knight.jar` with POTATO and Update Manifest

**Files:**
- Rebuild: `vendor/game/Zeus_Knight.jar`
- Update: `vendor/game/zeus-jar.json`

- [ ] **Step 1: Execute merged build pipeline**
  - Run `sync-build-merged.sh` inside WSL Ubuntu.
  - Verify all gate checks pass:
    - `K_*` count: 38
    - `CTL_VERSION`: 15
    - `Zeus.paint` sites: 2
    - `snapshot version`: 6
    - `snapshot keys`: 49
    - `POTATO.guard`: true
- [ ] **Step 2: Record new JAR SHA256 and update `zeus-jar.json`**

---

### Task 4: Update Rust `zeus-core` Control Schema and Validation

**Files:**
- Modify: `tool/crates/zeus-core/src/control.rs`

- [ ] **Step 1: Update constants, struct fields, and serialization**
  - `CONTROL_VERSION = 15;`
  - `CTL_KEY_COUNT = 38;`
  - `CTL_KEY_NAMES`: replace `dungeon.schedule` with `dungeon.startMin` and `dungeon.endMin`.
  - `ControlSettings`: replace `dungeon_schedule: i8` with `dungeon_start_min: i16`, `dungeon_end_min: i16`.
  - `to_wire()`: format `dungeon.startMin` and `dungeon.endMin`.

- [ ] **Step 2: Update `from_wire()` and validation rules**
  - Parse `dungeon.startMin` and `dungeon.endMin` with range and order validation.
  - Ensure legacy / v14 / unknown versions fail closed.

- [ ] **Step 3: Update `zeus-core` control tests and run `cargo test -p zeus-core control`**
  - Verify all tests pass.

---

### Task 5: Update Rust `zeus-agent` Compatibility and Default Configs

**Files:**
- Modify: `tool/crates/zeus-agent/src/supabase_rest.rs`
- Modify: `tool/crates/zeus-agent/src/main_loop.rs`

- [ ] **Step 1: Update runtime contract in `supabase_rest.rs`**
  - Update `AUTO_DUNGEON_V14` to `AUTO_DUNGEON_V15` with `ctl_version: 15`.
  - Update `AUTO_DUNGEON_COMPATIBLE_JAR_SHA256` with the new JAR SHA256.
  - Update manifest test.

- [ ] **Step 2: Update `main_loop.rs` control generation and tests**
  - Update default control json to include `"dungeon.startMin": -1, "dungeon.endMin": -1`.
  - Update `evaluate_control_version_gate` to accept version 15.
  - Run `cargo test -p zeus-agent`.

---

### Task 6: Update Rust Windows UI (`zeus-ui`) Controls and Validation

**Files:**
- Modify: `tool/crates/zeus-ui/src/model.rs`
- Modify: `tool/crates/zeus-ui/src/port.rs`
- Modify: `tool/crates/zeus-ui/src/windows/dialogs.rs`
- Modify: `tool/crates/zeus-ui/src/windows/dialog_window.rs`

- [ ] **Step 1: Update model and port mappings**
  - In `UiControl`: replace `dungeon_schedule: u8` with `dungeon_start_min: i16`, `dungeon_end_min: i16`.
  - Remove legacy `DUNGEON_SCHEDULE_OPTIONS` / `DUNGEON_SCHEDULE_VALUES`.
  - Update `port.rs` conversions.

- [ ] **Step 2: Update `dialogs.rs` and `dialog_window.rs`**
  - Replace `ID_CFG_DUNGEON_SCHED` with `ID_CFG_DUNGEON_START` and `ID_CFG_DUNGEON_END`.
  - Labels: `"Bắt đầu phó bản (UTC+7)"` and `"Kết thúc khung (UTC+7)"`.
  - Add `parse_time_hhmm` helper and time validation errors in `FieldError`.
  - Validate blank-both (returns -1/-1), reject single blank, reject invalid HH:mm, reject start >= end.
  - Update dialog row count assertion (from 44 to 45, Travel group 13 to 14).
  - Run `cargo test -p zeus-ui`.

---

### Task 7: Update Dockerfile & Final Verification

**Files:**
- Modify: `Dockerfile`

- [ ] **Step 1: Update Dockerfile JAR SHA pin**
- [ ] **Step 2: Run all verification commands**
  - `DungeonStateMachineTest`
  - `DialogGateTest`
  - `ReconnectTravelTest`
  - `CharacterSlotTest`
  - `cargo test -p zeus-core control`
  - `cargo test -p zeus-ui`
  - `cargo test -p zeus-agent`
  - `git diff --check`
- [ ] **Step 3: Commit changes with explicit git staging on `feature/dungeon-daily-utc7-window`**
