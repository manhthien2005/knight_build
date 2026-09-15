# Windows Manager Worker v1 checkpoint

Fresh verification completed at `2026-08-24T16:48:50.7045439Z` on branch
`feature/windows-manager-worker-v1`.

`verified_implementation_head: bd1bb021506b16146975d06a13bd61262b9c6653`

This value was captured immediately before the documentation edits. The documentation commit is
reported separately in the handoff; this checkpoint intentionally does not try to contain its own
self-referential commit SHA.

Sections through "Metadata, protected inputs, secrets and artifacts" record exactly what was
observed at that initial head and are retained as historical evidence. Two later test-only commits
raised the Windows Rust test counts, so a reader who runs the published commands on the current
branch head sees higher numbers than the lists below. "Post-final-review follow-up evidence" records
that fresh run and accounts for the difference.

## Public boundary and semantics

The M2 addition is a public Windows-only `ManagerWorker`, not a UI or a second lifecycle authority.
One named standard-library thread owns one complete `ManagerController` by value. Production worker
code reaches lifecycle behavior only through the public controller and does not import the private
Supervisor or process adapter.

- UI-side submission and polling use non-blocking `try_send`/`try_recv` behavior. Core, SQLite,
  runtime validation and process lifecycle work execute on the worker thread.
- The command queue capacity is exactly 16 and the event queue capacity is exactly 32. A full
  command queue returns immediately; bounded blocking event delivery applies backpressure inside
  the worker without blocking the UI caller.
- Each successfully accepted command receives one nonzero, monotonic request ID and one FIFO result
  event. State rejection, oversize input, queue full and disconnect do not consume an ID.
- Startup is observable as `Starting` until `Ready` or `OpenFailed` is consumed. A successfully
  enqueued shutdown immediately moves admission to `Closing` and rejects every later submission
  while that shutdown is in flight.
- Only consuming a successful `ShutdownResult` confirms successful shutdown. `Closed` may also
  result from consuming `OpenFailed` or observing worker disconnect. `CloseIncomplete` retains
  `Closing`, permits only session inspection/cleanup and an explicit shutdown retry, and preserves
  request/event order.
- Dropping the UI-side handle does not join, sleep or call lifecycle work. Disconnect cleanup runs
  best-effort on the worker; confirmed shutdown still requires consuming a successful result.

The worker remains single-owner, `Send + !Sync`, non-cloneable, synchronous internally and free of
an async runtime, mutex-wrapped controller, timer, callback, unbounded queue or configurable queue
capacity.

## Fresh Windows source and build evidence

Commands run from the feature worktree:

```powershell
& .\scripts\Invoke-Cargo.ps1 fmt --all -- --check
& .\scripts\Invoke-SourceTests.ps1
& .\scripts\Invoke-Cargo.ps1 clippy --locked --workspace --all-targets '--' '--deny' warnings
& .\scripts\Invoke-Cargo.ps1 build --locked --workspace --release
```

All four commands exited 0. Formatting passed. At the initial head the source tier reported six
passing PowerShell contract groups, then 193 Rust tests passed, 0 failed and 11 were intentionally
ignored:

- library: 128 passed, 9 ignored;
- binary unit target: 0 tests;
- `core_commands`: 2 passed;
- `foundation_scope`: 8 passed (9 at the current head; see the follow-up section);
- `launch_snapshot`: 26 passed, 1 ignored;
- `manager_controller`: 10 passed;
- `profile_store`: 1 passed;
- `runtime_registry`: 1 passed;
- `runtime_validation`: 7 passed, 1 ignored;
- `store_bootstrap`: 10 passed.

The ignored set includes the two exact Manager/worker game gates, five Supervisor/live harness
gates, two dedicated handle-leak gates and the two exact runtime validation/snapshot gates; they
remain outside the default source tier for explicit safety reasons. Warning-denied workspace
all-target Clippy reported zero warnings, and the locked release build completed successfully.

## Exact public worker live evidence

The fresh authorized command used the runner's supported canonical runtime parameter exactly:

```powershell
& .\scripts\Invoke-WindowsManagerWorkerLiveTest.ps1 `
  -RuntimeRoot 'D:\Gaming\KnightOnline_402\Zeus_HSO\runtimes\windows-x64\temurin-11.0.32+9_microemu-2.0.4_ko402' `
  -AllowGameLaunch
```

The listing selected exactly
`manager::worker::windows_live_runtime_tests::exact_runtime_runs_through_public_manager_worker`
and reported `1 test, 0 benchmarks`. The exact run reported 1 passed, 0 failed, 0 ignored and 136
filtered out in 3.24 seconds, followed by `PASS: Windows manager worker live test` and exit 0.

