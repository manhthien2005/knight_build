# Windows Account UI v1 checkpoint

Fresh verification completed at `2026-08-28T16:23:26Z` on branch `feature/windows-account-ui-v1`.

`verified_implementation_head: 1b8d2b0d821a3fd92fec5ec7129774a5cc219fd6`

This value was captured immediately before the documentation edits. The documentation commit is
reported separately in the handoff; this checkpoint intentionally does not try to contain its own
self-referential commit SHA.

## What M3.1 adds

One Windows x64 GUI executable that manages up to 100 game accounts from a portable folder. The
operator adds an account with a username and password, runs it, and the tool opens the pinned KO402
runtime and performs the fixed login script for that account.

`zeus-ui.exe` sits beside its own `data/` and `runtimes/` directories. Copying the whole `Zeus/`
folder to another writable fixed local drive moves the database, credential key, per-account config,
profile directories, and runtime together.

## Portable root repair and runtime relocation

Startup derives every path from the executable's own directory. Nothing is read from the registry, an
environment variable, or the working directory.

- A copied root is repaired before use: owner is set to the current user where the token permits, and
  inherited ACLs are replaced with a protected current-user/System allowlist.
- Enumeration is capped at the known managed shape: the root, its known state files, the `profiles`
  directory, each UUID profile directory, and the two known launch directories `microemu-home` and
  `temp`. Repair never walks arbitrary unknown descendants, and an unknown descendant keeps the ACLs
  it arrived with.
- A link or reparse point anywhere in the managed shape fails closed, as does an unmarked nonempty
  root, a foreign-owned entry without privilege, and a UNC, device, or removable path.
- `.vault.key.tmp-<uuid>` residue from an interrupted key publish is recognized as a known transient
  file, so a power loss during publish cannot permanently brick a legitimate root.
- The pinned runtime is relocated in one transaction only when runtime ID and descriptor digest both
  match. A digest mismatch changes nothing. A moved-folder launch gate proves the old path is never
  launched, sweeping the whole snapshot and the process launch spec rather than only the stored rows.

## Account aggregate

Each account owns exactly one generated profile. Every mutation is one Immediate transaction with
exactly one global-revision increment, so an account and its profile are never observed half-created,
and a rolled-back import leaves no orphan directory on disk.

- Usernames are printable ASCII, 1..=64 bytes, trimmed exactly, and case-insensitively unique through
  an ASCII-lowercase collation key. Passwords are printable ASCII, 1..=128 bytes.
- Passwords are stored with AES-GCM through Windows CNG using an adjacent 32-byte key file. A rename
  never decrypts or re-encrypts an unchanged password, so renaming still works when the key is
  unavailable.
- Audit stamps clamp to the stored value, so a backwards system clock cannot move a row's time
  backwards.
- Deletion removes the account row and archives its profile. This is not a secure erase: M3.1 leaves
  SQLite `secure_delete` off, so freed pages may still contain old ciphertext.

## Public boundary

The UI never sees an account identity, a password, ciphertext, a config blob, a profile ID, a session
ID, a runtime ID, a PID, or an HWND.

- `ManagerAccountId` is an opaque `Copy + Eq + Ord + Hash` row key with no public constructor, no
  `Display`, and no string accessor.
- `ManagerAccountView` carries exactly `account_id`, `revision`, `username`, `status`, and
  `last_run_at_unix_ms`.
- Reconciled status is exactly four values: `Idle`, `Running`, `LoginFailed`, `CleanupPending`.
  `CleanupPending` outranks `Running`, which outranks a persisted `LoginFailed`, so a live session
  always wins over a stale failure. `Starting`, `Authenticating`, and `Stopping` are UI-local and are
  structurally rejected from the public enum.
- Failures cross the boundary as stable redacted codes, never as backend text.

## Worker, admission, and events

All database, crypto, runtime, and process work happens on one worker thread. The UI thread submits
and polls without blocking.

- Command capacity is 16 and event capacity is 32. Each accepted command consumes exactly one nonzero
  monotonic request ID and produces exactly one FIFO result. A rejected or queue-full command consumes
  no ID.
- Request results are exactly `AccountsListed`, `AccountImported`, `AccountUpdated`, `AccountDeleted`,
  `AccountsRunScheduled`, `AccountStopResult`, and `AccountCleanupRetried`.
- M3.1 adds exactly one unsolicited notification, `AccountStateChanged`, for terminal asynchronous
  readiness completion. It carries no request ID.
- The worker parks instead of blocking on receive, and every send and every disconnect unparks it.
  Without that, `Drop` left the worker parked while holding the Core instance lock forever.

