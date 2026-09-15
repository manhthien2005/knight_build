# Session Supervisor v1 checkpoint

Ngày kiểm chứng: 2026-08-24.

## Scope and architecture

`SessionSupervisor` là lớp Windows-only, crate-private và in-memory sở hữu `CoreState`, backend
process, session map và profile admission index. Nó nhận tối đa bốn session và đúng một active
session cho mỗi profile. Admission materialize `ProcessLaunchSpec` từ snapshot do Core kiểm chứng;
owner được giữ fail-closed cho tới khi root exit và Job tree-empty đều được xác nhận.

Lifecycle v1 chỉ có observe, confirmed hard stop và retry cleanup. Không có graceful control
channel, terminal history, persistence, CLI, IPC, UI hoặc public API. Production không dùng shell
hay `std::process::Command`.

## Files changed

- `crates/zeus-core/src/lib.rs`
- `crates/zeus-core/src/process_adapter/mod.rs`
- `crates/zeus-core/src/process_adapter/windows.rs`
- `crates/zeus-core/src/process_adapter/windows/test_support.rs`
- `crates/zeus-core/src/process_adapter/windows/tests.rs`
- `crates/zeus-core/src/process_launch_spec.rs`
- `crates/zeus-core/src/session_supervisor/backend.rs`
- `crates/zeus-core/src/session_supervisor/mod.rs`
- `crates/zeus-core/src/session_supervisor/tests.rs`
- `crates/zeus-core/src/session_supervisor/windows_tests.rs`
- `crates/zeus-core/tests/foundation_scope.rs`
- `README.md`
- `docs/session-supervisor-v1-checkpoint.md`

Không thêm dependency, không đổi `Cargo.lock` hoặc database schema, và không thêm Cargo target,
CLI command, IPC, UI hay Ubuntu process adapter.

## Toolchain and protected hashes

Toolchain repository báo `rustc 1.98.0 (88d9e12ae 2026-08-18)`, host
`x86_64-pc-windows-msvc`, và `cargo 1.98.0 (797e8a9bc 2026-08-05)`.

Live dummy probe được compile trực tiếp bằng `rustc.exe` của toolchain pinned
`1.98.0-x86_64-pc-windows-gnu` và `rust-lld.exe` cùng target. Test canonicalize rồi dùng absolute
path cho cả compiler lẫn linker, với linker flavor `ld.lld`; evidence không ghi machine-specific
absolute path.

- `Cargo.lock`: `E5619FADFC851356392FDD9FF4363874FCA0CB9C4EB2EEA6C61B2B8E0A8E7882`.
- `crates/zeus-core/src/store/schema.rs`:
  `779C3213A874DDE50C1FE14F0E8E8F8A1EEF321C17552ADBDEEF068D6F776F1E`.

## Source-tier and Windows results

- Windows format check: pass.
- `Invoke-SourceTests.ps1`: 109 passed, 0 failed, 5 intentional ignored; provisioning contracts,
  test-tier fail-closed contracts và SmokeLauncher contract đều pass.
- Workspace all-target clippy với deny warnings: pass.
- Locked workspace release build: pass.
- Deterministic fake-backend Supervisor suite: 17 passed, 0 failed, 0 ignored.
- Full live Windows Supervisor module trong source tier: 4 passed; parent-child harness và handle
  gate là intentional ignores riêng.

Source tier không đọc hoặc launch JRE, MicroEmulator hay game payload.

## Live ownership and parent-crash evidence

Dummy Rust probe chứng minh hai session đồng thời có session/profile/birth identity riêng, hard stop
xác nhận cả root lẫn descendant đã signal, và drop Supervisor không block nhưng vẫn đóng toàn bộ Job
đang sở hữu. Test parent-crash riêng đã pass: sau khi terminate process giữ Supervisor, cả root và
descendant đều signal trong deadline năm giây và child test process được reap. Evidence không chứa
PID, argv, environment hoặc temp path. Live ownership đi qua test-only `start_spec_for_test` với
probe do pinned GNU compiler/linker tạo; nó không đi qua Core snapshot và không launch runtime.

## Serialized handle gates

Mỗi command chọn đúng một ignored test và chạy serialized:

- Adapter: 1 passed; baseline `108`, final normal `108`, final terminate `108`.
- Supervisor: 1 passed; baseline `108`, final natural `108`, final stop `108`.

Supervisor gate chạy warm-up rồi 32 natural-exit cycle và 32 hard-stop cycle; mọi sample hoàn tất
cleanup phải không vượt `baseline + 2`.

## Exact-runtime regression without Java launch

Provisioning `-VerifyOnly` trả `verified` cho runtime ID
`windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402`, 337 JRE file và descriptor SHA-256
`3e39b998bac686d8d61b8fb2da1b7f207efac463097d643341d1990525c7eac9`.

Hai exact ignored Rust gate được chọn riêng và đều pass 1/1: descriptor validation vẫn trả
`NeedsValidation`, và exact snapshot materialization vẫn pass. SmokeLauncher exact-runtime dry-run
contract cũng pass. Không command nào trong tier này launch Java.

## Ubuntu portable results

- Ubuntu format check: pass.
- Locked workspace all-target tests: 54 passed, 0 failed, 0 ignored.
- Workspace all-target clippy với deny warnings: pass.
- Locked workspace release build: pass.

`cfg(windows)` loại toàn bộ live Supervisor, adapter và probe support khỏi Ubuntu build. Kết quả này
chỉ chứng minh portable Core regression; không phải evidence containment trên Ubuntu.

## Metadata, scope, and artifact isolation

Locked Cargo metadata có 9 target và không khai báo target hoặc source path mang tên
`windows_process_probe` hay `session_supervisor_parent`. Target tree audit cũng có 0 emitted helper
artifact mang hai tên này. Probe và parent harness chỉ được compile trực tiếp trong test Windows.

Scope audit có 0 public export/module match cho Supervisor hoặc process adapter, 0 forbidden
command/shell match trong production adapter/Supervisor sources, và CLI tiếp tục từ chối lifecycle
commands. Sau toàn bộ gate, repository root và system temp đều có 0 UUID test directory do live
probe tạo.

## Explicit limits and next gates

- Runtime exact Windows vẫn là `NeedsValidation`, không phải `Supported`.
- Không có claim Supervisor launch Java, MicroEmulator hoặc game.
- Không có graceful stop, control channel, persistence, terminal history, monitoring, logs, IPC
  hoặc UI.
- Không có Ubuntu containment claim hoặc Ubuntu process adapter.
- Dummy probe không thay thế gate RMS thật, hai emulator thật đồng thời, long-run/resource soak,
  nested/restrictive Job compatibility hoặc production Manager wiring.
- Public request boundary và bất kỳ Java/game enablement nào phải là milestone riêng với evidence
  fail-closed tương ứng.