The gate traversed public worker readiness, exact runtime catalog, start, qualified process/window
readiness, observe, confirmed stop, empty session inventory and successful shutdown. Test-private
birth identity remained behind `cfg(all(test, windows))`; it did not enter a public request or
event. The runtime remained `NeedsValidation`.

After the runner returned, an independent cleanup check observed:

- successful shutdown already consumed by the live test;
- `java`/`javaw` process count: 0;
- guarded live base absent, therefore UUID live-root child count: 0;
- named worker live/helper executable artifact count: 0.

The gate did not log in, supply credentials, send input, read title/text or capture a screenshot.

## Fresh Ubuntu portability evidence

Commands run through WSL at the same worktree:

```powershell
wsl.exe --cd /mnt/d/Gaming/KnightOnline_402/Zeus_HSO/.worktrees/windows-manager-worker-v1 -- `
  ./scripts/Invoke-Cargo-Ubuntu.sh fmt --all -- --check
wsl.exe --cd /mnt/d/Gaming/KnightOnline_402/Zeus_HSO/.worktrees/windows-manager-worker-v1 -- `
  ./scripts/Invoke-Cargo-Ubuntu.sh test --locked --workspace --all-targets
wsl.exe --cd /mnt/d/Gaming/KnightOnline_402/Zeus_HSO/.worktrees/windows-manager-worker-v1 -- `
  ./scripts/Invoke-Cargo-Ubuntu.sh clippy --locked --workspace --all-targets -- --deny warnings
wsl.exe --cd /mnt/d/Gaming/KnightOnline_402/Zeus_HSO/.worktrees/windows-manager-worker-v1 -- `
  ./scripts/Invoke-Cargo-Ubuntu.sh build --locked --workspace --release
```

All four commands exited 0. Tests reported 58 passed, 0 failed and 0 ignored: library 8,
`core_commands` 1, `foundation_scope` 8, `launch_snapshot` 26, `runtime_validation` 7 and
`store_bootstrap` 8; Windows-only integration targets contained 0 tests. Warning-denied Clippy and
the locked release build passed without warnings. The foundation boundary test confirms the worker
exports remain absent on Ubuntu; this is portable Core evidence, not a Windows containment claim.

## Metadata, protected inputs, secrets and artifacts

The following gates were run freshly:

```powershell
& .\scripts\Invoke-Cargo.ps1 metadata --locked --no-deps --format-version 1
& .\scripts\Invoke-Cargo.ps1 test --locked -p zeus-core --test foundation_scope '--' --nocapture
Get-FileHash -Algorithm SHA256 Cargo.lock
Get-FileHash -Algorithm SHA256 crates\zeus-core\src\store\schema.rs
git grep -n -I -E 'sk-[A-Za-z0-9]{32,}|api[.]9aws[.]net' -- .
git status --short
```

Metadata exited 0 with one package and exactly 10 targets:
`zeus_core`, `zeus-core`, `core_commands`, `foundation_scope`, `launch_snapshot`,
`manager_controller`, `profile_store`, `runtime_registry`, `runtime_validation` and
`store_bootstrap`. There is no declared worker live/helper target. Locked metadata and the unchanged
lock hash establish that M2 added no Cargo dependency.

The focused foundation gate passed 8/8 with 0 failed and 0 ignored at the initial head, and 9/9 at
the current head. Protected hashes matched:

- `Cargo.lock`: `E5619FADFC851356392FDD9FF4363874FCA0CB9C4EB2EEA6C61B2B8E0A8E7882`;
- `crates/zeus-core/src/store/schema.rs`:
  `779C3213A874DDE50C1FE14F0E8E8F8A1EEF321C17552ADBDEEF068D6F776F1E`.

The tracked-tree prohibited credential/domain scan returned exit 1 with no matches. Before the
documentation edit, `git status --short` returned no entries. No live root, owned Java process or
named worker live/helper executable remained.

## Post-final-review follow-up evidence

`verified_test_head: 8a9ec547532c399494d6fc3d7fa3e77e7bdbe8e3`

`bd1bb02..8a9ec54` contains two documentation commits and two test-only commits. Comparing those two
heads, only `README.md`, this checkpoint file, `crates/zeus-core/tests/foundation_scope.rs` and
`tests/WindowsManagerWorkerLive.Tests.ps1` differ; nothing under `crates/zeus-core/src` or `scripts`
changed, so the production worker and the worker live runner are byte-identical to the initial head.