## Bounded readiness and the input gate

At most four readiness tasks and four retained sessions exist at once. A fifth Run is refused before
profile start, secret decrypt, or task creation.

- One bounded Run batch reserves one admission generation atomically before any member starts. The
  generation closes when every reserved member settles its start.
- The global input gate opens only after every started member is Ready or terminally failed, and it
  opens in finite time even when one member never becomes ready. A timed-out task fails and stops only
  its own account.
- Input is serialized: exactly one ready job holds the foreground at a time, and the injection flag is
  cleared on every path including cancellation.
- A completion is accepted only when account, session key, and epoch all match. A stale record is
  dropped with no state, persistence, or event change. The epoch is bumped before any stop or cleanup,
  so an in-flight task abandons its work instead of racing a teardown.

## Fixed login script

The script is valid only for the pinned KO402 runtime and bundle. The operator can neither supply nor
edit it.

- A qualified window is visible, unowned, non-tool, captioned, responsive, owned by the exact PID and
  creation time, epoch-current, and the only qualifying window for that process.
- Client dimensions normalize to reference DPI 120 and must land within two pixels of 239x362. A zero
  or unsupported DPI fails closed. The four measured reference clicks scale independently by width and
  height.
- Every click must fall inside both the client rectangle and the virtual desktop, so a clipped or
  off-screen target fails before any mouse input.
- Foreground acquisition attaches the foreground and target input queues, calls
  `SetForegroundWindow`, and then requires `GetForegroundWindow` to equal the target. The plain call
  measured 1/4 on stable game windows; the attached procedure measured 4/4. The attachment is always
  undone, including on failure.
- Each credential batch is one `SendInput` array beginning with Ctrl+A, so text replaces rather than
  appends and nothing can be interspersed inside it. No credential batch is sent after a revalidation
  mismatch.
- A held modifier key or a higher-integrity target is refused up front. A short or zero send is
  reported only as a generic input rejection, because Windows never reveals whether UIPI discarded it.

## UI surface

The main window has a native report-style table, a toolbar, a per-row command strip, a redacted status
line, and modal dialogs. Table columns are exactly selection checkbox, username, status, last run, and
actions.

- Toolbar commands are Add, Run selected, Stop selected, Delete selected, and Refresh, routed by
  control id through one `WM_COMMAND` router. The row strip carries one contextual action, Edit, and
  Delete; the contextual button's caption is re-derived from the checked row on every
  `LVN_ITEMCHANGED`, so it never shows a stale action.
- Every command control carries its Vietnamese tooltip text as its caption and its stable accessible
  name as its control id.
- The Add dialog offers username, masked password, an optional show/hide toggle, cancel, and add. The
  toggle flips the edit control's password character, so the secret never leaves the native control.
  There is no Save-and-Run button, no auto-login checkbox, and no config control. Destructive delete
  requires typing `XOA`, from both the bulk and the per-row path.
- Edit uses its own password label stating that leaving the field empty keeps the stored secret,
  because a rename never re-encrypts.
- A 50 ms timer drains at most 32 events per tick. The same drain runs from every dialog, so a modal
  dialog cannot stall worker progress.
- Boot state is visible, not hidden: the window shows `Đang khởi động` with every command disabled
  until the worker reports ready, and the first account list is submitted on that readiness event
  rather than at window-create time, because admission refuses a pre-ready command with `NotReady`.
- A portable-root or pinned-runtime boot failure renders in-window as `Tool chưa sẵn sàng` plus
  bounded Vietnamese guidance, with every command still disabled. Only failures that happen before the
  window exists use the bounded message box and a nonzero exit code.
- The status line renders one bounded Vietnamese string with a fixed precedence: boot failure, then a
  closed worker, then close progress, then the last error class, then the boot banner, then the
  account count. No path, OS code, or backend token can reach it.
- Closing with active accounts asks for confirmation and offers only stop-and-exit or cancel. Shutdown
  is submitted exactly once and only after the inventory is empty; a row pending cleanup holds the
  window open. A refused shutdown submission completes the close locally, because a refused command
  produces no result event to wait for.
- The row key travels in the list-view item data and is never rendered as text.

### Character panel

A fixed-width panel beside the table shows the character of the *highlighted* row. Highlight rather
than checkbox: clicking a row is how the operator asks to look at it, while the checkboxes drive batch
commands and are often left checked across several rows.

- The panel has three distinct states, and they are deliberately different messages: no row focused is
  an instruction (`Chọn một tài khoản...`), a focused row with nothing published is a status
  (`Chưa có dữ liệu.`), and a reading is the fifteen labelled lines.
