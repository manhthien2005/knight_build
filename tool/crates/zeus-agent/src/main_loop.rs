//! Agent main loop — vòng lặp trung tâm, đồng bộ, sở hữu mọi state.
//!
//! ## Kiến trúc luồng (AGENT-SPEC §4.2)
//!
//! ```text
//! ┌─ thread realtime (tungstenite blocking) ─────────────────────┐
//! │  phx_join → postgres_changes → mpsc::Sender<CloudEvent>      │
//! └──────────────────────────────────────────────────────────────┘
//!                       │ mpsc
//! ┌─ vòng chính (thread này) ────────────────────────────────────┐
//! │  recv_timeout(1s)                                            │
//! │   ├─ CloudEvent::AccountChanged → apply_config              │
//! │   ├─ CloudEvent::CommandQueued  → dispatch_command          │
//! │   ├─ tick 2 s → đọc snapshot + push telemetry khi đổi      │
//! │   ├─ tick 5 s → đếm VNC client (viewer throttle)           │
//! │   ├─ tick 60 s → heartbeat + device metrics                 │
//! │   └─ waitpid(-1, WNOHANG) → reap zombie (agent là PID 1)   │
//! └──────────────────────────────────────────────────────────────┘
//! ```
//!
//! Không `Arc<Mutex<_>>` chia state giữa hai thread. Thread realtime chỉ
//! đẩy event vào channel — vòng chính là owner duy nhất.
//!
//! ## Startup sequence
//!
//! 1. Đọc `zeus-jar.json` → khai jar contract lên Supabase
//! 2. `fetch_accounts` + `drain_commands` → full state lúc boot
//! 3. Với mỗi account: apply config (B5.1) → reconcile desired_state → start nếu cần
//! 4. Spawn thread realtime; subscribe devices, accounts, commands
//! 5. Bước vào vòng chính
//!
//! ## Reconnect (B2.2)
//!
//! Khi realtime thread gửi `CloudEvent::Disconnected`, vòng chính:
//! 1. Refresh JWT token
//! 2. Dừng cũ, spawn thread mới
//! 3. Gọi `fetch_accounts` + `drain_commands` ngay — realtime có thể đã miss event
//! 4. Reconcile lại toàn bộ state, loại bỏ account đã xóa

