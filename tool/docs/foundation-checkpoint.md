# Zeus HSO Foundation — Acceptance Checkpoint

Ngày chốt: 2026-08-22. Mốc này hoàn tất Core foundation, Runtime Registry/static Validator,
Profile Store và diagnostic CLI. Không có code launch/stop, Supervisor, local IPC, log capture,
UI, background polling hoặc auto-update.

## Kết quả kiến trúc

### Core ownership và persistence

- Một process Core giữ exclusive file lock trước khi mở DB; Core thứ hai cùng data root bị từ
  chối.
- Một `rusqlite::Connection`, synchronous; không async runtime, ORM hoặc connection pool.
- SQLite: `foreign_keys=ON`, `journal_mode=DELETE`, `synchronous=FULL`, busy timeout 250 ms,
  `trusted_schema=OFF`, temp store trong memory.
- Schema v1 dùng `STRICT` tables; DB cap 64 MiB bằng `max_page_count` và pre-open size check.
- Schema hiệu lực được preflight qua connection read-only, kể cả version chỉ còn trong live WAL,
  trước mọi PRAGMA có thể đổi journal/limit. Migration DB cũ tạo đúng một last-known-good backup
  bounded trước transaction; schema mới hơn bị từ chối và main DB không đổi byte.
- Không có cột username, password, token, secret hoặc credential.

### Filesystem và isolation

- Windows data root và từng file marker/lock/DB có DACL exact current user SID + `SYSTEM`;
  file managed bị cấp principal rộng sẽ làm startup abort mà không sửa DB; DACL cũ chỉ gồm
  current user + `SYSTEM` có thể được khóa inheritance an toàn. Ubuntu dùng directory `0700`,
  state file `0600` và local-filesystem allowlist.
- Windows chỉ nhận fixed local drive; removable/optical/remote/unknown drive bị từ chối.
- Data/runtime root qua UNC, remote filesystem, symlink hoặc reparse point bị từ chối.
- Profile ID là UUIDv4 do Core sinh; `display_name` không tham gia path.
- Profile directory chỉ kế thừa hai principal hợp lệ và được kiểm lại trước khi commit metadata.
- Exact Windows runtime tree đã được harden riêng 415 item về current user + `SYSTEM`. Validator
  kiểm owner/DACL hoặc uid/mode trên từng directory/file liên quan và không tự sửa permission.

### Runtime Registry

- Descriptor JSON tối đa 64 KiB, manifest tối đa 2 MiB, JRE tối đa 10.000 file/512 MiB và
  `deny_unknown_fields`.
- Path phải relative/normalized, nằm trong exact runtime root và không đi qua link/reparse.
- Validator rehash descriptor, manifest, toàn bộ JRE tree, MicroEmulator JAR và game JAR; kiểm
  file count, size, missing/extra file và Java executable. Không spawn Java.
- Registry insert-only: cùng ID/cùng descriptor digest là idempotent; cùng ID/digest khác bị
  conflict. Pagination keyset tối đa 100.
- Foundation chỉ cho phép persist `NeedsValidation`; không có đường ghi `Supported`.

Exact tuple đã static-pass:

`windows-x64_temurin-11.0.32+9_microemu-2.0.4_ko402`

Game SHA-256:

`6608bb0c77f03749e46165f711e9566dca4e172ce232256497b35faafe74c259`

### Profile Store

- Tên 1–128 Unicode scalar, tối đa 512 UTF-8 byte, không control char hoặc leading/trailing
  whitespace.
- Runtime foreign key bắt buộc; không có custom data path hoặc credential field.
- Revision bắt đầu 1. Rename/bind/archive dùng parameterized SQL với
  `WHERE profile_id = ? AND revision = ?`; stale revision trả `RevisionConflict`.
- Archive lần đầu tăng revision một lần; gọi lại với revision hiện tại trả cùng record và không
  mutation.
- List active/all dùng keyset UUID cursor, stable ordering và cap 100.

## Evidence chạy thực tế

### Windows x64

- Rust `1.98.0` target `x86_64-pc-windows-gnu`.
- Fresh suite tại checkpoint: 27 test (unit + integration), gồm exact descriptor,
  Registry, Profile Store, CLI và reparse/junction cases.
- `cargo fmt --check`, `cargo clippy --all-targets --deny warnings`, release build: pass.
- Release binary: 2,201,600 byte;
  SHA-256 `ceba2d4ef27b8e4f69608fface03ec0bcdfc9fc9af5db2760513b143e97b9202`.
- Disposable CLI acceptance: exact runtime `NeedsValidation`; tạo hai profile, rename/archive
  profile A tới revision 3; stale rename exit 65/`RevisionConflict`; list trả 2 profile; DB
  36,864 byte; không tạo process Java mới.
- Một smoke sample release (không phải performance gate):
  - `init`: 62.34 ms, sampled max working set 8.07 MiB, max private bytes 1.36 MiB.
  - exact static registration: 231.37 ms, sampled max working set 8.85 MiB, max private bytes
    1.80 MiB.

Các số trên là một lần đo ngắn trên máy phát triển, không phải cam kết percentile hoặc idle
daemon gate.

### Ubuntu x64 qua WSL2 Ubuntu

- Rust `1.98.0` target `x86_64-unknown-linux-gnu`; SQLite bundled link bằng Zig `0.16.0`.
- Fresh suite tại checkpoint: 21 test; profile revision/isolation, Core lock, DB schema/future-WAL
  preflight, bounded validator và CLI init đều pass trên `/tmp` local tmpfs.
- `cargo fmt --check`, test, clippy deny warnings và release build: pass.
- Release binary: 2,414,640 byte;
  SHA-256 `b36ee1509ccdb5dea7f9ea7013b94a634d22e70cafb62a05f5b7985ae6f62be5`.
