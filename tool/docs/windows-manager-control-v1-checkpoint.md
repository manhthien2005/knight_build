# Windows Manager Control v1 checkpoint

Kiểm chứng cuối trên implementation HEAD `3c9328c1060113cdd324e7bce4824044775f3714`, branch
`feature/windows-manager-control-v1`, tại `2026-08-24T12:56:06.4829575Z`. Base của lát cắt là
`fbe568d7f41b7ad01b3fa042f5e15eae17a29f44`. Documentation commit/HEAD cuối được báo riêng trong
handoff để tránh SHA tự tham chiếu.

## Public scope và ownership boundary

Public surface mới chỉ tồn tại dưới `#[cfg(windows)]`:

- `ManagerController::open_at`, `open_default`, `from_core`;
- `list_profiles`, `list_runtimes`, `list_sessions`;
- `start_profile`, `observe_session`, `stop_session`, `retry_cleanup`, `close`;
- các page/view/observation/exit type và `ManagerError` taxonomy đã redaction.

Controller synchronous, `Send` và chủ ý `!Sync`, giữ một owner duy nhất. `SessionSupervisor`,
process adapter, process identity và live observation vẫn crate-private. Ubuntu không export Manager;
portable `CoreState` và diagnostic CLI không đổi.

## State, deadline, cleanup và redaction

- State machine private là `Open → Closing → Closed`. Closing chặn start trước Core/backend nhưng
  vẫn cho catalog/list/observe/stop/retry/close. Closed close idempotent, list session rỗng và mọi
  operation còn lại trả `ControllerClosed` trước validation hoặc process work.
- Một profile chỉ có một retained owner; capacity toàn controller là bốn. Session inventory luôn
  sort theo canonical UUIDv4.
- Mỗi start, terminal observe cleanup, stop, cleanup retry và từng owner action trong close nhận một
  deadline nội bộ mới đúng 10 giây; caller không cung cấp hay kéo dài deadline.
- Một close call snapshot inventory đã sort, tác động đúng một lần lên mỗi owner, tiếp tục sau lỗi,
  rồi đọc inventory authoritative. Tối đa bốn wait budget, tức tối đa 40 giây configured adapter
  waits mỗi call. `CloseIncomplete` chỉ chứa tối đa bốn redacted session view và retry được.
- Natural exit chỉ trả `Exited` một lần rồi `SessionNotFound`. Stop/retry/terminal-cleanup lỗi giữ
  owner để retry; không bịa một trạng thái success.
- Public views/errors không chứa PID, process birth/creation identity, exit code, descriptor digest,
  path, raw handle, Win32 stage/OS code hay arbitrary source. `Display`/`Debug` bounded và
  `Error::source()` luôn `None`.
- Không có blocking custom `Drop`; field destruction vẫn đi tới kill-on-close fail-safe hiện hữu.

## Files và commits

Production scope mới gồm `src/manager/{mod,error,types}.rs`, Windows-only export trong `src/lib.rs`
và visibility crate-private tối thiểu trong `session_supervisor/{mod,backend}.rs`. Test scope gồm
manager unit/public integration, shared Windows live fixture, ignored one-session gate, fail-closed
PowerShell runner contract và expanded foundation scope. Design/plan nằm dưới `docs/superpowers`.

Commit từ base đến implementation HEAD:

- `dc8bb3aac5cb6eded1593995d4f845505ff4fcf0` — `docs: design Windows manager control v1`
- `8a2e11eb830c5c43421790a9afb0b487854b6cfd` — `feat: define Windows manager control contract`
- `66e335cd30aceb54d5d1ca2f629c47edbb2dfa33` — `feat: expose redacted manager catalog reads`
- `d39f8d7aa5da6d4b3e62c439720b1c0e87d148eb` — `feat: start profiles through manager control`
- `c333d10d02efbcd751d8dd1321343ca77f41343e` — `feat: control manager session lifecycle`
- `9b9c40205696b5aafb3326d2d451d3d2d531dbe4` — `feat: close manager sessions safely`
- `d4bddb7e5a0e3bb0b78f127a85d74c6a31f78ee7` — `test: qualify public manager runtime control`
- `3c9328c1060113cdd324e7bce4824044775f3714` — `test: lock Windows manager platform boundary`

## Protected inputs