#[cfg(unix)]
use std::{
    collections::HashMap,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

#[cfg(unix)]
use crate::{
    launch::AccountPaths,
    process_unix::{self, Child, StopOutcome},
    supabase_realtime::{ChangeType, RealtimeClient, RealtimeEvent},
    supabase_rest::{
        check_restart_precondition, evaluate_apply_config_status, evaluate_restart_status,
        evaluate_start_status, evaluate_stop_status, evaluate_stop_transition, CommandDedupe,
        CommandRow, CommandStatus, ConfigStatus, DeviceHeartbeat, JarManifest, RestartPrecondition,
        RestError, RuntimePayload, StopTransition, SupabaseRest,
    },
};

#[cfg(unix)]
use zeus_core::wire::{
    clear_credentials, clear_settings, clear_snapshot, write_settings, SUPPORTED_VERSION,
};

#[cfg(any(unix, test))]
use zeus_core::wire::{parse_settings, ControlSettings};

// ── SIGTERM/SIGINT signal flag ─────────────────────────────────────────────────

#[cfg(unix)]
static RUNNING: AtomicBool = AtomicBool::new(true);

#[cfg(unix)]
extern "C" fn handle_signal(_: libc::c_int) {
    RUNNING.store(false, Ordering::SeqCst);
}

// ── types ─────────────────────────────────────────────────────────────────────

/// Sự kiện từ thread realtime → vòng chính.
#[cfg(unix)]
#[derive(Debug)]
pub enum CloudEvent {
    /// Hàng `accounts` thay đổi.
    AccountChanged { account_id: String, record: serde_json::Value },
    /// Account mới được thêm (không có trong accounts map hiện tại).
    AccountAdded { record: serde_json::Value },
    /// Account bị xóa.
    AccountDeleted { account_id: String },
    /// Hàng `commands` INSERT với status='queued'.
    CommandQueued { command: CommandRow },
    /// Socket WebSocket đứt — cần reconnect.
    Disconnected { reason: String },
    /// Realtime subscriptions established for all required tables (accounts + commands).
    RealtimeReady,
}

/// State của một account trong vòng chính.
#[cfg(unix)]
struct AccountState {
    /// UUID từ Supabase
    id: String,
    slot_index: i32,
    desired_state: String,   // "running" | "stopped"
    control_version: i32,
    control: serde_json::Value,
    config_version: i32,
    applied_version: i32,    // version đã apply thành công lần gần nhất
    // Credentials — cần để unseal và seed JVM khi start/restart
    username: String,
    server_index: u8,
    character_slot: i16,
    secret_sealed: serde_json::Value,
    runtime_config: serde_json::Value, // heap_max_mib, headless, autostart
    /// JVM đang chạy, nếu có.
    process: Option<Child>,
    /// Snapshot lần đọc trước (để detect thay đổi có nghĩa).
    last_snapshot: Option<serde_json::Value>,
    /// Nhịp telemetry: chỉ push khi thay đổi ý nghĩa hoặc 60s tick.
    last_telemetry_push: Instant,
    /// Số lần restart do crash.
    restarts: u32,
    /// Cờ đánh dấu account đã bị xóa trên cloud và đang trong quá trình dừng JVM an toàn.
    retiring: bool,
    /// Pending Detect Spots scan, if any.
    pending_spot_scan: Option<crate::spot_scan::PendingSpotScan>,
    /// Retained latest Detect Spots scan result.
    last_spot_scan: Option<crate::spot_scan::RetainedSpotScan>,
    /// Pending single-item enhancement, if any.
    pending_enhancement: Option<crate::enhancement::PendingEnhancement>,
    /// Enhancement queue orchestrator tracker.
    pub queue_tracker: crate::enhancement_queue::AccountQueueTracker,
}

/// Cấu hình môi trường agent. Đọc từ biến môi trường lúc boot.
#[cfg(unix)]
pub struct AgentConfig {
    pub device_id: String,
    pub jar_manifest_path: String,
    pub supabase_url: String,
    pub supabase_anon_key: String,
    pub display_num: u32,
    pub external_port: u16,
    pub viewer_base_url: Option<String>,
}

#[cfg(unix)]
impl AgentConfig {
    pub fn from_env() -> Result<Self, String> {
        fn var(key: &str) -> Result<String, String> {
            std::env::var(key).map_err(|_| format!("missing env var: {key}"))
        }
        // ZEUS_DEVICE_ID có thể chưa có lúc from_env() chạy; sẽ được set từ pair_state
        let device_id = std::env::var("ZEUS_DEVICE_ID").unwrap_or_default();
        let supabase_url = std::env::var("SUPABASE_URL")
            .unwrap_or_else(|_| crate::supabase_rest::SUPABASE_URL.to_string());
        let supabase_anon_key = std::env::var("SUPABASE_ANON_KEY")
            .unwrap_or_else(|_| crate::supabase_rest::SUPABASE_ANON_KEY.to_string());
        let display_num = std::env::var("DISPLAY_NUM")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(1);
        let external_port = std::env::var("PORT")
            .ok()
            .and_then(|s| s.parse::<u16>().ok())
            .unwrap_or(6080);
        fn normalize_viewer_base(input: &str) -> String {
            let trimmed = input.trim().trim_end_matches('/');
            if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
                trimmed.to_string()
            } else {
                format!("https://{trimmed}")
            }
        }
        let viewer_base_url = std::env::var("VIEWER_BASE_URL")
            .ok()
            .or_else(|| std::env::var("RAILWAY_PUBLIC_DOMAIN").ok())
            .map(|s| normalize_viewer_base(&s));
        Ok(Self {
            device_id,
            jar_manifest_path: std::env::var("JAR_MANIFEST_PATH")
                .unwrap_or_else(|_| "/opt/knight/game/zeus-jar.json".to_string()),
            supabase_url,
            supabase_anon_key,
            display_num,
            external_port,
            viewer_base_url,
        })
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

/// Interval between enhancement queue orchestrator polling/ticks.
#[cfg(unix)]
pub(crate) const ENHANCEMENT_QUEUE_TICK_INTERVAL: Duration = Duration::from_secs(2);

/// Determines whether the enhancement queue tick should run on this main loop iteration.
/// Enforces bounded cadence and prevents busy-looping.
#[cfg(unix)]
pub(crate) fn should_tick_enhancement_queue(now: Instant, last_tick: Instant) -> bool {
    now.duration_since(last_tick) >= ENHANCEMENT_QUEUE_TICK_INTERVAL
}

/// Identifies whether a command is an enhancement dry-run command based on its canonical discriminator.
#[cfg(unix)]
pub(crate) fn is_enhancement_dry_run_command(kind: &str) -> bool {
    kind == "enhance-item-dry-run"
}

/// Chạy vòng chính cho đến khi nhận SIGTERM (không return bình thường).
///
/// Gọi từ `main()` sau khi pairing xong và `device_id` đã có.
#[cfg(unix)]
pub fn run(cfg: AgentConfig, access_token: String, secret_key_bytes: [u8; 32]) -> ! {
    // Đăng ký SIGTERM/SIGINT handler để PID 1 không bị kernel drop tín hiệu
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = handle_signal as *const () as libc::sighandler_t;
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
    }

    let boot_time = Instant::now();

    eprintln!("[main_loop] starting — device_id={}", cfg.device_id);

    let mut rest = {
        let mut r = SupabaseRest::new(cfg.supabase_url.clone(), cfg.supabase_anon_key.clone());
        r.set_access_token(access_token.clone());
        r
    };

    // Tạo DeviceIdentity từ secret_key_bytes để unseal credentials
    let identity = crate::crypto::DeviceIdentity::from_seed_bytes(&secret_key_bytes);

    // Đọc jar manifest và khai contract.
    let manifest = read_jar_manifest(&cfg.jar_manifest_path);
    if let Some(ref m) = manifest {
        if let Err(e) = rest.announce_jar_contract(&cfg.device_id, m) {
            eprintln!("[main_loop] announce_jar_contract failed: {e}");
        }
    }
    let jar_ctl_version = manifest.as_ref().map(|m| m.ctl_version as i32).unwrap_or(13);

    // Boot: đọc full state.
    let mut accounts = match boot_fetch_accounts(&rest, &cfg.device_id) {
        Ok(accs) => accs,
        Err(e) => {
            eprintln!("[boot] initial fetch_accounts failed: {e}, starting with empty local cache; will recover on RealtimeReady");
            HashMap::new()
        }
    };

    // Ghi potato.ctl ban đầu "0 3" cho tất cả slots hiện có — fix Issue #24
    for acc in accounts.values() {
        let paths = AccountPaths::for_slot(acc.slot_index);
        write_potato_ctl_for_path(&paths, "0 3");
    }

    // Apply config + reconcile tất cả account lúc boot (B5.1, B7.3).
    // Startup stagger: 3s giữa các account để tránh OOM Killer — Issue #90
    let mut first = true;
    for acc in accounts.values_mut() {
        if !first {
            std::thread::sleep(Duration::from_secs(3));
        }
        first = false;
        // Dọn sạch snapshot/control rác của phiên trước — Issue #82
        let paths = AccountPaths::for_slot(acc.slot_index);
        let _ = clear_snapshot(&paths.home);
        let _ = clear_settings(&paths.home);
        crate::spot_scan::clean_spot_files(&paths.home);
        let _ = crate::enhancement_queue::recover_account_enhancement_queue_on_boot(
            &paths.home,
            &cfg.device_id,
            &acc.id,
            &rest,
        );
        // prepare_directories trước khi spawn — Issue #16
        let _ = crate::launch::prepare_directories(&paths);
        let _ = try_apply_config(acc, jar_ctl_version, &rest);
        reconcile_desired_state(acc, &rest, &identity);
    }

    // Startup Barrier (R5A.1):
    // Stale sidecar files were purged in the account loop above.
    // Realtime thread must NOT be spawned and command consumption must NOT begin
    // until abandoned detect-spots recovery has completed successfully.
    let max_barrier_attempts = 5;
    let mut attempt = 1;
    loop {
        let recovery_res = rest.recover_abandoned_spot_scans(&cfg.device_id);
        match crate::supabase_rest::evaluate_startup_barrier_step(
            &recovery_res,
            attempt,
            max_barrier_attempts,
        ) {
            crate::supabase_rest::StartupBarrierAction::ProceedToRealtime { recovered_count } => {
                if recovered_count > 0 {
                    eprintln!(
                        "[recovery] startup barrier resolved: {recovered_count} abandoned detect-spots command(s) recovered"
                    );
                } else {
                    eprintln!("[recovery] startup barrier resolved: no abandoned detect-spots commands");
                }
                break;
            }
            crate::supabase_rest::StartupBarrierAction::Retry { attempt: cur_attempt, max_retries } => {
                let err_msg = recovery_res.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
                eprintln!(
                    "[recovery] startup barrier unresolved (attempt {cur_attempt}/{max_retries}): {err_msg}; retrying in 2s"
                );
                attempt += 1;
                std::thread::sleep(Duration::from_secs(2));
            }
            crate::supabase_rest::StartupBarrierAction::FatalAbort { attempts_exhausted } => {
                let err_msg = recovery_res.as_ref().err().map(|e| e.to_string()).unwrap_or_default();
                eprintln!(
                    "[recovery] FATAL: startup barrier failed after {attempts_exhausted} attempts: {err_msg}; aborting startup"
                );
                std::process::exit(1);
            }
        }
    }

    // Spawn thread realtime.
    let (tx, rx) = mpsc::channel::<CloudEvent>();
    spawn_realtime_thread(
        cfg.supabase_url.clone(),
        cfg.supabase_anon_key.clone(),
        access_token.clone(),
        tx.clone(),
    );

    let mut dedupe = CommandDedupe::new(512);
    let mut retired_tombstones = std::collections::HashSet::<String>::new();

    // Ticks
    let mut last_snapshot_tick = Instant::now();
    let mut last_queue_tick = Instant::now();
    let mut last_heartbeat_tick = Instant::now();
    let mut last_viewer_tick = Instant::now();
    let mut last_token_refresh = Instant::now();
    let mut viewer_client_count: u32 = 0;
    let mut viewer_zero_since: Option<Instant> = None;
    let mut current_access_token = access_token;

    eprintln!("[main_loop] entering main loop");

    loop {
        // ── SIGTERM/SIGINT check ──────────────────────────────────────────
        if !RUNNING.load(Ordering::SeqCst) {
            eprintln!("[main_loop] received shutdown signal, performing graceful shutdown");
            // Dừng tất cả JVM
            for acc in accounts.values_mut() {
                if let Some(child) = acc.process.take() {
                    if child.pgid_alive() {
                        process_unix::stop(&child, Duration::from_secs(5));
                    }
                }
                let paths = AccountPaths::for_slot(acc.slot_index);
                let _ = clear_credentials(&paths.home);
            }
            // Báo offline lên Supabase
            if let Err(e) = rest.set_device_status(&cfg.device_id, "offline") {
                eprintln!("[shutdown] set_device_status failed: {e}");
            }
            std::process::exit(0);
        }

        // ── reap zombies (PID 1 obligation) ───────────────────────────────
        reap_zombies();

        // ── JWT token refresh mỗi 45 phút — Issues #11, #51 ─────────────
        if last_token_refresh.elapsed() >= Duration::from_secs(45 * 60) {
            last_token_refresh = Instant::now();
            match rest.sign_in_as_device(&cfg.device_id, &identity.public_key_sec1) {
                Ok(new_token) => {
                    rest.set_access_token(new_token.clone());
                    current_access_token = new_token;
                    eprintln!("[auth] JWT token refreshed successfully");
                }
                Err(e) => {
                    eprintln!("[auth] JWT refresh failed: {e}");
                }
            }
        }

        // ── receive realtime event (với timeout 250ms để poll sidecar nhạy) ──────
        match rx.recv_timeout(Duration::from_millis(250)) {
            Ok(event) => handle_cloud_event(
                event,
                &mut accounts,
                jar_ctl_version,
                &mut rest,
                &cfg,
                &identity,
                &mut current_access_token,
                &tx,
                &mut dedupe,
                &mut retired_tombstones,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {} // bình thường, xử lý ticks bên dưới
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[main_loop] realtime channel closed unexpectedly, reconnecting");
                spawn_realtime_thread(
                    cfg.supabase_url.clone(),
                    cfg.supabase_anon_key.clone(),
                    current_access_token.clone(),
                    tx.clone(),
                );
            }
        }

        // ── poll pending spot scans & enhancements (non-blocking) ─────────
        poll_pending_spot_scans(&mut accounts, &rest);
        poll_pending_enhancements(&mut accounts, &rest);

        let now = Instant::now();

        // ── tick 2 s: đọc snapshot + telemetry ───────────────────────────
        if now.duration_since(last_snapshot_tick) >= Duration::from_secs(2) {
            last_snapshot_tick = now;
            for acc in accounts.values_mut() {
                tick_snapshot_telemetry(acc, &rest);
            }

            // Kiểm tra Xvnc socket còn sống — Issue #27
            let display_num = cfg.display_num;
            let x_socket = format!("/tmp/.X11-unix/X{display_num}");
            if !std::path::Path::new(&x_socket).exists() {
                eprintln!("[main_loop] FATAL: Xvnc socket {x_socket} missing, exiting to trigger restart");
                std::process::exit(1);
            }

            // Retry retirement cho các account đang pending cleanup — Invariant C
            let retiring_ids: Vec<String> = accounts
                .values()
                .filter(|a| a.retiring)
                .map(|a| a.id.clone())
                .collect();
            for id in retiring_ids {
                retire_account_safely(&mut accounts, &id, &mut retired_tombstones);
            }

            // Auto-restart crashed JVMs — Issue #19 (chỉ autostart account không trong trạng thái retiring/tombstoned)
            for acc in accounts.values_mut() {
                if !acc.retiring && !retired_tombstones.contains(&acc.id) && acc.desired_state == "running" && acc.process.as_ref().map(|p| !p.alive()).unwrap_or(true) {
                    eprintln!("[reconcile] account={} crash detected, restarting", acc.id);
                    acc.restarts += 1;
                    reconcile_desired_state(acc, &rest, &identity);
                }
            }
        }

        // ── tick 2 s: enhancement queue orchestrator ─────────────────────
        if should_tick_enhancement_queue(now, last_queue_tick) {
            last_queue_tick = now;
            for acc in accounts.values_mut() {
                if !acc.retiring && !retired_tombstones.contains(&acc.id) {
                    let paths = AccountPaths::for_slot(acc.slot_index);
                    let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
                    crate::enhancement_queue::tick_account_enhancement_queue(
                        &paths.home,
                        &cfg.device_id,
                        &acc.id,
                        is_alive,
                        acc.pending_enhancement.is_some(),
                        &mut acc.queue_tracker,
                        &rest,
                    );
                }
            }
        }

        // ── tick 5 s: VNC viewer count ────────────────────────────────────
        if now.duration_since(last_viewer_tick) >= Duration::from_secs(5) {
            last_viewer_tick = now;
            // Đọc /proc/net/tcp thay vì fork ss — Issue #53
            let count = count_vnc_clients(cfg.display_num, cfg.external_port);
            if count == 0 && viewer_client_count > 0 {
                // Bắt đầu đếm hysteresis 15s (B8.3)
                viewer_zero_since = Some(now);
            } else if count > 0 {
                viewer_zero_since = None;
            }
            // Hysteresis: chỉ tắt throttle sau 15s liên tục không có client
            let throttle_off = viewer_zero_since
                .map(|t| now.duration_since(t) >= Duration::from_secs(15))
                .unwrap_or(false);
            if count > 0 || throttle_off {
                let mode = if count > 0 { "1 0" } else { "0 3" };
                // Ghi vào TẤT CẢ slots đang active — Issue #05, #92
                for acc in accounts.values() {
                    let paths = AccountPaths::for_slot(acc.slot_index);
                    write_potato_ctl_for_path(&paths, mode);
                }
                // Reset viewer_zero_since sau khi chuyển về throttle — Issue #24
                if throttle_off {
                    viewer_zero_since = None;
                }
            }
            viewer_client_count = count;
        }

        // ── tick 60 s: heartbeat + device metrics ─────────────────────────
        if now.duration_since(last_heartbeat_tick) >= Duration::from_secs(60) {
            last_heartbeat_tick = now;
            tick_heartbeat(&rest, &cfg.device_id, &mut accounts, boot_time);
        }
    }
}

// ── handlers ──────────────────────────────────────────────────────────────────

#[cfg(unix)]
#[allow(clippy::too_many_arguments)]
fn handle_cloud_event(
    event: CloudEvent,
    accounts: &mut HashMap<String, AccountState>,
    jar_ctl_version: i32,
    rest: &mut SupabaseRest,
    cfg: &AgentConfig,
    identity: &crate::crypto::DeviceIdentity,
    current_access_token: &mut String,
    tx: &mpsc::Sender<CloudEvent>,
    dedupe: &mut CommandDedupe,
    retired_tombstones: &mut std::collections::HashSet<String>,
) {
    match event {
        CloudEvent::AccountChanged { account_id, record } => {
            // Filter theo device_id — Issue #100 (multi-node collision)
            if let Some(dev_id) = record["device_id"].as_str() {
                if dev_id != cfg.device_id {
                    return; // Lệnh thuộc về node khác
                }
            }

            if crate::supabase_rest::evaluate_account_admission(
                &account_id,
                retired_tombstones.contains(&account_id),
            ) == crate::supabase_rest::AccountAdmissionDecision::RejectedTombstoned
            {
                eprintln!("[cloud] account {account_id} is tombstoned/retired, ignoring update/insertion");
                return;
            }

            if let Some(acc) = accounts.get_mut(&account_id) {
                if acc.retiring {
                    eprintln!("[cloud] account {account_id} is retiring, ignoring update");
                    return;
                }
                // Cập nhật config từ record nếu có
                if let Some(cv) = record["control_version"].as_i64() {
                    acc.control_version = cv as i32;
                }
                if let Some(ctrl) = record.get("control") {
                    acc.control = ctrl.clone();
                }
                if let Some(cfgv) = record["config_version"].as_i64() {
                    acc.config_version = cfgv as i32;
                }
                if let Some(ds) = record["desired_state"].as_str() {
                    acc.desired_state = ds.to_string();
                }
                // Cập nhật credentials nếu thay đổi — Issue #41
                if let Some(ss) = record.get("secret_sealed") {
                    acc.secret_sealed = ss.clone();
                }
                if let Some(si) = record["server_index"].as_i64() {
                    acc.server_index = si.clamp(0, 7) as u8;
                }
                if let Some(un) = record["username"].as_str() {
                    acc.username = un.to_string();
                }

                // Apply config nếu version mới (B5)
                let _ = try_apply_config(acc, jar_ctl_version, rest);
                // Reconcile desired_state (B7.3)
                reconcile_desired_state(acc, rest, identity);
            } else {
                // Account mới (không có trong map hiện tại) — Issue #12
                // Kiểm tra device_id thuộc về node này
                let dev_id = record["device_id"].as_str().unwrap_or("");
                if !dev_id.is_empty() && dev_id != cfg.device_id {
                    return;
                }
                eprintln!("[cloud] new account {account_id}, initializing");
                if let Some(acc) = make_account_state_from_record(&record) {
                    let paths = AccountPaths::for_slot(acc.slot_index);
                    let _ = crate::launch::prepare_directories(&paths);
                    let _ = clear_snapshot(&paths.home);
                    let mut acc = acc;
                    let _ = try_apply_config(&mut acc, jar_ctl_version, rest);
                    reconcile_desired_state(&mut acc, rest, identity);
                    accounts.insert(account_id, acc);
                }
            }
        }

        CloudEvent::AccountAdded { record } => {
            let account_id = record["id"].as_str().unwrap_or("").to_string();
            if account_id.is_empty() { return; }
            // Filter device_id
            let dev_id = record["device_id"].as_str().unwrap_or("");
            if !dev_id.is_empty() && dev_id != cfg.device_id {
                return;
            }
            if accounts.contains_key(&account_id)
                || crate::supabase_rest::evaluate_account_admission(
                    &account_id,
                    retired_tombstones.contains(&account_id),
                ) == crate::supabase_rest::AccountAdmissionDecision::RejectedTombstoned
            {
                eprintln!("[cloud] account {account_id} already exists or is tombstoned, ignoring addition");
                return;
            }
            eprintln!("[cloud] account added {account_id}");
            if let Some(acc) = make_account_state_from_record(&record) {
                let paths = AccountPaths::for_slot(acc.slot_index);
                let _ = crate::launch::prepare_directories(&paths);
                let _ = clear_snapshot(&paths.home);
                crate::spot_scan::clean_spot_files(&paths.home);
                let mut acc = acc;
                let _ = try_apply_config(&mut acc, jar_ctl_version, rest);
                reconcile_desired_state(&mut acc, rest, identity);
                accounts.insert(account_id, acc);
            }
        }

        CloudEvent::AccountDeleted { account_id } => {
            eprintln!("[cloud] realtime account deleted {account_id}, retiring safely");
            retired_tombstones.insert(account_id.clone());
            retire_account_safely(accounts, &account_id, retired_tombstones);
        }

        CloudEvent::CommandQueued { command } => {
            if !dedupe.record_if_new(&command.id) {
                eprintln!("[command] duplicate command id={} already processed, suppressing", command.id);
                return;
            }
            dispatch_command(command, accounts, rest, cfg, identity, jar_ctl_version, retired_tombstones);
        }

        CloudEvent::RealtimeReady => {
            eprintln!("[main_loop] realtime ready: recovering queued commands");
            recover_queued_commands(rest, &cfg.device_id, accounts, cfg, identity, jar_ctl_version, dedupe, retired_tombstones);
        }

        CloudEvent::Disconnected { reason } => {
            eprintln!("[main_loop] realtime disconnected: {reason}, reconnecting");
            // Refresh JWT trước khi reconnect — Issues #50, #104
            let sign_in_res = rest.sign_in_as_device(&cfg.device_id, &identity.public_key_sec1);
            let decision = crate::supabase_rest::evaluate_reconnect_token_refresh(
                current_access_token,
                sign_in_res,
            );
            if decision.refreshed {
                rest.set_access_token(decision.rest_token);
                *current_access_token = decision.realtime_token.clone();
                eprintln!("[auth] JWT token refreshed before reconnect");
            } else {
                eprintln!("[auth] JWT refresh on disconnect failed, preserving current token");
            }

            spawn_realtime_thread(
                cfg.supabase_url.clone(),
                cfg.supabase_anon_key.clone(),
                decision.realtime_token,
                tx.clone(),
            );
        }
    }
}

// ── config apply (B5) ─────────────────────────────────────────────────────────

#[cfg(any(unix, test))]
pub(crate) fn evaluate_control_version_gate(acc_control_version: i32, jar_ctl_version: i32) -> bool {
    if jar_ctl_version == 14 {
        acc_control_version == 13 || acc_control_version == 14
    } else {
        acc_control_version == jar_ctl_version
    }
}

#[cfg(unix)]
fn try_apply_config(
    acc: &mut AccountState,
    jar_ctl_version: i32,
    rest: &SupabaseRest,
) -> Result<(), &'static str> {
    // Đã apply rồi, không apply lại.
    if acc.applied_version == acc.config_version && acc.applied_version != 0 {
        return Ok(());
    }

    // Version gate (CLOUD-SPEC §3, B5.1).
    // New zeus-agent accepts database account control_version 13 and 14 when jar is 14.
    let version_compatible = evaluate_control_version_gate(acc.control_version, jar_ctl_version);

    if !version_compatible {
        eprintln!(
            "[config] account={} version_mismatch: control_version={} != jar_ctl_version={}",
            acc.id, acc.control_version, jar_ctl_version
        );
        if let Err(e) = rest.set_config_status(&acc.id, ConfigStatus::VersionMismatch, None, None) {
            eprintln!("[config] set_config_status failed: {e}");
        }
        return Err("config version mismatch");
    }

    // Dựng ControlSettings từ JSONB — Issues #04, #52
    let settings = match build_control_settings(&acc.control, acc.control_version) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[config] account={} build_settings failed: {e}", acc.id);
            if let Err(re) = rest.set_config_status(
                &acc.id,
                ConfigStatus::Error,
                Some(&e),
                None,
            ) {
                eprintln!("[config] set_config_status failed: {re}");
            }
            return Err("config was not applied");
        }
    };

    // Ghi file. write_settings(microemu_home, settings) — FIX Issue #01
    let paths = AccountPaths::for_slot(acc.slot_index);
    match write_settings(&paths.home, &settings) {
        Ok(_) => {
            eprintln!("[config] account={} applied config_version={}", acc.id, acc.config_version);
            acc.applied_version = acc.config_version;
            if let Err(e) = rest.set_config_status(
                &acc.id,
                ConfigStatus::Applied,
                None,
                Some(acc.config_version),
            ) {
                eprintln!("[config] set_config_status failed: {e}");
            }

            // B5.2: nav.detectSpots one-shot — reset sau khi ghi.
            if settings_has_detect_spots(&acc.control) {
                if let Err(e) = rest.clear_detect_spots(&acc.id) {
                    eprintln!("[config] clear_detect_spots failed: {e}");
                }
            }
            Ok(())
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("[config] account={} write_settings failed: {msg}", acc.id);
            if let Err(re) = rest.set_config_status(&acc.id, ConfigStatus::Error, Some(&msg), None) {
                eprintln!("[config] set_config_status failed: {re}");
            }
            Err("config was not applied")
        }
    }
}