- Đây là compile/adapter evidence trong WSL, chưa phải exact Ubuntu runtime tuple hoặc VPS 1 GiB
  qualification.

### Toolchain provenance

Portable toolchains nằm dưới `.devtools/`; archive tải về nằm trong `.source-cache/`. Cả hai bị
ignore và không đóng gói vào release.

| Artifact | SHA-256 |
|---|---|
| Rustup init Windows x64 | `86478e53f769379d7f0ebfa7c9aa97cb76ca92233f79aa2cc0dbee2efaac73c7` |
| Zig 0.16.0 Windows x64 | `68659eb5f1e4eb1437a722f1dd889c5a322c9954607f5edcf337bc3684a75a7e` |
| Rustup init Linux x64 | `4acc9acc76d5079515b46346a485974457b5a79893cfb01112423c89aeb5aa10` |
| Zig 0.16.0 Linux x64 | `70e49664a74374b48b51e6f3fdfbf437f6395d42509050588bd49abe52ba3d00` |

Nguồn bootstrap:

- `https://static.rust-lang.org/rustup/dist/<target>/rustup-init[.exe]` và file `.sha256` cùng
  endpoint.
- `https://ziglang.org/download/0.16.0/`.

## Dependency inventory

Direct production dependencies được giữ nhỏ:

| Dependency resolved | Vai trò | License expression |
|---|---|---|
| `rusqlite 0.40.2` + bundled SQLite | Một connection/state DB | MIT |
| `serde 1.0.229`, `serde_json 1.0.151` | Strict descriptor/JSON CLI | MIT OR Apache-2.0 |
| `sha2 0.10.9` | Streaming SHA-256 | MIT OR Apache-2.0 |
| `uuid 1.25.0` | Profile UUIDv4 | Apache-2.0 OR MIT |
| `windows-sys 0.61.2` | Native Windows ACL/Known Folder/drive API | MIT OR Apache-2.0 |
| `libc 0.2.189` | Unix uid/mode/statfs adapter | MIT OR Apache-2.0 |

`junction 2.0.0` (MIT) chỉ là Windows dev-dependency để test reparse point thật. `Cargo.lock`
chứa 61 non-workspace packages khi tính mọi target/dev graph; `cargo metadata --locked` không
trả package nào thiếu license expression. Không có Electron/WebView, async runtime, ORM,
connection pool, logging framework hoặc file-lock crate.

## Giả định chưa đủ evidence / gate bắt buộc còn mở

1. Exact Ubuntu JRE/MicroEmulator/game descriptor chưa tồn tại và chưa chạy trên native Ubuntu
   VPS; WSL build không thay thế gate này.
2. Chưa thấy actual file RMS write của game trong profile directory. Smoke cũ chỉ chứng minh
   config/profile-local write.
3. Chưa chạy hai profile thật đồng thời để chứng minh RMS/session state không lẫn.
4. Chưa chạy long-run trên VPS 1–2 vCPU / 1 GiB để đo RSS, CPU, FD/handle và disk growth.
5. Chưa có Supervisor nên chưa chứng minh forced-stop escalation, parent-crash containment,
   PID-reuse/process-birth identity hoặc zero-orphan behavior.
6. Chưa có daemon/local IPC nên chưa có idle 10-minute Core measurement, endpoint permission,
   frame cap/backpressure hoặc reconnect/revision-gap evidence.
7. Chưa có bounded log implementation; vì milestone này không capture log nên cũng chưa có
   rotation/truncation/redaction evidence.
8. DB hard cap/max-page behavior được cấu hình và test pre-open/invariant; chưa fault-inject disk
   full/power loss trên production filesystem.
9. Opaque `microemu-home` ở milestone sau có thể chứa credential/session do game tự ghi dù
   Manager schema không lưu credential; phải tiếp tục bảo vệ toàn cây profile.
10. `GetDriveTypeW=DRIVE_FIXED` không chứng minh một custom data root nằm ngoài OneDrive,
    Dropbox hay thư mục cloud-sync không mang reparse flag. Triển khai phải dùng default
    LocalAppData hoặc xác nhận thủ công path custom không được đồng bộ.
11. Permission hiện bảo vệ khỏi account OS khác, không bảo vệ khỏi malware/process chạy cùng
    user, Administrator/root hoặc `SYSTEM`; đây là trust boundary bắt buộc phải chấp nhận hoặc
    bổ sung hardening vận hành trước production.
12. Linux filesystem adapter cố ý fail-closed theo allowlist ext/XFS/Btrfs/tmpfs/ZFS/overlayfs;
    filesystem VPS khác phải được nhận diện và test durability trước khi thêm vào allowlist.

Không được đổi runtime sang `Supported` trước khi các gate liên quan runtime/process ở trên có
evidence. Không được suy diễn WSL test thành native Ubuntu VPS qualification.

## Điểm bàn giao session sau

Session Supervisor có thể bắt đầu từ `CoreState`, Runtime Registry và Profile Store hiện tại,
nhưng phải giữ các ranh giới sau:

- Mỗi emulator/JAR là process riêng; tuyệt đối không nhúng emulator.
- Launch chỉ dùng absolute canonical paths đã snapshot từ immutable runtime/profile revision.
- Process ownership/containment và stop escalation phải có adapter riêng Windows/Ubuntu và test
  crash/kill thật trước khi nối UI.
- Runtime vẫn `NeedsValidation` trong suốt quá trình thu thập RMS/concurrency/long-run/kill
  evidence.
- Local IPC, bounded logs và UI là milestone sau Supervisor; không ghép vào cùng một bước để
  tránh làm mờ failure domain.