- The projection is pure text in `player_view.rs`, so every line is asserted without a window and
  without a running game. Fifteen tests pin the exact rendering, including the three values that are
  easy to get wrong:
  - experience is permille of the *current level*, so 105 renders as `10,5%` and there is no total;
  - an undelivered wallet renders as `—`, because gold and gem read zero until the inventory packet
    lands and zero is indistinguishable from a broke account by value alone;
  - an exhausted attack quota is named (`0 (đã hết)`), because at zero the client drops out of
    auto-attack with nothing on screen.
- The reading is polled once a second for the focused row only, matching the writer's cadence. A
  reading is never treated as a table change: the table renders no character data, and a repaint
  rebuilds every item, so reporting a change made the poll wipe the operator's highlight and
  checkboxes every second.
- A reading that arrives for a row the operator has since left is dropped rather than shown under the
  newly focused account's name, and a failed poll is silent because it repeats every second and the
  operator cannot act on it.
- A repaint now rewrites cells in place whenever the row keys line up, and restores the checks and the
  highlight when it must rebuild. Before this, any worker event silently discarded a mid-batch
  selection.

## Character transport

The mod runs inside the game's JVM and cannot share memory with the tool, so it writes one small
`key=value` file and the tool reads it. `docs/core/11-player-transport.md` records why a file and not a
socket: a socket inside the client would be an unauthenticated local endpoint any process could drive.

- The launch specification passes `-Dzeus.player.out=<profile>/microemu-home/zeus-player.txt`, pinned
  to the profile's own private directory and revalidated at the adapter boundary, so a mutated argument
  cannot redirect the published file where a sibling session could read or replace it. The property is
  passed on every launch: an unrecognised `-D` is inert to the JVM, so one argv shape serves both the
  vanilla and the modded jar.
- The parser is strict. An unknown key, a missing key, a value outside its documented range, or an
  unexpected format version is a rejection, because a silently mis-parsed reading would be rendered to
  the operator as fact. A *missing* file is not an error: it means the client has not published yet.
- A confirmed stop removes the reading, so a stopped account cannot keep showing a live character.
- The reading crosses the manager boundary as values only. It carries no account identity, no profile
  ID, no session ID, and no path, which is why it needs no separate redacted view type.

## Gate results at the verified head

Windows:

- `fmt --all -- --check` exit 0
- `Invoke-SourceTests.ps1`: 10 PowerShell contracts pass, then the full workspace test run
- `clippy --locked --workspace --all-targets -- --deny warnings` exit 0
- `build --locked --workspace --release` exit 0
- `zeus-core --lib` 236 passed, 0 failed, 9 ignored
- `zeus-ui` 65 passed, 0 failed
- integration targets: `store_bootstrap` 26/0/1, `launch_snapshot` 26/0/1, `foundation_scope` 13/0,
  `manager_controller` 10/0, `runtime_validation` 7/0/1, `runtime_registry` 3/0, `profile_store` 2/0,
  `core_commands` 2/0

Ubuntu, through the pinned zig-based cross toolchain:

- `fmt --all -- --check` exit 0
- `test --locked --workspace --all-targets`: 11 targets, all ok, 0 failures
- `clippy --locked --workspace --all-targets -- --deny warnings` exit 0
- `build --locked --workspace --release` exit 0

## Metadata, protected inputs, secrets, and artifacts

- Locked metadata is exactly 2 packages and 11 targets. `zeus-ui` declares only its binary.
- `Cargo.lock` SHA256 `d6cd971c2a5e04a560fc6aec3a1457edf86a67c0edf84982cdb6471ce6e46053`
- `crates/zeus-core/src/store/schema.rs` SHA256
  `7f148046868dccff655c42800f582a847da696f3bc4f03a313a39fc3ac80fd17`
- `zeus-ui.exe` reports PE subsystem 2, so no console window appears. No WebView2, Qt, GTK, wx, or
  Electron dependency is linked.
- The tracked-secret scan finds no credential literal. The only match is a synthetic value derived
  from a per-run UUID inside the live evidence runner.
- `git status --short` is empty and no `javaw` or `zeus-ui` process survives the run.

## Package contract

`scripts/Assemble-WindowsAccountUiPortable.ps1` copies the locked release binary, and an optional
locally provisioned runtime, into a fresh output root and emits a package-relative inventory. It
writes no registry key, mutates no PATH, and creates no shortcut or uninstaller.

`tests/WindowsAccountUi.Tests.ps1` launches the real binary twice from a workspace path containing
spaces. Fresh startup must create `data/.zeus-hso-root`, `data/state.sqlite3`, `data/vault.key`, and
`data/profiles` exe-relative with nothing written outside the package root. The whole package is then
copied to a different path and launched again, proving the moved copy opens its own state while the
original stays intact.

