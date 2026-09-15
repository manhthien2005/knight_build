# Windows Live Runtime Bridge v1 checkpoint

Ngày kiểm chứng live: `2026-08-24T08:36:59.1079950Z`.

## Scope and architecture

Bridge v1 là evidence Windows-only, test-private cho production path nội bộ
`CoreState` snapshot → `ProcessLaunchSpec` → process adapter do `SessionSupervisor` sở hữu.
Runner PowerShell fail-closed yêu cầu opt-in rõ ràng, chạy tuần tự một session, bốn session và
parent-crash, rồi dùng parser strict cho một record hiệu năng numeric-only.

Supervisor, adapter và live observation vẫn crate-private. Không có public caller, CLI launch,
IPC, UI hay background orchestration. Runtime exact vẫn là `NeedsValidation`.

## Files and commits

Surface chính gồm live runner/parser contracts, hai module Rust live test-private, Windows
observation support, typed Windows invocation rendering, scope gates và process-free exact dry-run.
Artifact của mốc này là
`smoke-results/windows-live-runtime-bridge-v1-2026-08-24.json`.

Lịch sử commit thực tế từ design/plan tới prerequisite cuối, gồm mọi corrective commit:

- `eb6109d476be87a53f00fe71e8b8c49f8b002e1d` — `docs: design Windows live runtime bridge v1`
- `9c018ff382f90e9e98a3880702b873d16a5b377f` — `docs: plan Windows live runtime bridge v1`
- `a344004a60131f838ff2123fc1c39a15418e3c90` — `test: add fail-closed live runtime bridge runner`
- `0c7a50307c726b55a90d91b858f67ed89be6c5ac` — `fix: require integer live performance evidence`
- `f39ca512cf26a7e2687c7067da76fffa705f3820` — `test: add live runtime observation support`
- `8a583f39b1dba7d8fc54021d6ba385fba615b9b0` — `test: prove exact runtime supervisor launch`
- `efe36a3cbf97287933156786187509bb98ff8942` — `fix: restore absent live environment variables`
- `9fea7421a6c8e7701d09ccb7304412f514fb3bbf` — `fix: preserve live contract environment state`
- `8e45e967bab9a9624806663652e6b1791dfcedbf` — `test: qualify four exact runtime sessions`
- `9aafc1074762401a394981acd14892df53b2e904` — `fix: enforce sampled CPU and reverse live stops`
- `cf4090a81175f573d5f6426eaf675d30c46ee914` — `test: prove exact runtime parent crash containment`
- `feae1ba0eac770082233648384768e609b85db0f` — `fix: serialize live test base mutations`
- `0758a0a2e6b9a77df4ac4eea6622990f05ee16fa` — `test: gate live runtime bridge scope`
- `caa0b805d3d6427cb3ef2b22bde4d8fa5c22a6c1` — `fix: keep exact dry run process-free`

## Toolchain and protected hashes

- Rust `1.98.0 (88d9e12ae 2026-08-18)`, host `x86_64-pc-windows-gnu`; Cargo
  `1.98.0 (797e8a9bc 2026-08-05)`.
- `Cargo.lock`: `E5619FADFC851356392FDD9FF4363874FCA0CB9C4EB2EEA6C61B2B8E0A8E7882`.
- Store schema: `779C3213A874DDE50C1FE14F0E8E8F8A1EEF321C17552ADBDEEF068D6F776F1E`.
- Game JAR: `6608BB0C77F03749E46165F711E9566DCA4E172CE232256497B35FAAFE74C259`.
- Runtime ID: `windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402`; descriptor digest
  `3e39b998bac686d8d61b8fb2da1b7f207efac463097d643341d1990525c7eac9`.
- Provisioning verify-only trả `verified` với đúng 337 JRE file.

## Source and exact-static results

Windows source tier: library 81 passed, 0 failed, 7 intentional ignored; integrations lần lượt
`core_commands` 2/0, `foundation_scope` 6/0, `launch_snapshot` 26/0 với 1 ignored,
`profile_store` 1/0, `runtime_registry` 1/0, `runtime_validation` 7/0 với 1 ignored và
`store_bootstrap` 10/0. Format, workspace all-target Clippy deny-warnings và release build đều pass.

Exact-static chọn riêng hai gate và mỗi gate pass 1/0; SmokeLauncher exact dry-run pass mà không
khởi động Java process. Monitor 10 ms lấy 345 mẫu, `java`/`javaw` baseline, peak và final đều bằng 0.
Hai handle gate serialized cũng pass 1/0: adapter `116 → 116/116`, Supervisor `116 → 116/116`.
Parent-process-death probe gate pass 1/0.

## Single-profile production-path evidence

Gate đầu của runner Task 7 list đúng một test và pass 1/0. Nó dùng production
`SessionSupervisor::start_session`, kiểm runtime ID/digest exact nhưng vẫn `NeedsValidation`, profile
revision 1, metadata/birth identity, running observation, responsive visible HWND và canonical
non-reparse `config2.xml` chỉ dưới profile đó. Stop dùng identity-checked owner, xác nhận root signal,
Supervisor rỗng và cleanup root trước khi pass.

