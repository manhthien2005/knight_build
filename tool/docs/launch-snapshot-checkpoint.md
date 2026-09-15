# Zeus HSO LaunchSnapshot — Acceptance Checkpoint

Ngày chốt: 2026-08-22. Lát cắt này bổ sung đầu vào launch bất biến cho Supervisor/process
adapter. Nó không tạo process, không gọi Java/emulator và không thêm launch/stop/session CLI.

## API và ownership

- `CoreState::prepare_launch_snapshot(profile_id, expected_revision)` đọc profile đúng một lần
  và runtime record đúng một lần từ connection duy nhất của Core.
- Profile phải tồn tại, có revision dương/khớp và chưa archive. Failure trả typed error
  `ProfileNotFound`, `InvalidRevision`, `RevisionConflict` hoặc `ProfileArchived`.
- Mỗi snapshot có session UUIDv4 mới, profile UUID/revision, runtime ID và descriptor SHA-256.
- `LaunchSnapshot` chỉ có field riêng tư và getter read-only. Nó không derive `Serialize`, không
  chứa display name, username, password, token, credential hay raw Manager environment.

## Runtime preflight và structured launch input

- Registration/explicit validation luôn full content validation. Trong một Core epoch, lần dùng
  đầu của runtime chưa có cache cũng full-hash toàn bộ JRE; thành công mới ghi metadata
  fingerprint vào cache in-memory keyed bởi runtime ID + descriptor SHA-256.
- Cache là LRU bounded tối đa 16 runtime. Eviction không làm thay đổi Registry và chỉ khiến lần
  dùng sau full-validate lại; mở Core mới bắt đầu với cache rỗng.
- Fast preflight vẫn canonicalize và kiểm file set, permission, symlink/reparse, file identity,
  size và modification time của mọi regular file trong JRE. Descriptor, JRE manifest,
  MicroEmulator JAR và game JAR luôn được hash lại.
- Identity/integrity/Registry mismatch xóa cache entry và trả typed error. Không sửa artifact,
  không tự cập nhật record và không spawn Java.
- Mọi thuộc tính runtime bất biến sau revalidation phải khớp Runtime Registry; descriptor vẫn
  hợp lệ nhưng đổi digest/launch defaults trả `RuntimeRegistryMismatch`. Artifact đổi trả lỗi
  integrity cụ thể từ static validator.
- Snapshot giữ Java executable, MicroEmulator JAR và game JAR dưới dạng `PathBuf` riêng. Không
  materialize classpath hoặc command-line string.
- Main/MIDlet class, screen size, heap, `UseSerialGc`, `DisablePerfData`, file RMS, quiet và
  quit-on-destroy đều xuất phát từ launch defaults đã validate. JVM flags là mảng fixed-size.
- Environment là mảng đúng ba entry có key enum `TEMP`, `TMP`, `TMPDIR`; mọi value trỏ tới temp
  riêng của profile. Snapshot không có map hoặc API nhận environment tùy ý.
- Runtime Registry vẫn giữ `NeedsValidation`; không có đường nâng thành `Supported`.

## Filesystem isolation

- Runtime/profile/artifact/working/home/temp paths trong snapshot đều absolute, canonical, nằm
  dưới đúng runtime hoặc UUID profile root và không đi qua symlink/reparse point.
- `microemu-home` và `temp` được tạo hoặc kiểm tra bằng primitive private directory hiện có:
  Windows dùng DACL foundation; Ubuntu dùng mode `0700`.
- Working directory là profile root. Display name không tham gia bất kỳ filesystem path nào và
  hai profile luôn có working/home/temp khác nhau.

## ProcessLaunchSpec thuần dữ liệu

- `LaunchSnapshot::process_launch_spec()` trả object read-only gồm session/profile UUID, argv
  schema version `1`, absolute Java executable, canonical working directory, bounded
  `Vec<OsString>`, environment fixed-size và stdio policy typed. Không có public constructor nhận
  executable/argv/environment tùy ý.