`data/` is deliberately not pre-created by the assembly script. The application creates it with a
protected owner-only DACL; an empty directory created by the packager carries inherited ACLs and fails
the private-root check.

`-GameJar <name>` repoints the *copied* descriptor at a different jar in the runtime's `game/`
directory, measuring its size and digest from the jar itself and deriving a distinct runtime id. The
source runtime is never modified, so the pinned vanilla descriptor stays intact for the exact-runtime
gates while a modded assembly stays reproducible from one command:

```
scripts/Assemble-WindowsAccountUiPortable.ps1 -OutputRoot <fresh> `
  -RuntimeSource <provisioned runtime> -GameJar Zeus_Knight.jar
```

Two development aids sit beside the runners and are not part of the product:
`scripts/Capture-Window.ps1` reads one window's pixels for a by-eye check, and
`scripts/Dev-DriveShell.ps1` posts messages to the shell's own controls so a panel or dialog can be
brought on screen without a human clicking through it. Nothing in either is compiled into the tool; the
shell itself synthesises no input.

## Operator-gated evidence

Two runners exist so the operator can produce evidence that cannot be automated. Neither can act
without explicit consent, so no default or source-tier invocation launches the game or changes a
privilege.

- `scripts/Invoke-WindowsAccountUiLiveTest.ps1` requires `-AllowGameLaunch`, runs one named ignored
  test serially, derives synthetic credentials per run, and restores the process environment in
  `finally`. It offers no elevation switch.
- `scripts/Invoke-WindowsAccountUiElevatedAclTest.ps1` requires `-AllowElevation` and then requires
  the caller to already be elevated. It never self-elevates.

## Known gaps at this checkpoint

These are stated plainly rather than presented as complete:

- **No run against a real account has been performed.** The launch path is proven end to end against a
  synthetic account: the tool launched `game/Zeus_Knight.jar` with
  `-Dzeus.player.out=<profile>/microemu-home/zeus-player.txt`, the mod inside that client published a
  reading, the panel rendered it, and a confirmed stop removed both the process and the reading. What
  that run could not show is a character: a synthetic account cannot authenticate, so the client stayed
  on its own level-0 `unname` placeholder with an undelivered wallet — which is exactly what the panel
  reported. Reaching the in-game screen needs the operator's own credentials.
- **The exact-game login screenshots are not captured.** Login now happens by seeding the record stores
  the client's own `bs.c()` path reads, before the JVM starts, rather than by injecting input; the two
  operator-confirmed screenshots still do not exist.
- **The elevated foreign-owner ACL repair test body is not written.** That path cannot be proven
  without elevation; its runner is in place and fail-closed.
- **Four accounts running at once has not been observed.** Per-account isolation is argued from the
  per-profile launch directories and the unit tests, not from a four-process run. The character
  transport is per profile by construction — the integration test asserts two profiles never share a
  snapshot path — but no run has had four clients publishing at once.
- **The readiness subsystem is unbound: `login.rs` and `login/windows.rs` are reachable only from
  their own tests.** This is the whole-branch review's Critical C2. It no longer blocks logging in —
  record-store seeding replaced it — but the code is still there and still unreachable, so it should be
  deleted rather than left to look like a pending feature.

- **Independent review happened and its Critical C2 is still open.** Layer-level review agents failed
  on an environment defect, so a whole-branch review was run instead over `97a8034..344b8af`. It
  returned Spec compliance FAIL, Branch quality CHANGES_REQUIRED, 2 Critical / 4 Important / 1 Minor,
  and "do not merge". C1 (the shell bound almost nothing), I1 through I4 and M1 are fixed; C2 above is
  not. The full transcription, including the reviewer's own checkpoint-honesty audit of this file, is
  in `.superpowers/sdd/2026-08-25-windows-account-ui-v1/branch-review-report.md`. The reviewer also
  flagged that `model.rs`, `port.rs` and `shell_window.rs` carried zero tests. `model.rs` and `app.rs`
  now cover the boot, readiness and close-path rules, and `port.rs` has five tests pinning its
  redaction mappers. `shell_window.rs` is still verified only by the package smoke test, which
  enumerates the live window's children and asserts a clean `WM_CLOSE` exit.
- **Commits after the review have not been re-reviewed.** `c522176`, `4a017f8`, `94bfff2`, `0c337db`
  and the marker-deletion commit all landed after the reviewer stopped, and each is self-verified with
  observed RED, mutation probes and full gates rather than by a third party.
