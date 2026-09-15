# Windows Contained Process v1 checkpoint

Ngày kiểm chứng: 2026-08-23.

Đây là checkpoint cho adapter Windows crate-private nhận một `ProcessLaunchSpec` đã validate. Adapter
vẫn chưa được nối vào caller nào. Dummy probe Rust chỉ là test support được compile trực tiếp trong
test Windows; probe không phải Cargo target và không nằm trong release.

## Phạm vi file

Toàn bộ lát cắt WCP v1 từ Task 0 đến Task 7 thay đổi đúng các file sau:

- `crates/zeus-core/Cargo.toml`
- `crates/zeus-core/src/lib.rs`
- `crates/zeus-core/src/launch_snapshot.rs`
- `crates/zeus-core/src/process_launch_spec.rs`
- `crates/zeus-core/src/process_launch_spec/windows.rs`
- `crates/zeus-core/src/process_adapter/mod.rs`
- `crates/zeus-core/src/process_adapter/windows.rs`
- `crates/zeus-core/src/process_adapter/windows/tests.rs`
- `crates/zeus-core/tests/support/windows_process_probe.rs`
- `crates/zeus-core/tests/foundation_scope.rs`
- `README.md`
- `docs/windows-contained-process-v1-checkpoint.md`

Không có dependency crate mới. `Cargo.lock`, schema database, public API, CLI command, script,
design và implementation plan không đổi.

## Toolchain và kết quả gate

Toolchain Windows ghim tại repository báo `release: 1.98.0` và
`host: x86_64-pc-windows-gnu` (`rustc 1.98.0 (88d9e12ae 2026-08-18)`).

Windows PowerShell:

- `& .\scripts\Invoke-Cargo.ps1 fmt --all --check`: pass.
- `& .\scripts\Invoke-Cargo.ps1 test --locked --workspace --all-targets`: 87 passed, 0 failed,
  1 ignored. Test ignored là gate leak riêng, không được tính là đã chạy trong suite thường.
- `& .\scripts\Invoke-Cargo.ps1 clippy --locked --workspace --all-targets '--' '--deny' warnings`:
  pass, không warning.
- `& .\scripts\Invoke-Cargo.ps1 build --locked --workspace --release`: pass.
- `& .\scripts\Invoke-Cargo.ps1 test --locked -p zeus-core --lib process_adapter::windows::tests::windows_handle_count_stays_bounded '--' --ignored --exact --test-threads=1 --nocapture`:
  đúng 1 test passed; baseline 109 handle, final normal 109, final terminate 109. Mỗi pha gồm 64
  cycle và mọi sample đều không vượt `baseline + 2`.

Ubuntu wrapper:

- `./scripts/Invoke-Cargo-Ubuntu.sh fmt --all --check`: pass.
- `./scripts/Invoke-Cargo-Ubuntu.sh test --locked --workspace --all-targets`: 52 passed, 0 failed,
  0 ignored.
- `./scripts/Invoke-Cargo-Ubuntu.sh clippy --locked --workspace --all-targets -- --deny warnings`:
  pass, không warning.

Windows adapter và live probe được `cfg(windows)` loại khỏi build/test Ubuntu; số test Ubuntu trên
không chứa evidence live Windows.

## Release isolation và scope guard

Fresh release procedure dùng temp root tuyệt đối và một leaf UUID được kiểm tra trước khi build và
kiểm tra lại trước khi xóa. Cargo metadata có 1 package, 9 target và 0 target khai báo probe theo
cả source path lẫn target name. Fresh locked workspace release build thành công, cây target mới có
0 artifact tên `windows_process_probe`, và đúng target UUID đã kiểm tra được xóa thành công. Không
ghi lại random temp path.

Protected-file hashes bằng Task 0:

- `Cargo.lock`: `E5619FADFC851356392FDD9FF4363874FCA0CB9C4EB2EEA6C61B2B8E0A8E7882`.
- `crates/zeus-core/src/store/schema.rs`:
  `779C3213A874DDE50C1FE14F0E8E8F8A1EEF321C17552ADBDEEF068D6F776F1E`.

Source/scope checks cho kết quả:

- 0 match export `pub use .*process_adapter` hoặc `pub mod process_adapter`; adapter vẫn
  crate-private và unwired.
- 0 forbidden-token match trong đúng ba production source
  `src/process_adapter/mod.rs`, `src/process_adapter/windows.rs` và
  `src/process_launch_spec/windows.rs` cho `std::process::Command`, `Command::new`, `cmd.exe`,
  `powershell`, `/bin/sh`, `sh -c`.
- CLI dispatch vẫn chỉ có `init`, `runtime`, `profile`; không có launch, stop hoặc session command.
- Audit sau gate không còn temp UUID test child hoặc repo probe artifact được sinh bởi test.

## Giới hạn evidence

Evidence này chỉ áp dụng cho dummy Rust probe. Nó không chứng minh:

- Java/JDK 11, MicroEmulator hoặc game launch đúng;
- cleanup khi parent/Core crash;
- admission/compatibility khi Manager nằm trong nested hay restrictive Job;
- bất kỳ kết nối nào với Supervisor, `CoreState` hoặc CLI;
- đóng hoàn toàn cửa sổ TOCTOU khi tạo process.

Không có tuyên bố Supervisor hoàn tất hoặc Java/game đã sẵn sàng. Graceful stop, session,
persistence, monitoring, IPC, logs, UI và Ubuntu containment vẫn nằm ngoài milestone này.