/// Xây ControlSettings từ JSONB control block — Issues #04, #52.
///
/// Nếu control là {} hoặc thiếu key, dùng ControlSettings::default() làm nền.
/// Parse từ in-memory string thay vì NamedTempFile.
#[cfg(any(unix, test))]
pub(crate) fn build_control_settings(
    control: &serde_json::Value,
    control_version: i32,
) -> Result<ControlSettings, String> {
    if control_version != 13 && control_version != 14 {
        return Err(format!("unsupported control_version={control_version}"));
    }

    // Dùng ControlSettings::default() làm nền — fix Issue #52
    let default_settings = ControlSettings::default();

    let map = match control.as_object() {
        Some(m) => m,
        None => {
            // Không phải JSON object → dùng default
            return Ok(default_settings);
        }
    };

    // Nếu control trống hoặc chỉ có v= → dùng default
    if map.is_empty() || (map.len() == 1 && map.contains_key("v")) {
        return Ok(default_settings);
    }

    // Serialize thành text format mà zeus-control.txt dùng, dùng control_version
    use std::fmt::Write as FmtWrite;
    let mut text = String::new();
    writeln!(text, "v={}", control_version).unwrap();
    for (k, v) in map {
        if k == "v" { continue; }
        let val = match v {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
            serde_json::Value::Null => continue,
            other => other.to_string(),
        };
        writeln!(text, "{k}={val}").unwrap();
    }

    // Parse bằng parse_settings của zeus-core — fails closed on unknown keys or invalid values
    parse_settings(&text).map_err(|e| format!("parse_settings: {e}"))
}

#[cfg(unix)]
fn settings_has_detect_spots(control: &serde_json::Value) -> bool {
    control
        .get("nav.detectSpots")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || control
            .get("nav.detectSpots")
            .and_then(|v| v.as_str())
            .map(|s| s == "1" || s == "true")
            .unwrap_or(false)
}

// ── credential seeding ────────────────────────────────────────────────────────

/// Unseal và seed credentials vào RMS trước khi khởi JVM — Issues #06, #35
#[cfg(unix)]
fn seed_account_credentials(
    acc: &AccountState,
    identity: &crate::crypto::DeviceIdentity,
) -> Result<(), String> {
    // Kiểm tra secret_sealed có đủ fields không — Issue #35
    if acc.secret_sealed.get("alg").is_none() {
        return Err("missing sealed credentials".to_string());
    }

    let sealed: crate::crypto::SealedSecret = serde_json::from_value(acc.secret_sealed.clone())
        .map_err(|e| format!("deserialize sealed failed: {e}"))?;

    let plaintext = crate::crypto::unseal(identity, &sealed)
        .map_err(|e| format!("unseal failed: {e}"))?;

    // Kiểm tra server_index trong khoảng hợp lệ 0..7 — Issue #87
    let server_index = acc.server_index.min(7);

    let paths = AccountPaths::for_slot(acc.slot_index);
    crate::crypto::seed_then_forget(&paths.home, plaintext, server_index)
        .map_err(|e| format!("seed_credentials failed: {e}"))?;

    // plaintext bị zero khi drop (PlaintextCredentials::Drop)
    Ok(())
}

// ── reconcile desired state (B7.3) ────────────────────────────────────────────

#[cfg(unix)]
fn reconcile_desired_state(
    acc: &mut AccountState,
    rest: &SupabaseRest,
    identity: &crate::crypto::DeviceIdentity,
) {
    if acc.retiring {
        eprintln!("[reconcile] account={} is retiring, skipping desired state reconciliation", acc.id);
        return;
    }
    let running = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
    match acc.desired_state.as_str() {
        "running" if !running => {
            acc.process = None;
            eprintln!("[reconcile] account={} slot={}: starting", acc.id, acc.slot_index);
            let paths = AccountPaths::for_slot(acc.slot_index);
            // prepare_directories trước khi spawn — Issue #16
            let _ = crate::launch::prepare_directories(&paths);

            // Dọn sạch snapshot cũ — Issue #82
            let _ = clear_snapshot(&paths.home);

            // Ghi potato.ctl ban đầu "0 3" — Issue #24
            write_potato_ctl_for_path(&paths, "0 3");

            // Seed credentials trước khi start JVM — đảm bảo invariant trước khi spawn
            if let Err(e) = seed_account_credentials(acc, identity) {
                eprintln!(
                    "[reconcile] account={} credential preparation failed: {e}, aborting spawn",
                    acc.id
                );
                return;
            }

            // Validate character_slot before JVM process spawn — CHAR-SLOT-02
            if let Err(err_msg) = crate::supabase_rest::validate_character_slot(acc.character_slot) {
                eprintln!(
                    "[reconcile] account={} invalid character_slot={}: {}, aborting spawn",
                    acc.id, acc.character_slot, err_msg
                );
                let _ = rest.set_config_status(
                    &acc.id,
                    crate::supabase_rest::ConfigStatus::Error,
                    Some(&format!("invalid character_slot: {}", acc.character_slot)),
                    None,
                );
                return;
            }

            // Đọc runtime config — Issue #49
            let mut spec = crate::launch::LaunchSpec::default_for_paths(paths.clone());
            spec.character_slot = acc.character_slot;

            // Áp dụng heap_max_mib và headless từ acc.runtime_config
            if let Some(heap_mib) = acc.runtime_config["heap_max_mib"].as_u64() {
                spec.heap.maximum_mib = heap_mib as u32;
            }
            if let Some(headless) = acc.runtime_config["headless"].as_bool() {
                spec.headless = headless;
            }

            // Thêm -Xshare:auto cho CDS — Issue #107
            let mut command = spec.command();

            // Truyền environment — Issue #34
            let display_num = std::env::var("DISPLAY_NUM").ok()
                .and_then(|s| s.parse::<u32>().ok()).unwrap_or(1);
            let display = format!(":{display_num}");
            command.envs(spec.environment(&display));

            // KHÔNG gọi prepare() riêng — spawn đã gọi — Issue #48
            match crate::process_unix::spawn(command) {
                Ok(child) => {
                    eprintln!("[reconcile] account={} spawned pid={}", acc.id, child.pid);
                    // Báo cáo process_state ngay — Issue #39
                    let now_str = crate::supabase_rest::now_rfc3339();
                    if let Err(e) = rest.push_runtime(
                        &acc.id,
                        &RuntimePayload {
                            process_state: "running".into(),
                            pid: Some(child.pid),
                            ram_mb: None,
                            cpu_pct: None,
                            snapshot_version: None,
                            snapshot: None,
                            restarts: Some(acc.restarts),
                            updated_at: now_str,
                        },
                    ) {
                        eprintln!("[reconcile] push_runtime failed: {e}");
                    }
                    acc.process = Some(child);
                }
                Err(e) => {
                    eprintln!("[reconcile] account={} spawn failed: {e}", acc.id);
                }
            }
        }
        "stopped" => {
            let (has_process, is_alive) = acc
                .process
                .as_ref()
                .map(|p| (true, p.pgid_alive()))
                .unwrap_or((false, false));

            let outcome = if is_alive {
                if let Some(ref child) = acc.process {
                    eprintln!("[reconcile] account={} slot={}: stopping", acc.id, acc.slot_index);
                    let res = process_unix::stop(child, Duration::from_secs(5));
                    match res {
                        StopOutcome::AlreadyGone | StopOutcome::Terminated => {
                            eprintln!("[reconcile] account={}: stopped cleanly", acc.id);
                        }
                        StopOutcome::Killed => {
                            eprintln!("[reconcile] account={}: killed", acc.id);
                        }
                        StopOutcome::Failed => {
                            eprintln!(
                                "[reconcile] account={}: stop failed, process group still alive",
                                acc.id
                            );
                        }
                    }
                    Some(res)
                } else {
                    None
                }
            } else {
                None
            };

            match evaluate_stop_transition(has_process, is_alive, outcome) {
                StopTransition::ConfirmedStopped => {
                    acc.process = None;
                    // Xóa credentials và snapshot — Issues #32, #89
                    let paths = AccountPaths::for_slot(acc.slot_index);
                    if let Err(e) = clear_credentials(&paths.home) {
                        eprintln!("[reconcile] clear_credentials failed: {e}");
                    }
                    if let Err(e) = clear_snapshot(&paths.home) {
                        eprintln!("[reconcile] clear_snapshot failed: {e}");
                    }
                }
                StopTransition::FailedStillAlive => {
                    eprintln!(
                        "[reconcile] account={}: stop failed, keeping live process handle",
                        acc.id
                    );
                }
            }
        }
        _ => {} // state đã đúng
    }
}