Outer monitor xác nhận phase concurrency đầu là 1. Khoảng HWND ngắn không rơi vào sample 100 ms;
readiness nội bộ identity-checked của gate đã pass. Không title/text/input/screenshot được thu thập.

## Four-profile isolation and capacity evidence

Gate thứ hai list đúng một test và pass 1/0. Bốn profile đăng ký riêng được launch đồng thời qua
production path; writable home/temp/config không trùng nhau. Profile thứ năm trả
`CapacityReached { maximum: 4 }` trước spawn. Sau 15 giây stabilization, bốn owner được kiểm
identity/liveness/responsiveness và lấy đúng 13 sample tại offset tuyệt đối 0..60 giây.

Outer monitor ghi phase concurrency `[1, 4, 1]`, peak 4, bốn window hiệu năng responsive, không
overcapacity và không nonresponsive observation. Cleanup stop bốn session theo thứ tự ngược với
birth/signal confirmation và final Supervisor rỗng.

## Four-session performance evidence

Phần record chuẩn hóa của Task 7 được real parser chấp nhận trước khi thêm metadata và giữ nguyên:

- `schema_version: 1`, `concurrent_sessions: 4`, `stabilization_seconds: 15`,
  `observation_seconds: 60`, `sample_interval_seconds: 5`, `samples_per_session: 13`;
  capacity rejection, toàn bộ window responsive và cleanup đều được confirm `true`.
- Start-to-window: `[471, 532, 541, 599]` ms.
- Session 1: WS max/final `90656768/85540864`, private max/final
  `76091392/70496256`, handles `492`, CPU x100 `234`.
- Session 2: WS max/final `88674304/84291584`, private max/final
  `73748480/69083136`, handles `495`, CPU x100 `247`.
- Session 3: WS max/final `87801856/85131264`, private max/final
  `72839168/69836800`, handles `492`, CPU x100 `159`.
- Session 4: WS max/final `84914176/84373504`, private max/final
  `70582272/69517312`, handles `492`, CPU x100 `253`.
- Aggregate WS first/max/final/growth:
  `344858624/349523968/339337216/-5521408` bytes.
- Aggregate private first/max/final/growth:
  `288477184/290643968/278933504/-9543680` bytes.
- Aggregate handles `1968`; aggregate CPU x100 `893`.

Mọi raw sample/interval đáp ứng fixed ceiling: 256 MiB mỗi session, 768 MiB aggregate,
1,200/4,800 handles, aggregate one-core CPU x100 tối đa 10,000 và final growth tối đa 128 MiB.
Record ghi `representative_of_1gib_target: false`, `capacity_claim: false` và chỉ có scope
`development_windows_host`.

## Real-Java parent-crash evidence

Gate thứ ba list đúng một test và pass 1/0. Child Supervisor launch một exact Java owner, publish
record identity cố định, rồi outer chỉ terminate identity-checked Supervisor owner. Java root signal
trong deadline 10 giây, child được reap và guarded root chỉ xóa sau confirmation. Outer monitor ghi
phase concurrency cuối là 1 và final Java bằng 0. Khoảng HWND ngắn của phase này không rơi vào sample
100 ms; readiness nội bộ identity-checked đã pass.

## Ubuntu portable results

Ubuntu format pass; locked workspace all-target tests 56 passed, 0 failed, 0 ignored: library 8,
`core_commands` 1, `foundation_scope` 6, `launch_snapshot` 26, `runtime_validation` 7 và
`store_bootstrap` 8; các target còn lại có 0 test portable. Clippy deny-warnings và release build
đều pass. Đây chỉ là portable Core evidence, không phải Windows/live/containment evidence.

## Metadata, privacy, and artifact cleanup

Locked Cargo metadata có đúng 9 target, không có live/helper target. Foundation scope 6/0 chứng
minh không public export Supervisor/adapter/live surface, production boundary không có
`Command`/shell/live token, CLI từ chối `launch`/`stop`/`session` bằng bounded `UnknownCommand`, và
capability producer vẫn là `NeedsValidation`.

Artifact UTF-8 không BOM dài 2,050 byte giữ nguyên mọi giá trị đo từ phần record/schema đã được
real parser chấp nhận trước khi thêm đúng ba thuộc tính `tested_at_utc`, `host_scope` và
`capacity_claim`; artifact cuối pass audit riêng về exact base-schema-plus-three và có 0 normalized
privacy violation. Sau mọi gate:
Java/javaw, live/probe UUID child, emitted probe executable/process, dry-run root và monitoring job
đều bằng 0; caller environment được restore chính xác.

## Explicit limits and next milestone

- Không có claim RMS compatibility, gameplay, login, credential hay network success.
- Host này không đại diện target 1 GiB; không có long-run, soak hoặc stress qualification.
- Không có public launch API/caller, CLI launch, UI, IPC, background orchestration hay production
  support. Runtime exact vẫn là `NeedsValidation`, không phải `Supported`.
- Live runner là opt-in qualification gate cho developer; game chưa được nối vào user workflow.
- Mốc tiếp theo là thiết kế first internal/public caller boundary và non-technical UI/control
  surface, kèm authorization/lifecycle evidence riêng; các surface đó chưa tồn tại ở mốc này.