- Rust/Cargo portable toolchain: `1.98.0`; Windows host `x86_64-pc-windows-gnu`.
- `Cargo.lock` SHA-256:
  `E5619FADFC851356392FDD9FF4363874FCA0CB9C4EB2EEA6C61B2B8E0A8E7882`.
- `crates/zeus-core/src/store/schema.rs` SHA-256:
  `779C3213A874DDE50C1FE14F0E8E8F8A1EEF321C17552ADBDEEF068D6F776F1E`.
- Runtime ID: `windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402`; capability vẫn
  `NeedsValidation`. Exact preflight yêu cầu đúng 337 JRE file.
- Không có thay đổi `Cargo.lock`, store schema hay exact runtime payload trong lát cắt này.

## Windows verification

Fresh verification trên implementation HEAD:

- `fmt --all --check`: pass.
- `Invoke-SourceTests.ps1`: mọi provisioning/test-tier/smoke/live-manager fail-closed contract pass;
  Cargo tổng 162 passed, 0 failed, 10 intentional ignored.
- Library: 101 passed, 8 ignored. Integrations: `core_commands` 2; `foundation_scope` 7;
  `launch_snapshot` 26 + 1 ignored; `manager_controller` 7; `profile_store` 1;
  `runtime_registry` 1; `runtime_validation` 7 + 1 ignored; `store_bootstrap` 10.
- Workspace all-target Clippy với `--deny warnings`: pass.
- Locked workspace release build: pass.
- Locked Cargo metadata: đúng 10 target, không có live/helper target.
- `git diff --check`: pass; hai protected hash khớp giá trị ở trên.

## One-session public Manager live evidence

Runner `Invoke-WindowsManagerControlLiveTest.ps1` fail-closed nếu thiếu `-AllowGameLaunch`, sai host,
runtime không phải fixed local directory hoặc exact verify-only không khớp. Nó list/run đúng test:

`manager::windows_live_runtime_tests::exact_runtime_runs_through_public_manager_control`

Fresh re-qualification sau thay đổi test-boundary: 1 passed, 0 failed, 108 filtered, khoảng 3.04 giây.
Gate đăng ký một exact runtime và tạo một profile trước `ManagerController::from_core`; sau đó dùng
public start/list/observe/stop/close. Test-private birth identity chỉ mở identity-checked observation
để xác nhận một visible responsive window và cleanup; không đi vào public result. Public runtime
view trước/sau readiness vẫn `NeedsValidation`.

Sau pass: Manager inventory bằng 0, `java`/`javaw` còn lại bằng 0, guarded live-root có 0 child và
hai process-scoped live environment value được restore. Không đọc title/text, không gửi input, không
login, không dùng credential và không chụp screenshot.

## Ubuntu compatibility

WSL2 Ubuntu dùng portable toolchain đã có trong repo; không thay đổi system toolchain. Final gates:

- format: pass;
- locked workspace all-target tests: 57 passed, 0 failed, 0 ignored;
- library 8, `core_commands` 1, `foundation_scope` 7, `launch_snapshot` 26,
  `runtime_validation` 7, `store_bootstrap` 8; Windows-only integration targets có 0 test;
- workspace all-target Clippy `--deny warnings`: pass;
- locked workspace release build: pass.

Gate đầu tiên đã bắt fixture Windows khai báo quá rộng dưới `cfg(test)`; boundary được sửa thành
`cfg(all(test, windows))` rồi toàn bộ bốn Ubuntu gate được chạy lại GREEN. Kết quả này chỉ chứng minh
portable Core compatibility, không phải Windows process containment trên Ubuntu.

## Explicit exclusions và milestone kế tiếp

M1 không thêm UI, worker/background scheduler, async runtime, IPC, CLI lifecycle command,
logging/capture, graceful control channel, terminal history hay profile/runtime CRUD mutation. Không
có login automation, credential storage, input automation, title/text inspection hoặc screenshot.
Không có claim RMS compatibility, gameplay/network success, account isolation ở tầng game,
soak/stress, host 1 GiB hay production support. Runtime không được nâng thành `Supported`.

Game thật hiện chỉ nối được qua public Windows API và opt-in developer gate, chưa nối vào user
workflow. Milestone kế tiếp là worker orchestration cùng non-technical UI/control surface trên
contract M1 này; việc đó cần spec, authorization và lifecycle evidence riêng.