// ── command dispatch (B7) ─────────────────────────────────────────────────────

#[cfg(unix)]
fn dispatch_command(
    cmd: CommandRow,
    accounts: &mut HashMap<String, AccountState>,
    rest: &SupabaseRest,
    cfg: &AgentConfig,
    identity: &crate::crypto::DeviceIdentity,
    jar_ctl_version: i32,
    retired_tombstones: &std::collections::HashSet<String>,
) {
    // Issue #98: lọc device_id, bỏ qua lệnh không thuộc node này
    if let Some(ref cmd_device_id) = cmd.device_id {
        if cmd_device_id != &cfg.device_id {
            eprintln!("[command] id={} belongs to device={}, ignoring", cmd.id, cmd_device_id);
            return; // Không gọi finish_command để tránh phá node khác
        }
    }

    // B7.2: TTL check — Issue #25
    let now_str = crate::supabase_rest::now_rfc3339();
    if cmd.expires_at < now_str {
        eprintln!("[command] id={} expired, marking as expired", cmd.id);
        if let Err(e) = rest.finish_command(&cmd.id, CommandStatus::Expired, Some("TTL elapsed")) {
            eprintln!("[command] finish_command (expired) failed: {e}");
        }
        return;
    }

    eprintln!("[command] id={} type={}", cmd.id, cmd.kind);

    let account_id = match &cmd.account_id {
        Some(id) => id.clone(),
        None => {
            // Device-level command (open-viewer, close-viewer không có account_id).
            match cmd.kind.as_str() {
                "open-viewer" => {
                    // Issue #10, #57: tạo viewer URL từ env
                    let viewer_url = cfg.viewer_base_url.as_ref()
                        .map(|base| format!("{base}/vnc.html?autoconnect=1&resize=scale&path=websockify"))
                        .unwrap_or_else(|| "noVNC not configured".to_string());
                    // Ghi potato.ctl "1 0" cho tất cả slots
                    for acc in accounts.values() {
                        let paths = AccountPaths::for_slot(acc.slot_index);
                        write_potato_ctl_for_path(&paths, "1 0");
                    }
                    // Tính expires_at = now + 15 phút (900 giây)
                    let expires_rfc = crate::supabase_rest::rfc3339_offset_from_now(900);
                    let _ = rest.set_viewer(&cfg.device_id, Some((&viewer_url, &expires_rfc)));
                }
                "close-viewer" => {
                    for acc in accounts.values() {
                        let paths = AccountPaths::for_slot(acc.slot_index);
                        write_potato_ctl_for_path(&paths, "0 3");
                    }
                    if let Err(e) = rest.set_viewer(&cfg.device_id, None) {
                        eprintln!("[command] set_viewer(None) failed: {e}");
                    }
                }
                other => eprintln!("[command] unknown device command: {other}"),
            }
            if let Err(e) = rest.finish_command(&cmd.id, CommandStatus::Success, None) {
                eprintln!("[command] finish_command failed: {e}");
            }
            return;
        }
    };

    if retired_tombstones.contains(&account_id) {
        eprintln!("[command] account_id={account_id} is tombstoned/retired, rejecting command {}", cmd.kind);
        if let Err(e) = rest.finish_command(
            &cmd.id,
            CommandStatus::Failed,
            Some("account is deleted or retiring"),
        ) {
            eprintln!("[command] finish_command failed: {e}");
        }
        return;
    }

    let acc = match accounts.get_mut(&account_id) {
        Some(a) => a,
        None => {
            eprintln!("[command] account_id={account_id} not found");
            if let Err(e) = rest.finish_command(
                &cmd.id,
                CommandStatus::Failed,
                Some("account not found"),
            ) {
                eprintln!("[command] finish_command failed: {e}");
            }
            return;
        }
    };

    if acc.retiring {
        eprintln!("[command] account_id={account_id} is retiring/deleted, rejecting command {}", cmd.kind);
        if let Err(e) = rest.finish_command(
            &cmd.id,
            CommandStatus::Failed,
            Some("account is deleted or retiring"),
        ) {
            eprintln!("[command] finish_command failed: {e}");
        }
        return;
    }

    match cmd.kind.as_str() {
        "start" => {
            acc.desired_state = "running".to_string();
            // Sync desired_state lên DB — Issues #14, #97
            if let Err(e) = rest.set_account_desired_state(&account_id, "running") {
                eprintln!("[command] set_account_desired_state failed: {e}");
            }
            reconcile_desired_state(acc, rest, identity);
            let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
            let (status, msg) = evaluate_start_status(is_alive);
            if let Err(e) = rest.finish_command(&cmd.id, status, msg) {
                eprintln!("[command] finish_command failed: {e}");
            }
        }
        "stop" => {
            acc.desired_state = "stopped".to_string();
            // Sync desired_state lên DB — Issues #14, #97
            if let Err(e) = rest.set_account_desired_state(&account_id, "stopped") {
                eprintln!("[command] set_account_desired_state failed: {e}");
            }
            reconcile_desired_state(acc, rest, identity);
            let is_alive = acc.process.as_ref().map(|p| p.pgid_alive()).unwrap_or(false);
            let (status, msg) = evaluate_stop_status(is_alive);
            if let Err(e) = rest.finish_command(&cmd.id, status, msg) {
                eprintln!("[command] finish_command failed: {e}");
            }
        }
        "restart" => {
            // Issue #38: đợi process cũ dừng hẳn trước khi spawn mới
            acc.desired_state = "stopped".to_string();
            if let Err(e) = rest.set_account_desired_state(&account_id, "stopped") {
                eprintln!("[command] set_account_desired_state failed: {e}");
            }
            reconcile_desired_state(acc, rest, identity);

            let old_is_alive = acc.process.as_ref().map(|p| p.pgid_alive()).unwrap_or(false);
            match check_restart_precondition(old_is_alive) {
                RestartPrecondition::BlockedOldProcessAlive => {
                    eprintln!(
                        "[command] restart: account {} old process group failed to stop; aborting replacement spawn",
                        account_id
                    );
                    if let Err(e) = rest.finish_command(
                        &cmd.id,
                        CommandStatus::Failed,
                        Some("old process failed to stop"),
                    ) {
                        eprintln!("[command] finish_command failed: {e}");
                    }
                    return;
                }
                RestartPrecondition::Proceed => {}
            }

            // Đợi 500ms để process cũ có thời gian tắt
            std::thread::sleep(Duration::from_millis(500));
            acc.desired_state = "running".to_string();
            if let Err(e) = rest.set_account_desired_state(&account_id, "running") {
                eprintln!("[command] set_account_desired_state failed: {e}");
            }
            reconcile_desired_state(acc, rest, identity);
            let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
            let (status, msg) = evaluate_restart_status(is_alive);
            if let Err(e) = rest.finish_command(&cmd.id, status, msg) {
                eprintln!("[command] finish_command failed: {e}");
            }
        }
        "apply-config" => {
            // Issue #31: ép apply lại config.
            acc.applied_version = 0;
            let result = try_apply_config(acc, jar_ctl_version, rest);
            let (status, msg) = evaluate_apply_config_status(result);
            if let Err(e) = rest.finish_command(&cmd.id, status, msg) {
                eprintln!("[command] finish_command failed: {e}");
            }
        }
        "detect-spots" => {
            // 1. JVM/process phải đang chạy
            let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
            if !is_alive {
                eprintln!("[command] detect-spots: account {account_id} JVM is not running");
                if let Err(e) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some("account process is not running"),
                ) {
                    eprintln!("[command] finish_command failed: {e}");
                }
                return;
            }

            // 2. Reject nếu đã có scan đang pending
            if let Err(msg) = crate::spot_scan::can_accept_spot_scan(&acc.pending_spot_scan) {
                eprintln!("[command] detect-spots: account {account_id} already has pending scan");
                if let Err(e) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some(msg),
                ) {
                    eprintln!("[command] finish_command failed: {e}");
                }
                return;
            }

            // 3. Mark command running trên database
            let _ = rest.mark_command_running(&cmd.id);

            // 4. Dọn stale request/payload/ready files của riêng account này
            let paths = AccountPaths::for_slot(acc.slot_index);
            crate::spot_scan::clean_spot_files(&paths.home);

            // 5. Ghi zeus-spot.req
            let scan_id = cmd.id.clone();
            let now_rfc = crate::supabase_rest::now_rfc3339();
            if let Err(e) = crate::spot_scan::write_spot_request_file(&paths.home, &scan_id, Some(&now_rfc)) {
                eprintln!("[command] detect-spots: write_spot_request_file failed: {e}");
                if let Err(fe) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some("failed to write spot request file"),
                ) {
                    eprintln!("[command] finish_command failed: {fe}");
                }
                return;
            }

            // 6. Lưu PendingSpotScan vào AccountState
            acc.pending_spot_scan = Some(crate::spot_scan::PendingSpotScan {
                scan_id: scan_id.clone(),
                command_id: cmd.id.clone(),
                started_at: Instant::now(),
            });

            // 7. Cập nhật last_spot_scan sang pending để publish telemetry
            acc.last_spot_scan = Some(crate::spot_scan::RetainedSpotScan {
                scan_id,
                status: crate::spot_scan::SpotScanStatus::Pending,
                detected_at: Some(now_rfc),
                completed_at: None,
                map_id: None,
                captured_zone: None,
                candidates: None,
            });

            // 8. Publish telemetry snapshot ngay lập tức
            push_account_runtime_telemetry(acc, rest);
        }
        "enhance-item" | "enhance-single-item" | "enhance-item-dry-run" => {
            let is_dry_run_cmd = is_enhancement_dry_run_command(&cmd.kind);
            // 1. JVM/process phải đang chạy
            let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
            if !is_alive {
                eprintln!("[command] enhance-item: account {account_id} JVM is not running");
                if let Err(e) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some("account process is not running"),
                ) {
                    eprintln!("[command] finish_command failed: {e}");
                }
                return;
            }

            // 2. Reject nếu đã có enhancement đang pending (single-item hoặc queue attempt in flight)
            if acc.pending_enhancement.is_some() || acc.queue_tracker.in_flight_attempt.is_some() {
                eprintln!("[command] enhance-item: account {account_id} already has pending enhancement");
                if let Err(e) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some("account already has an enhancement attempt in flight"),
                ) {
                    eprintln!("[command] finish_command failed: {e}");
                }
                return;
            }

            // 3. Parse and validate command payload strictly
            let mut payload: crate::enhancement::SingleItemEnhanceCommandPayload = match cmd.payload {
                Some(ref p) => match serde_json::from_value(p.clone()) {
                    Ok(parsed) => parsed,
                    Err(e) => {
                        eprintln!("[command] enhance-item: payload parse error: {e}");
                        let _ = rest.finish_command(&cmd.id, CommandStatus::Failed, Some("malformed enhancement payload"));
                        return;
                    }
                },
                None => {
                    eprintln!("[command] enhance-item: missing payload");
                    let _ = rest.finish_command(&cmd.id, CommandStatus::Failed, Some("missing enhancement payload"));
                    return;
                }
            };

            if is_dry_run_cmd {
                payload.validation_only = true;
            }

            if let Err(e) = crate::enhancement::validate_single_item_payload(&payload) {
                eprintln!("[command] enhance-item: invalid payload: {e}");
                let err_str = e.to_string();
                let _ = rest.finish_command(&cmd.id, CommandStatus::Failed, Some(&err_str));
                return;
            }

            // 4. Mark command running trên database
            let _ = rest.mark_command_running(&cmd.id);

            // 5. Clean stale enhancement files for this account
            let paths = AccountPaths::for_slot(acc.slot_index);
            crate::enhancement::clean_enhancement_files(&paths.home);

            // 6. Write zeus-enhance.req
            let now_rfc = crate::supabase_rest::now_rfc3339();
            let req_payload = crate::enhancement::EnhancementRequestPayload {
                request_id: cmd.id.clone(),
                captured_slot: payload.captured_slot,
                template_id: payload.template_id,
                category: payload.category,
                base_name: payload.base_name,
                tier: payload.tier,
                expected_level: payload.expected_level,
                target_level: payload.target_level,
                charm_mode: payload.charm_mode,
                payment_type: payload.payment_type,
                max_attempts: payload.max_attempts,
                validation_only: payload.validation_only,
                requested_at: Some(now_rfc),
            };

            if let Err(e) = crate::enhancement::write_enhancement_request_file(&paths.home, &req_payload) {
                eprintln!("[command] enhance-item: write_enhancement_request_file failed: {e}");
                if let Err(fe) = rest.finish_command(
                    &cmd.id,
                    CommandStatus::Failed,
                    Some("failed to write enhancement request file"),
                ) {
                    eprintln!("[command] finish_command failed: {fe}");
                }
                return;
            }

            // 7. Track PendingEnhancement in AccountState
            acc.pending_enhancement = Some(crate::enhancement::PendingEnhancement {
                command_id: cmd.id.clone(),
                request_id: cmd.id.clone(),
                started_at: Instant::now(),
                template_id: payload.template_id,
                category: payload.category,
                target_level: payload.target_level,
                validation_only: payload.validation_only,
            });

            // 8. Publish telemetry snapshot ngay lập tức
            push_account_runtime_telemetry(acc, rest);
        }
        other => {
            eprintln!("[command] unknown command type: {other}");
            if let Err(e) = rest.finish_command(
                &cmd.id,
                CommandStatus::Failed,
                Some(&format!("unknown command type: {other}")),
            ) {
                eprintln!("[command] finish_command failed: {e}");
            }
        }
    }
}