`8a9ec54` itself touches only those two test files. It adds one Windows-gated integration test,
`manager_worker_handle_stays_send_never_sync_and_never_cloneable`, which proves the handle stays
`Send + !Sync` and non-cloneable through trait resolution rather than source spelling; the worker
source-text scan beside it is retained only as defence in depth. It also adds one AST identity check
requiring the runner's exact `-AllowGameLaunch` rejection to be a top-level runner statement rather
than a nested decoy. That single new test is the entire reason the counts here differ from the
initial head: it is the 9th `foundation_scope` test on Windows and, being `#[cfg(windows)]`, is
absent on Ubuntu.

The published commands were re-run at that test head, with only comment and documentation follow-up
edits present in the working tree and no executable Rust change:

- Windows `fmt --all -- --check` exited 0.
- Windows `Invoke-SourceTests.ps1` exited 0 with six passing PowerShell contract groups, then 194
  Rust tests passed, 0 failed and 11 intentionally ignored. Only `foundation_scope` moved, from 8 to
  9 passed; library 128 passed with 9 ignored, `core_commands` 2, `launch_snapshot` 26 with 1
  ignored, `manager_controller` 10, `profile_store` 1, `runtime_registry` 1, `runtime_validation` 7
  with 1 ignored and `store_bootstrap` 10 all match the initial head.
- The focused foundation gate `test --locked -p zeus-core --test foundation_scope '--' --nocapture`
  reported 9 passed, 0 failed, 0 ignored.
- Windows warning-denied all-target Clippy and the locked workspace release build exited 0 and
  emitted no warnings.
- Ubuntu `test --locked --workspace --all-targets` through WSL exited 0 with 58 passed, 0 failed and
  0 ignored, `foundation_scope` still 8. The Ubuntu tier above therefore remains current as written.

No live game launch was performed for this follow-up. Because the production worker and the live
runner are byte-identical to the initial head, the exact public worker live evidence recorded above
remains the authorized evidence for this branch and was deliberately not re-launched.

## Implementation and review provenance

Tasks 1-7 were executed serially by fresh `gpt-5.6-sol` implementer contexts at high reasoning,
with independent root reproduction of focused gates and a task-scoped spec/quality review after
each implementation commit. Task 3 required one reviewed fix round for cloning only after request
ID admission. Tasks 1, 2, 4, 5 and 6 closed without a code fix round. Task 7 required three reviewed
fix rounds to complete the Rust forbidden-surface guards and exact PowerShell AST contract; the
third scoped re-review closed every Important finding. The verified implementation head therefore
contains no unresolved Critical or Important task-review finding.

Task 5 had one explicit procedural ruling: its backpressure/disconnect tests were already green on
the Task 4 base because Task 4 had mandated ordinary event-send cleanup. The implementer validated
test sensitivity with a temporary, uncommitted cleanup mutation, restored it, then committed only
the required delivery/cleanup centralization. This avoids inventing a historical RED while retaining
mutation evidence for the behavior.

Task 7 carried two scope rulings required by the binding spec. It owned the otherwise omitted
`worker/engine.rs` and Manager live-test module only for the private birth-evidence bridge, and its
capacity-one reply channel is permitted only behind exact `cfg(all(test, windows))`; the ban on
per-request channels remains binding for production/public worker operation.

Task 7 also exposed one environmental ACL concern. The worktree-local runtime copy correctly fails
the unchanged secure ACL preflight. No preflight, ACL or runtime content was weakened. The canonical
pinned protected runtime shown above passed Task 7 twice and this fresh Task 8 run once. Therefore
qualification currently requires the explicit canonical `-RuntimeRoot`; the failing local copy is
not accepted as evidence.

## Explicit exclusions and M3 boundary

M2 does not add or claim a UI, IPC, daemon/service, network listener, logging/capture, credentials,
login/input automation, screenshots, telemetry, CLI lifecycle commands, profile/runtime mutation,
automatic polling, scheduler, persistence, reconnect, runtime `Supported` status,
gameplay/network/RMS compatibility, soak/stress, host 1 GiB suitability or production support.

M3 is the future non-technical Windows UI milestone. It may own one `ManagerWorker`, drain events on
the UI loop and map stable codes to user-facing Vietnamese/English copy. It must not call
`ManagerController`, Core lifecycle, Supervisor or the process adapter directly. UI toolkit,
screens, localization, refresh cadence, packaging and any later IPC remain separate design work.