- Argv materialize theo thứ tự cố định: profile-local `user.home`/`java.io.tmpdir`, heap,
  SerialGC, DisablePerfData, profile-local fatal-error file, classpath MicroEmulator rồi game,
  main class, screen, file RMS, profile UUID, typed `quiet`/`quit`, MIDlet class.
- Classpath separator là `;` trên Windows và `:` trên Ubuntu, vẫn là đúng một `OsString`; path có
  khoảng trắng không bị tách và không có shell/raw command-line string.
- Environment đúng ba entry `TEMP`, `TMP`, `TMPDIR`, `inherit_environment=false`; stdin/stdout/
  stderr đều `Null`.
- Hard bounds: tối đa 32 argument, 4.096 native units mỗi argument và conservative total tối đa
  32.767 Windows command-line units. NUL, artifact path không còn canonical hoặc classpath path
  chứa separator bị từ chối bằng `CoreError::ProcessLaunchSpec`.

## Evidence chạy thực tế

Windows x64, Rust `1.98.0`, `Cargo.lock`:

- `fmt --all --check`: pass.
- `test --locked --workspace --all-targets`: 59 pass, 0 fail; gồm 27 test LaunchSnapshot và
  exact Windows runtime.
- `clippy --locked --workspace --all-targets -- --deny warnings`: pass.
- `build --locked --workspace --release`: pass.

Ubuntu x64 qua wrapper portable, Rust `1.98.0`, `Cargo.lock`:

- `fmt --all --check`: pass.
- `test --locked --workspace --all-targets`: 52 pass, 0 fail; gồm 26 test LaunchSnapshot
  portable.
- `clippy --locked --workspace --all-targets -- --deny warnings`: pass.

Regression coverage gồm cold/full rồi fast path, cache cap/eviction, JRE content/size/file
identity/file set/permission/link-reparse mutation, same-size manifest/MicroEmulator/game JAR
mutation, profile missing/archived/stale, Registry mismatch, UUID path isolation,
display-name independence và inert non-executable Java fixture. Existing CLI tests tiếp tục từ
chối `launch`, `stop` và `session`.

## Số đo preflight

Đo bằng test instrumented, không spawn Java/emulator:

- Exact Windows runtime: cold `2,427,577 us`, hash tổng `127,823,079` byte, trong đó JRE
  `126,057,478` byte; fast `110,562 us`, hash tổng `1,765,601` byte và JRE `0` byte. Lần đo này
  nhanh hơn khoảng `22.0x` và giảm bytes hashed khoảng `72.4x`.
- Ubuntu portable fixture: cold `470 us`, `2,307` byte tổng / `22` byte JRE; fast `350 us`,
  `2,285` byte tổng / `0` byte JRE. Fixture rất nhỏ nên chỉ là evidence nhánh portable, không là
  benchmark hiệu năng production.

## Phạm vi chưa có evidence

- Chưa có process adapter, spawn, containment, admission, session persistence, monitoring, stop
  escalation hoặc crash cleanup; không được tuyên bố Supervisor hoàn tất.
- `ProcessLaunchSpec` chỉ là dữ liệu bàn giao cho process adapter; không gọi `Command`,
  `CreateProcess`, fork/exec hoặc shell và không thêm launch/stop/session CLI.
- Chưa chạy Java/emulator trong lát cắt này. Exact runtime vẫn thiếu các gate RMS thật, hai
  emulator đồng thời, long-run/resource, forced-stop/parent-crash và Ubuntu tuple.
- Cache metadata không giải quyết TOCTOU từ sau preflight tới lúc process adapter mở artifact.
  Chưa chứng minh atomic file identity tại process spawn; adapter/containment milestone phải giữ
  fail-closed ownership và không dùng shell/PATH.
- Không đổi schema DB và không thêm dependency. Snapshot hiện là object in-memory sẵn sàng cho
  process adapter; persistence/admission/request lifecycle thuộc milestone Supervisor riêng.