// ── detect spots sidecar polling & immediate telemetry ────────────────────────

#[cfg(unix)]
fn push_account_runtime_telemetry(acc: &mut AccountState, rest: &SupabaseRest) {
    let paths = AccountPaths::for_slot(acc.slot_index);
    let now = crate::supabase_rest::now_rfc3339();
    let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
    let process_state = if is_alive { "running" } else { "stopped" };

    let mut snap_json = read_snapshot_as_json(&paths.snapshot_file())
        .or_else(|| acc.last_snapshot.clone())
        .unwrap_or_else(|| serde_json::json!({}));

    let current_map = snap_json.get("map").and_then(|v| v.as_u64()).map(|v| v as u8);
    crate::spot_scan::merge_spot_scan_into_snapshot(
        &mut snap_json,
        &acc.last_spot_scan,
        current_map,
        Instant::now(),
    );
    crate::inventory::merge_inventory_into_snapshot(
        &mut snap_json,
        &paths.inventory_file(),
    );
    crate::enhancement::merge_enhancement_into_snapshot(
        &mut snap_json,
        &paths.enhancement_status_file(),
    );

    let pid = acc.process.as_ref().map(|p| p.pid);
    acc.last_snapshot = Some(snap_json.clone());
    acc.last_telemetry_push = Instant::now();

    if let Err(e) = rest.push_runtime(
        &acc.id,
        &RuntimePayload {
            process_state: process_state.into(),
            pid,
            ram_mb: None,
            cpu_pct: None,
            snapshot_version: Some(SUPPORTED_VERSION as u32),
            snapshot: Some(snap_json),
            restarts: Some(acc.restarts),
            updated_at: now,
        },
    ) {
        eprintln!("[telemetry] account={} immediate push_runtime failed: {e}", acc.id);
    }
}

#[cfg(unix)]
fn poll_pending_spot_scans(
    accounts: &mut HashMap<String, AccountState>,
    rest: &SupabaseRest,
) {
    let now = Instant::now();
    for acc in accounts.values_mut() {
        let pending = match acc.pending_spot_scan.take() {
            Some(p) => p,
            None => continue,
        };

        let paths = AccountPaths::for_slot(acc.slot_index);

        if crate::spot_scan::is_pending_scan_timed_out(&pending, now) {
            eprintln!(
                "[spot_scan] account {} scan {} timed out after {}s",
                acc.id, pending.scan_id, crate::spot_scan::SPOT_SCAN_TIMEOUT_SECS
            );
            crate::spot_scan::clean_spot_files(&paths.home);
            acc.last_spot_scan = Some(crate::spot_scan::RetainedSpotScan {
                scan_id: pending.scan_id.clone(),
                status: crate::spot_scan::SpotScanStatus::Timeout,
                detected_at: None,
                completed_at: Some(now),
                map_id: None,
                captured_zone: None,
                candidates: None,
            });
            if let Err(e) = rest.finish_command(
                &pending.command_id,
                CommandStatus::Failed,
                Some("detect-spots scan timed out waiting for JVM result"),
            ) {
                eprintln!("[spot_scan] finish_command (timeout) failed: {e}");
            }
            push_account_runtime_telemetry(acc, rest);
            continue;
        }

        match crate::spot_scan::poll_spot_result(&paths.home, &pending.scan_id) {
            crate::spot_scan::SpotPollOutcome::NoReadyMarker => {
                acc.pending_spot_scan = Some(pending);
            }
            crate::spot_scan::SpotPollOutcome::Success(result) => {
                eprintln!(
                    "[spot_scan] account {} scan {} succeeded: map={} zone={} candidates={}",
                    acc.id, result.scan_id, result.map_id, result.captured_zone, result.candidates.len()
                );
                crate::spot_scan::clean_spot_files(&paths.home);

                let is_empty = result.candidates.is_empty();
                let status = if is_empty {
                    crate::spot_scan::SpotScanStatus::Empty
                } else {
                    crate::spot_scan::SpotScanStatus::Completed
                };

                acc.last_spot_scan = Some(crate::spot_scan::RetainedSpotScan {
                    scan_id: result.scan_id.clone(),
                    status,
                    detected_at: Some(crate::supabase_rest::now_rfc3339()),
                    completed_at: Some(now),
                    map_id: Some(result.map_id),
                    captured_zone: Some(result.captured_zone),
                    candidates: Some(result.candidates),
                });

                if let Err(e) = rest.finish_command(
                    &pending.command_id,
                    CommandStatus::Success,
                    None,
                ) {
                    eprintln!("[spot_scan] finish_command (success) failed: {e}");
                }
                push_account_runtime_telemetry(acc, rest);
            }
            crate::spot_scan::SpotPollOutcome::InvalidPayload(err_msg) => {
                eprintln!(
                    "[spot_scan] account {} scan {} invalid payload: {err_msg}",
                    acc.id, pending.scan_id
                );
                crate::spot_scan::clean_spot_files(&paths.home);

                acc.last_spot_scan = Some(crate::spot_scan::RetainedSpotScan {
                    scan_id: pending.scan_id.clone(),
                    status: crate::spot_scan::SpotScanStatus::Error,
                    detected_at: None,
                    completed_at: Some(now),
                    map_id: None,
                    captured_zone: None,
                    candidates: None,
                });

                if let Err(e) = rest.finish_command(
                    &pending.command_id,
                    CommandStatus::Failed,
                    Some(&format!("invalid spot scan result: {err_msg}")),
                ) {
                    eprintln!("[spot_scan] finish_command (error) failed: {e}");
                }
                push_account_runtime_telemetry(acc, rest);
            }
        }
    }
}

#[cfg(unix)]
fn poll_pending_enhancements(
    accounts: &mut HashMap<String, AccountState>,
    rest: &SupabaseRest,
) {
    let now = Instant::now();
    for acc in accounts.values_mut() {
        let pending = match acc.pending_enhancement.take() {
            Some(p) => p,
            None => continue,
        };

        let paths = AccountPaths::for_slot(acc.slot_index);

        let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
        if !is_alive {
            eprintln!(
                "[enhancement] account {} process died while command {} in flight",
                acc.id, pending.command_id
            );
            crate::enhancement::clean_enhancement_files(&paths.home);
            if let Err(e) = rest.finish_command(
                &pending.command_id,
                CommandStatus::Failed,
                Some("account process died during enhancement"),
            ) {
                eprintln!("[enhancement] finish_command (process died) failed: {e}");
            }
            push_account_runtime_telemetry(acc, rest);
            continue;
        }

        if crate::enhancement::is_pending_enhancement_timed_out(&pending, now) {
            eprintln!(
                "[enhancement] account {} command {} timed out after {}s",
                acc.id, pending.command_id, crate::enhancement::ENHANCEMENT_TIMEOUT_SECS
            );
            crate::enhancement::clean_enhancement_files(&paths.home);
            if let Err(e) = rest.finish_command(
                &pending.command_id,
                CommandStatus::Failed,
                Some("enhancement attempt timed out waiting for JVM completion"),
            ) {
                eprintln!("[enhancement] finish_command (timeout) failed: {e}");
            }
            push_account_runtime_telemetry(acc, rest);
            continue;
        }

        match crate::enhancement::poll_enhancement_status(&paths.home, &pending.request_id) {
            crate::enhancement::EnhancementPollOutcome::NoStatusYet => {
                acc.pending_enhancement = Some(pending);
            }
            crate::enhancement::EnhancementPollOutcome::TerminalSuccess(status) => {
                eprintln!(
                    "[enhancement] account {} command {} finished with state: {}",
                    acc.id, pending.command_id, status.state
                );
                crate::enhancement::clean_enhancement_request_file(&paths.home);
                if let Err(e) = rest.finish_command(
                    &pending.command_id,
                    CommandStatus::Success,
                    None,
                ) {
                    eprintln!("[enhancement] finish_command (success) failed: {e}");
                }
                push_account_runtime_telemetry(acc, rest);
            }
            crate::enhancement::EnhancementPollOutcome::TerminalFailure { state, error_message } => {
                eprintln!(
                    "[enhancement] account {} command {} failed: state={} err={:?}",
                    acc.id, pending.command_id, state, error_message
                );
                crate::enhancement::clean_enhancement_request_file(&paths.home);
                let err_desc = error_message
                    .unwrap_or_else(|| format!("enhancement ended in failure state: {state}"));
                if let Err(e) = rest.finish_command(
                    &pending.command_id,
                    CommandStatus::Failed,
                    Some(&err_desc),
                ) {
                    eprintln!("[enhancement] finish_command (failed) failed: {e}");
                }
                push_account_runtime_telemetry(acc, rest);
            }
            crate::enhancement::EnhancementPollOutcome::InvalidPayload(err_msg) => {
                eprintln!(
                    "[enhancement] account {} command {} invalid payload: {err_msg}",
                    acc.id, pending.command_id
                );
                crate::enhancement::clean_enhancement_files(&paths.home);
                if let Err(e) = rest.finish_command(
                    &pending.command_id,
                    CommandStatus::Failed,
                    Some(&format!("invalid enhancement status file: {err_msg}")),
                ) {
                    eprintln!("[enhancement] finish_command (error) failed: {e}");
                }
                push_account_runtime_telemetry(acc, rest);
            }
        }
    }
}

// ── telemetry tick (B6) ───────────────────────────────────────────────────────

#[cfg(unix)]
fn tick_snapshot_telemetry(acc: &mut AccountState, rest: &SupabaseRest) {
    let paths = AccountPaths::for_slot(acc.slot_index);
    let now = crate::supabase_rest::now_rfc3339();

    // Báo cáo process_state độc lập với snapshot — Issue #39
    let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
    let process_state = if is_alive { "running" } else { "stopped" };

    if process_state == "stopped" {
        // Chỉ push nếu chưa push stopped.
        if acc.last_snapshot.is_some() {
            acc.last_snapshot = None;
            crate::inventory::clear_inventory(&paths.home);
            crate::enhancement::clean_enhancement_files(&paths.home);
            if let Err(e) = rest.push_runtime(
                &acc.id,
                &RuntimePayload {
                    process_state: "stopped".into(),
                    pid: None,
                    ram_mb: None,
                    cpu_pct: None,
                    snapshot_version: None,
                    snapshot: None,
                    restarts: Some(acc.restarts),
                    updated_at: now,
                },
            ) {
                eprintln!("[telemetry] account={} push_runtime (stopped) failed: {e}", acc.id);
            }
        }
        return;
    }

    // Single-pass I/O: đọc file một lần, vừa validate vừa parse — Issue #106
    let mut snap_json = match read_snapshot_as_json(&paths.snapshot_file()) {
        Some(v) => v,
        None => return, // snapshot chưa được publish (trước khi char vào game)
    };

    let current_map = snap_json.get("map").and_then(|v| v.as_u64()).map(|v| v as u8);
    crate::spot_scan::merge_spot_scan_into_snapshot(
        &mut snap_json,
        &acc.last_spot_scan,
        current_map,
        Instant::now(),
    );
    crate::inventory::merge_inventory_into_snapshot(
        &mut snap_json,
        &paths.inventory_file(),
    );
    crate::enhancement::merge_enhancement_into_snapshot(
        &mut snap_json,
        &paths.enhancement_status_file(),
    );

    // Prune retained spot_scan if it has expired or map has changed
    if let Some(ref scan) = acc.last_spot_scan {
        if !crate::spot_scan::is_valid_retained_scan(scan, current_map, Instant::now()) {
            acc.last_spot_scan = None;
        }
    }

    // B6.2: chỉ push khi thay đổi có nghĩa.
    if !has_meaningful_change(&acc.last_snapshot, &snap_json) {
        return;
    }

    let pid = acc.process.as_ref().map(|p| p.pid);
    acc.last_snapshot = Some(snap_json.clone());
    acc.last_telemetry_push = Instant::now();

    if let Err(e) = rest.push_runtime(
        &acc.id,
        &RuntimePayload {
            process_state: process_state.into(),
            pid,
            ram_mb: None,
            cpu_pct: None,
            snapshot_version: Some(SUPPORTED_VERSION as u32),
            snapshot: Some(snap_json),
            restarts: Some(acc.restarts),
            updated_at: now,
        },
    ) {
        eprintln!("[telemetry] account={} push_runtime failed: {e}", acc.id);
    }
}

/// Thay đổi có nghĩa — Issues #55: bổ sung state, map, zone, quota, dungeonstate, enhancedone, spot_scan, enhancement
#[cfg(unix)]
fn has_meaningful_change(old: &Option<serde_json::Value>, new: &serde_json::Value) -> bool {
    let old = match old {
        None => return true, // lần đầu
        Some(v) => v,
    };
    // So sánh các field quan trọng — Issue #55
    for key in ["ctl", "atkstate", "stuck", "lv", "state", "map", "zone", "quota", "dungeonstate", "enhancedone", "spot_scan", "inventory", "enhancement"] {
        if old.get(key) != new.get(key) {
            return true;
        }
    }
    // HP/MP vượt ngưỡng 10%.
    for (hp_key, max_key) in [("hp", "hpmax"), ("mp", "mpmax")] {
        let old_v = old.get(hp_key).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let new_v = new.get(hp_key).and_then(|v| v.as_f64()).unwrap_or(0.0);
        let max_v = new.get(max_key).and_then(|v| v.as_f64()).unwrap_or(1.0);
        if max_v > 0.0 && (new_v - old_v).abs() / max_v > 0.1 {
            return true;
        }
    }
    false
}

/// Đọc file snapshot raw (key=value) và chuyển thành JSON object cho telemetry.
/// Single-pass I/O — Issue #106.
/// Parse boolean đúng — Issue #94.
/// Bảo vệ trường string khỏi numeric parsing — Issue #56.
#[cfg(unix)]
fn read_snapshot_as_json(path: &std::path::Path) -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(path).ok()?;

    // Các trường phải giữ nguyên kiểu String — Issue #56
    const STRING_FIELDS: &[&str] = &["name", "guild", "buffs", "drops", "mounts"];

    let mut map = serde_json::Map::new();
    for line in text.lines() {
        if line.is_empty() { continue; }
        if let Some((key, value)) = line.split_once('=') {
            // Parse boolean trước — Issue #94
            let json_val = if value == "true" {
                serde_json::Value::Bool(true)
            } else if value == "false" {
                serde_json::Value::Bool(false)
            } else if STRING_FIELDS.contains(&key) {
                // Giữ nguyên string — Issue #56
                serde_json::Value::String(value.to_string())
            } else if let Ok(n) = value.parse::<i64>() {
                serde_json::Value::Number(n.into())
            } else if let Ok(f) = value.parse::<f64>() {
                serde_json::Number::from_f64(f)
                    .map(serde_json::Value::Number)
                    .unwrap_or_else(|| serde_json::Value::String(value.to_string()))
            } else {
                serde_json::Value::String(value.to_string())
            };
            map.insert(key.to_string(), json_val);
        }
    }
    if map.is_empty() { return None; }
    Some(serde_json::Value::Object(map))
}

// ── heartbeat tick (B6.3) ─────────────────────────────────────────────────────

#[cfg(unix)]
fn tick_heartbeat(
    rest: &SupabaseRest,
    device_id: &str,
    accounts: &mut HashMap<String, AccountState>,
    boot_time: Instant,
) {
    // Uptime từ boot time của agent — Issue #33
    let uptime_s = boot_time.elapsed().as_secs();

    if let Err(e) = rest.heartbeat(
        device_id,
        &DeviceHeartbeat {
            status: "online".into(),
            cpu_pct: read_container_cpu_pct(),
            ram_used_mb: read_container_ram_mb(),
            ram_total_mb: read_container_ram_total_mb(),
            uptime_s,
            last_seen: crate::supabase_rest::now_rfc3339(),
        },
    ) {
        eprintln!("[heartbeat] push failed: {e}");
    }

    // Cũng push snapshot cho tất cả account với xp/gold — Issue #54: re-read từ đĩa
    let now_str = crate::supabase_rest::now_rfc3339();
    for acc in accounts.values_mut() {
        let paths = AccountPaths::for_slot(acc.slot_index);
        // Force re-read snapshot từ đĩa để đồng bộ XP/gold — Issue #54
        if let Some(mut snap_json) = read_snapshot_as_json(&paths.snapshot_file()) {
            let current_map = snap_json.get("map").and_then(|v| v.as_u64()).map(|v| v as u8);
            crate::spot_scan::merge_spot_scan_into_snapshot(
                &mut snap_json,
                &acc.last_spot_scan,
                current_map,
                Instant::now(),
            );
            crate::inventory::merge_inventory_into_snapshot(
                &mut snap_json,
                &paths.inventory_file(),
            );
            acc.last_snapshot = Some(snap_json.clone());
            let pid = acc.process.as_ref().map(|p| p.pid);
            let is_alive = acc.process.as_ref().map(|p| p.alive()).unwrap_or(false);
            if let Err(e) = rest.push_runtime(
                &acc.id,
                &RuntimePayload {
                    process_state: if is_alive { "running".into() } else { "stopped".into() },
                    pid,
                    ram_mb: None,
                    cpu_pct: None,
                    snapshot_version: snap_json.get("v").and_then(|v| v.as_u64()).map(|v| v as u32),
                    snapshot: Some(snap_json),
                    restarts: Some(acc.restarts),
                    updated_at: now_str.clone(),
                },
            ) {
                eprintln!("[heartbeat] account={} push_runtime failed: {e}", acc.id);
            }
        }
    }
}

// ── realtime thread ───────────────────────────────────────────────────────────

/// Spawn thread blocking chạy realtime client. Khi socket chết → gửi Disconnected.
#[cfg(unix)]
fn spawn_realtime_thread(
    base_url: String,
    anon_key: String,
    access_token: String,
    tx: mpsc::Sender<CloudEvent>,
) {
    std::thread::spawn(move || {
        let mut client = loop {
            match RealtimeClient::connect(&base_url, &anon_key) {
                Ok(c) => break c,
                Err(e) => {
                    eprintln!("[realtime] connect failed: {e}, retrying in 5s");
                    std::thread::sleep(Duration::from_secs(5));
                }
            }
        };

        // Subscribe các bảng cần thiết — Issue #88: lỗi subscribe = fatal, reconnect
        // Issue #42: KHÔNG subscribe account_runtime (agent là producer duy nhất)
        for table in ["accounts", "commands"] {
            if let Err(e) = client.subscribe(table, Some(&access_token)) {
                eprintln!("[realtime] subscribe {table} failed: {e}, reconnecting");
                let _ = tx.send(CloudEvent::Disconnected { reason: format!("subscribe {table} failed: {e}") });
                return;
            }
        }

        eprintln!("[realtime] connected and subscribed");
        let _ = tx.send(CloudEvent::RealtimeReady);

        let mut last_heartbeat = Instant::now();

        loop {
            // Gửi Phoenix heartbeat mỗi 25s — Issue #08
            if last_heartbeat.elapsed() >= Duration::from_secs(25) {
                last_heartbeat = Instant::now();
                if let Err(e) = client.heartbeat() {
                    eprintln!("[realtime] heartbeat failed: {e}");
                    let _ = tx.send(CloudEvent::Disconnected { reason: e.to_string() });
                    break;
                }
            }

            match client.read_event() {
                Ok(Some(RealtimeEvent::Change { table, change_type, record, old_record })) => {
                    let event = match table.as_str() {
                        "accounts" => {
                            match change_type {
                                ChangeType::Insert => {
                                    CloudEvent::AccountAdded { record }
                                }
                                ChangeType::Update => {
                                    let account_id = record["id"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string();
                                    CloudEvent::AccountChanged { account_id, record }
                                }
                                ChangeType::Delete => {
                                    // old_record là Value (không phải Option); fallback qua record nếu REPLICA IDENTITY FULL
                                    let account_id = old_record["id"].as_str()
                                        .or_else(|| record["id"].as_str())
                                        .unwrap_or("")
                                        .to_string();
                                    if account_id.is_empty() { continue; }
                                    CloudEvent::AccountDeleted { account_id }
                                }
                            }
                        }
                        "commands" if change_type == ChangeType::Insert => {
                            // Chỉ quan tâm INSERT queued.
                            if record["status"].as_str() == Some("queued") {
                                let cmd = CommandRow {
                                    id: record["id"].as_str().unwrap_or("").to_string(),
                                    account_id: record["account_id"].as_str().map(str::to_owned),
                                    device_id: record["device_id"].as_str().map(str::to_owned),
                                    kind: record["type"].as_str().unwrap_or("").to_string(),
                                    payload: record.get("payload").cloned(),
                                    expires_at: record["expires_at"].as_str().unwrap_or("").to_string(),
                                };
                                CloudEvent::CommandQueued { command: cmd }
                            } else {
                                continue;
                            }
                        }
                        _ => continue,
                    };
                    if tx.send(event).is_err() {
                        break; // vòng chính đã drop channel
                    }
                }
                Ok(Some(RealtimeEvent::Disconnected { reason })) => {
                    let _ = tx.send(CloudEvent::Disconnected { reason });
                    break;
                }
                Ok(None) => {} // noise frame, tiếp tục
                Err(e) => {
                    eprintln!("[realtime] read error: {e}");
                    let _ = tx.send(CloudEvent::Disconnected { reason: e.to_string() });
                    break;
                }
            }
        }
    });
}

// ── safe retirement & reconciliation ─────────────────────────────────────────

/// Dừng an toàn một account và chỉ giải phóng state khi process group đã chứng minh dừng.
///
/// Invariant C: Không bao giờ quên một process có thể đang sống.
/// Invariant D: Tái sử dụng process_unix::stop và evaluate_stop_transition.
#[cfg(unix)]
fn retire_account_safely(
    accounts: &mut HashMap<String, AccountState>,
    account_id: &str,
    retired_tombstones: &mut std::collections::HashSet<String>,
) {
    // Tombstone account_id ngay lập tức để không bao giờ bị resurrect/admission lại
    retired_tombstones.insert(account_id.to_string());

    let Some(acc) = accounts.get_mut(account_id) else {
        return;
    };

    // Đánh dấu retiring để không bao giờ autostart/restart/nhận lệnh nữa
    acc.retiring = true;
    acc.desired_state = "stopped".to_string();

    let (has_process, is_alive) = acc
        .process
        .as_ref()
        .map(|p| (true, p.pgid_alive()))
        .unwrap_or((false, false));

    let outcome = if is_alive {
        if let Some(ref child) = acc.process {
            eprintln!("[retire] account={} slot={}: stopping process group", acc.id, acc.slot_index);
            let res = process_unix::stop(child, Duration::from_secs(5));
            Some(res)
        } else {
            None
        }
    } else {
        None
    };

    match evaluate_stop_transition(has_process, is_alive, outcome) {
        StopTransition::ConfirmedStopped => {
            eprintln!("[retire] account={}: process proven stopped, cleaning local state", account_id);
            let slot_index = acc.slot_index;
            let paths = AccountPaths::for_slot(slot_index);
            if let Err(e) = clear_credentials(&paths.home) {
                eprintln!("[retire] account={}: clear_credentials failed: {e}", account_id);
            }
            if let Err(e) = clear_snapshot(&paths.home) {
                eprintln!("[retire] account={}: clear_snapshot failed: {e}", account_id);
            }
            crate::spot_scan::clean_spot_files(&paths.home);
            crate::enhancement::clean_enhancement_files(&paths.home);
            acc.process = None;
            accounts.remove(account_id);
        }
        StopTransition::FailedStillAlive => {
            eprintln!(
                "[retire] account={}: stop failed or process group still alive; retaining handle for retry",
                account_id
            );
            // Invariant C & Rust ownership guard: acc.process được giữ nguyên, account giữ trong accounts
        }
    }
}

/// Reconcile authoritative cloud accounts với local runtime accounts.
///
/// Invariant B: Snapshot rỗng thành công LÀ authoritative (tất cả local account bị xóa trên cloud).
/// Problem A: Account đang retiring là monotonic, không bao giờ bị merge/resurrect hay autostart lại.
#[cfg(unix)]
fn reconcile_cloud_accounts(
    accounts: &mut HashMap<String, AccountState>,
    fresh_rows: Vec<crate::supabase_rest::AccountRow>,
    rest: &SupabaseRest,
    identity: &crate::crypto::DeviceIdentity,
    jar_ctl_version: i32,
    retired_tombstones: &mut std::collections::HashSet<String>,
) {
    let local_entries: Vec<(&str, bool)> = accounts
        .iter()
        .map(|(id, acc)| (id.as_str(), acc.retiring))
        .collect();

    let plan = crate::supabase_rest::evaluate_account_reconciliation(
        local_entries,
        Ok::<&[crate::supabase_rest::AccountRow], std::convert::Infallible>(&fresh_rows),
        retired_tombstones.iter().map(|s| s.as_str()),
    );

    match plan {
        crate::supabase_rest::ReconciliationPlan::AbortedFetchFailed => {
            eprintln!("[reconcile] cloud fetch aborted; leaving local accounts untouched");
        }
        crate::supabase_rest::ReconciliationPlan::Apply {
            to_insert: _,
            to_update: _,
            to_retire,
        } => {
            // 1. Retire an toàn các account local không còn trên cloud HOẶC đang pending retirement
            for id in to_retire {
                eprintln!("[reconcile] account {id} retiring or missing from cloud set, ensuring safe retirement");
                retire_account_safely(accounts, &id, retired_tombstones);
            }

            // 2. Merge các account mới / cập nhật metadata các account hiện có (chỉ non-retiring & non-tombstoned)
            let fresh_map = account_states_from_rows(fresh_rows);
            merge_account_states(accounts, fresh_map, rest, identity, jar_ctl_version, retired_tombstones);
        }
    }
}

// ── boot helpers ──────────────────────────────────────────────────────────────

#[cfg(unix)]
fn boot_fetch_accounts(rest: &SupabaseRest, device_id: &str) -> Result<HashMap<String, AccountState>, RestError> {
    match rest.fetch_accounts(device_id) {
        Ok(rows) => Ok(account_states_from_rows(rows)),
        Err(e) => {
            eprintln!("[boot] fetch_accounts failed: {e}");
            Err(e)
        }
    }
}

#[cfg(unix)]
fn account_states_from_rows(rows: Vec<crate::supabase_rest::AccountRow>) -> HashMap<String, AccountState> {
    rows.into_iter()
        .map(|row| {
            let id = row.id.clone();
            let server_index = (row.server_index as i32).clamp(0, 7) as u8;
            let state = AccountState {
                id: row.id,
                slot_index: row.slot_index,
                desired_state: row.desired_state,
                control_version: row.control_version,
                control: row.control,
                config_version: row.config_version,
                applied_version: 0,
                username: row.username,
                server_index,
                character_slot: row.character_slot,
                secret_sealed: row.secret_sealed,
                runtime_config: row.runtime,
                process: None,
                last_snapshot: None,
                last_telemetry_push: Instant::now(),
                restarts: 0,
                retiring: false,
                pending_spot_scan: None,
                last_spot_scan: None,
                pending_enhancement: None,
                queue_tracker: crate::enhancement_queue::AccountQueueTracker::default(),
            };
            (id, state)
        })
        .collect()
}

/// Tạo AccountState từ một realtime record JSON.
#[cfg(unix)]
fn make_account_state_from_record(record: &serde_json::Value) -> Option<AccountState> {
    let id = record["id"].as_str()?.to_string();
    let slot_index = record["slot_index"].as_i64()? as i32;
    let server_index = (record["server_index"].as_i64().unwrap_or(0) as i32).clamp(0, 7) as u8;
    let character_slot = record
        .get("character_slot")
        .and_then(|v| v.as_i64())
        .map(|n| n as i16)
        .unwrap_or(1);
    Some(AccountState {
        id,
        slot_index,
        desired_state: record["desired_state"].as_str().unwrap_or("stopped").to_string(),
        control_version: record["control_version"].as_i64().unwrap_or(0) as i32,
        control: record.get("control").cloned().unwrap_or(serde_json::Value::Object(Default::default())),
        config_version: record["config_version"].as_i64().unwrap_or(0) as i32,
        applied_version: 0,
        username: record["username"].as_str().unwrap_or("").to_string(),
        server_index,
        character_slot,
        secret_sealed: record.get("secret_sealed").cloned().unwrap_or(serde_json::json!({})),
        runtime_config: record.get("runtime").cloned().unwrap_or(serde_json::json!({})),
        process: None,
        last_snapshot: None,
        last_telemetry_push: Instant::now(),
        restarts: 0,
        retiring: false,
        pending_spot_scan: None,
        last_spot_scan: None,
        pending_enhancement: None,
        queue_tracker: crate::enhancement_queue::AccountQueueTracker::default(),
    })
}

#[cfg(unix)]
fn merge_account_states(
    accounts: &mut HashMap<String, AccountState>,
    fresh: HashMap<String, AccountState>,
    rest: &SupabaseRest,
    identity: &crate::crypto::DeviceIdentity,
    jar_ctl_version: i32,
    retired_tombstones: &std::collections::HashSet<String>,
) {
    for (id, fresh_acc) in fresh {
        if crate::supabase_rest::evaluate_account_admission(
            &id,
            retired_tombstones.contains(&id),
        ) == crate::supabase_rest::AccountAdmissionDecision::RejectedTombstoned
        {
            eprintln!("[reconcile] account {id} is tombstoned/retired, skipping snapshot merge/insertion");
            continue;
        }

        if let Some(existing) = accounts.get_mut(&id) {
            if existing.retiring {
                eprintln!("[reconcile] account {id} is retiring, skipping cloud snapshot merge to prevent resurrection");
                continue;
            }
            existing.control_version = fresh_acc.control_version;
            existing.control = fresh_acc.control;
            existing.config_version = fresh_acc.config_version;
            existing.desired_state = fresh_acc.desired_state;
            existing.secret_sealed = fresh_acc.secret_sealed;
            existing.server_index = fresh_acc.server_index;
            existing.character_slot = fresh_acc.character_slot;
            existing.username = fresh_acc.username;
            existing.runtime_config = fresh_acc.runtime_config;
            // Preserves existing.process (supervised Child), existing.last_snapshot, existing.restarts, etc.
            let _ = try_apply_config(existing, jar_ctl_version, rest);
            reconcile_desired_state(existing, rest, identity);
        } else {
            // New account: initialize directories and clear old snapshot
            let paths = AccountPaths::for_slot(fresh_acc.slot_index);
            let _ = crate::launch::prepare_directories(&paths);
            let _ = clear_snapshot(&paths.home);
            crate::spot_scan::clean_spot_files(&paths.home);
            crate::enhancement::clean_enhancement_files(&paths.home);
            let mut acc = fresh_acc;
            let _ = try_apply_config(&mut acc, jar_ctl_version, rest);
            reconcile_desired_state(&mut acc, rest, identity);
            accounts.insert(id, acc);
        }
    }
}

#[cfg(unix)]
fn recover_queued_commands(
    rest: &SupabaseRest,
    device_id: &str,
    accounts: &mut HashMap<String, AccountState>,
    cfg: &AgentConfig,
    identity: &crate::crypto::DeviceIdentity,
    jar_ctl_version: i32,
    dedupe: &mut CommandDedupe,
    retired_tombstones: &mut std::collections::HashSet<String>,
) {
    if let Err(e) = rest.expire_stale_commands(device_id) {
        eprintln!("[recovery] expire_stale_commands failed: {e}");
    }

    let cmds = match rest.drain_commands(device_id) {
        Ok(cmds) => cmds,
        Err(e) => {
            eprintln!("[recovery] drain_commands failed: {e}");
            return;
        }
    };

    // State barrier: fetch authoritative fresh account snapshot AFTER drain to guarantee
    // account state reflects any creation/update/deletion that happened before or with the commands.
    let fresh_rows = match rest.fetch_accounts(device_id) {
        Ok(rows) => rows,
        Err(e) => {
            // Post-drain account refresh failed: do NOT dispatch account commands against stale state.
            // Leave commands queued in database for future recovery; do not record them in dedupe.
            eprintln!(
                "[recovery] post-drain fetch_accounts failed: {e}, aborting recovery to prevent dispatching against stale account state"
            );
            return;
        }
    };

    // Reconcile cloud accounts authoritatively
    reconcile_cloud_accounts(accounts, fresh_rows, rest, identity, jar_ctl_version, retired_tombstones);

    if cmds.is_empty() {
        return;
    }

    eprintln!("[recovery] drained {} queued command(s)", cmds.len());

    for cmd in cmds {
        if !dedupe.record_if_new(&cmd.id) {
            eprintln!("[recovery] duplicate command id={} already processed, suppressing", cmd.id);
            continue;
        }
        dispatch_command(cmd, accounts, rest, cfg, identity, jar_ctl_version, retired_tombstones);
    }
}

// ── PID 1: reap zombies (B1.3) ────────────────────────────────────────────────

#[cfg(unix)]
fn reap_zombies() {
    loop {
        let ret = unsafe { libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) };
        if ret <= 0 {
            break; // 0 = không có zombie, -1 = không có child nào
        }
        eprintln!("[zombies] reaped pid={ret}");
    }
}

// ── VNC viewer count (B8.1) — Issues #46, #53, #99 ───────────────────────────

/// Đọc /proc/net/tcp để đếm kết nối TCP vào VNC port và external port.
/// Không fork process ss — Issue #53.
#[cfg(unix)]
fn count_vnc_clients(display_num: u32, external_port: u16) -> u32 {
    let vnc_port = 5900u32 + display_num;
    // Chuyển sang hex little-endian như trong /proc/net/tcp
    let vnc_hex = format!("{:04X}", vnc_port);
    let ext_hex = format!("{:04X}", external_port);

    let Ok(text) = std::fs::read_to_string("/proc/net/tcp") else {
        return 0;
    };

    let mut count = 0u32;
    for line in text.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 4 { continue; }
        // Format: local_address (IP:PORT hex), remote_address, state (01=ESTABLISHED)
        let local = parts[1];
        let state = parts[3];
        if state != "01" { continue; } // chỉ ESTABLISHED
        // local_address = "XXXXXXXX:PPPP" where PPPP là port hex
        if let Some(port_part) = local.split(':').nth(1) {
            // /proc/net/tcp dùng little-endian hex cho IP nhưng big-endian cho port
            let port_upper = port_part.to_uppercase();
            if port_upper == vnc_hex || port_upper == ext_hex {
                count += 1;
            }
        }
    }
    count
}

#[cfg(unix)]
fn write_potato_ctl_for_path(paths: &AccountPaths, mode: &str) {
    // B8.5: atomic replace. potato.ctl là fail-safe, không fail agent khi ghi lỗi.
    // Tạo temp file trong cùng thư mục để tránh EXDEV — Issue #105
    use std::io::Write;
    let tmp_path = paths.home.join("potato.ctl.tmp");
    let final_path = paths.potato_file();
    if let Ok(mut f) = std::fs::File::create(&tmp_path) {
        let _ = f.write_all(mode.as_bytes());
        let _ = std::fs::rename(&tmp_path, &final_path);
    }
}

// ── jar manifest ──────────────────────────────────────────────────────────────

#[cfg(unix)]
fn read_jar_manifest(path: &str) -> Option<JarManifest> {
    crate::supabase_rest::read_jar_manifest(path)
}

// ── system metrics (B6.4) ─────────────────────────────────────────────────────

#[cfg(unix)]
fn read_container_cpu_pct() -> f32 {
    // TODO: implement cgroup v2 delta CPU — Issue #30, #108
    0.0
}

#[cfg(unix)]
fn read_container_ram_mb() -> u32 {
    // Thử cgroup v2 trước, fallback cgroup v1 — Issue #108
    if let Some(mb) = std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|b| (b / 1_048_576) as u32)
    {
        return mb;
    }
    // Cgroup v1 fallback
    std::fs::read_to_string("/sys/fs/cgroup/memory/memory.usage_in_bytes")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|b| (b / 1_048_576) as u32)
        .unwrap_or(0)
}

#[cfg(unix)]
fn read_container_ram_total_mb() -> u32 {
    // Thử cgroup v2 trước
    let s = std::fs::read_to_string("/sys/fs/cgroup/memory.max").unwrap_or_default();
    let s = s.trim();
    if s != "max" && !s.is_empty() {
        if let Ok(b) = s.parse::<u64>() {
            return (b / 1_048_576) as u32;
        }
    }
    // Cgroup v1 fallback
    if let Ok(s1) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
        let s1 = s1.trim();
        if let Ok(b) = s1.parse::<u64>() {
            // Cgroup v1 báo giá trị rất lớn khi không có limit
            if b < u64::MAX / 2 {
                return (b / 1_048_576) as u32;
            }
        }
    }
    // Fallback /proc/meminfo — Issue #95: tránh chia cho 0
    read_meminfo_total_mb()
}

/// Đọc MemTotal từ /proc/meminfo — Issue #95
#[cfg(unix)]
fn read_meminfo_total_mb() -> u32 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|text| {
            text.lines()
                .find(|l| l.starts_with("MemTotal:"))
                .and_then(|l| {
                    l.split_whitespace().nth(1).and_then(|v| v.parse::<u64>().ok())
                })
        })
        .map(|kb| (kb / 1024) as u32)
        .unwrap_or(1024) // fallback 1GB
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_v13_control_json() -> serde_json::Value {
        serde_json::json!({
            "atk.mode": 0,
            "atk.map": 0,
            "atk.zone": -1,
            "atk.x": -1,
            "atk.y": -1,
            "atk.radius": 100,
            "atk.hpOn": 0,
            "atk.hpPct": 50,
            "atk.mpOn": 0,
            "atk.mpPct": 50,
            "revive.mode": 1,
            "atk.buffs": "000",
            "atk.zoneMode": 0,
            "atk.zonePick": 1,
            "item.rank": 5,
            "item.mphp": 3,
            "item.gold": 1,
            "mount.on": 0,
            "mount.id": 0,
            "item.medalDialog": 0,
            "item.dropsOn": 0,
            "item.drops": "000000",
            "nav.target": -1,
            "ui.ring": 0,
            "atk.farmOnArrival": 0,
            "nav.detectSpots": 0,
            "revive.delay": 0,
            "revive.on": 0,
            "enhance.on": 0,
            "enhance.maxLv": 10,
            "enhance.charm": 0,
            "dungeon.on": 0,
            "dungeon.max": -1,
            "dungeon.schedule": -1
        })
    }

    #[test]
    fn test_control_version_gate_evaluation() {
        // When jar is v14:
        assert!(evaluate_control_version_gate(13, 14));
        assert!(evaluate_control_version_gate(14, 14));
        assert!(!evaluate_control_version_gate(12, 14));
        assert!(!evaluate_control_version_gate(15, 14));

        // When jar is v13:
        assert!(evaluate_control_version_gate(13, 13));
        assert!(!evaluate_control_version_gate(14, 13));
    }

    #[test]
    fn test_v13_account_normalizes_to_v14_wire_with_neutral_defaults() {
        let v13_json = sample_v13_control_json();
        let settings = build_control_settings(&v13_json, 13).expect("v13 control parses successfully");
        assert_eq!(settings.effects, 1);
        assert_eq!(settings.hide_players, 0);

        let wire = settings.to_wire();
        assert!(wire.starts_with("v=14\n"));
        assert!(wire.contains("ui.effects=1\n"));
        assert!(wire.contains("ui.hidePlayers=0\n"));
        let line_count = wire.lines().count();
        assert_eq!(line_count, 37, "canonical v14 wire must have exactly 37 keys");
    }

    #[test]
    fn test_v14_account_preserves_configured_qol_values() {
        let mut v14_json = sample_v13_control_json();
        v14_json["ui.effects"] = serde_json::json!(0);
        v14_json["ui.hidePlayers"] = serde_json::json!(2);

        let settings = build_control_settings(&v14_json, 14).expect("v14 control parses successfully");
        assert_eq!(settings.effects, 0);
        assert_eq!(settings.hide_players, 2);

        let wire = settings.to_wire();
        assert!(wire.starts_with("v=14\n"));
        assert!(wire.contains("ui.effects=0\n"));
        assert!(wire.contains("ui.hidePlayers=2\n"));
        assert_eq!(wire.lines().count(), 37);
    }

    #[test]
    fn test_unsupported_versions_fail_closed() {
        let json = sample_v13_control_json();
        assert!(build_control_settings(&json, 12).is_err());
        assert!(build_control_settings(&json, 15).is_err());
    }

    #[test]
    fn test_unknown_keys_fail_closed() {
        let mut json = sample_v13_control_json();
        json["unknown.field"] = serde_json::json!(1);
        assert!(build_control_settings(&json, 13).is_err());
        assert!(build_control_settings(&json, 14).is_err());
    }

    #[test]
    fn test_invalid_qol_values_fail_closed_in_v14() {
        let mut invalid_effects = sample_v13_control_json();
        invalid_effects["ui.effects"] = serde_json::json!(2);
        invalid_effects["ui.hidePlayers"] = serde_json::json!(0);
        assert!(build_control_settings(&invalid_effects, 14).is_err());

        let mut invalid_hide = sample_v13_control_json();
        invalid_hide["ui.effects"] = serde_json::json!(1);
        invalid_hide["ui.hidePlayers"] = serde_json::json!(3);
        assert!(build_control_settings(&invalid_hide, 14).is_err());
    }

    #[test]
    fn test_empty_account_control_uses_defaults() {
        let empty = serde_json::json!({});
        let settings = build_control_settings(&empty, 13).expect("empty control uses default");
        assert_eq!(settings.effects, 1);
        assert_eq!(settings.hide_players, 0);
        assert_eq!(settings.to_wire().lines().count(), 37);
    }

    #[test]
    fn test_enhancement_command_kind_classification() {
        use crate::supabase_rest::CommandRow;

        assert!(is_enhancement_dry_run_command("enhance-item-dry-run"));
        assert!(!is_enhancement_dry_run_command("enhance-item"));
        assert!(!is_enhancement_dry_run_command("enhance-single-item"));
        assert!(!is_enhancement_dry_run_command("start"));
        assert!(!is_enhancement_dry_run_command("spot-scan"));

        let dry_run_cmd = CommandRow {
            id: "cmd-dry-1".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "enhance-item-dry-run".to_string(),
            payload: None,
            expires_at: "2026-09-26T00:00:00Z".to_string(),
        };
        assert!(is_enhancement_dry_run_command(&dry_run_cmd.kind));
        assert_eq!(dry_run_cmd.kind, "enhance-item-dry-run");

        let normal_cmd = CommandRow {
            id: "cmd-norm-1".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "enhance-item".to_string(),
            payload: None,
            expires_at: "2026-09-26T00:00:00Z".to_string(),
        };
        assert!(!is_enhancement_dry_run_command(&normal_cmd.kind));
        assert_eq!(normal_cmd.kind, "enhance-item");

        let single_cmd = CommandRow {
            id: "cmd-single-1".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "enhance-single-item".to_string(),
            payload: None,
            expires_at: "2026-09-26T00:00:00Z".to_string(),
        };
        assert!(!is_enhancement_dry_run_command(&single_cmd.kind));
        assert_eq!(single_cmd.kind, "enhance-single-item");
    }

    #[test]
    fn test_enhancement_queue_tick_scheduling_cadence() {
        let base = Instant::now();

        // 1. Elapsed below 2 seconds must NOT tick (bounds cadence, prevents busy-loop)
        assert!(!should_tick_enhancement_queue(base, base));
        assert!(!should_tick_enhancement_queue(base + Duration::from_millis(500), base));
        assert!(!should_tick_enhancement_queue(base + Duration::from_millis(1000), base));
        assert!(!should_tick_enhancement_queue(base + Duration::from_millis(1999), base));

        // 2. Elapsed at or above 2 seconds MUST tick (reachability guaranteed)
        assert!(should_tick_enhancement_queue(base + Duration::from_secs(2), base));
        assert!(should_tick_enhancement_queue(base + Duration::from_secs(3), base));
        assert!(should_tick_enhancement_queue(base + Duration::from_secs(60), base));
    }

    #[test]
    fn test_queue_tick_polling_failure_is_nonfatal_for_main_loop() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home = temp_dir.path();
        let mut tracker = crate::enhancement_queue::AccountQueueTracker::default();
        // Point to an unreachable port to guarantee connection/REST failure
        let rest = crate::supabase_rest::SupabaseRest::new("http://127.0.0.1:9".to_string(), "test-key".to_string());

        // Must execute cleanly without panic
        crate::enhancement_queue::tick_account_enhancement_queue(
            home,
            "dev-test",
            "acc-test",
            true,
            false,
            &mut tracker,
            &rest,
        );

        // State remains uncorrupted
        assert!(tracker.active_job_id.is_none());
        assert!(tracker.in_flight_attempt.is_none());
        assert!(!home.join("enhancement_request.json").exists());
    }

    #[test]
    fn test_no_queue_mutation_when_no_queue_rows_exist() {
        let temp_dir = tempfile::tempdir().unwrap();
        let home = temp_dir.path();
        let mut tracker = crate::enhancement_queue::AccountQueueTracker::default();
        let rest = crate::supabase_rest::SupabaseRest::new("http://127.0.0.1:54321".to_string(), "test-key".to_string());

        crate::enhancement_queue::tick_account_enhancement_queue(
            home,
            "dev-test",
            "acc-empty",
            true,
            false,
            &mut tracker,
            &rest,
        );

        assert!(tracker.active_job_id.is_none());
        assert!(tracker.in_flight_attempt.is_none());
        assert!(!home.join("enhancement_request.json").exists());
        assert!(!home.join("zeus-enhance.req").exists());
    }
}
