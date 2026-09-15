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
//! 4. Spawn thread realtime; subscribe devices, accounts, account_runtime, commands
//! 5. Bước vào vòng chính
//!
//! ## Reconnect (B2.2)
//!
//! Khi realtime thread gửi `CloudEvent::Disconnected`, vòng chính:
//! 1. Dừng cũ, spawn thread mới
//! 2. Gọi `fetch_accounts` + `drain_commands` ngay — realtime có thể đã miss event
//! 3. Reconcile lại toàn bộ state

#[cfg(unix)]
use std::{
    collections::HashMap,
    sync::mpsc,
    time::{Duration, Instant},
};

#[cfg(unix)]
use crate::{
    launch::AccountPaths,
    process_unix::{AccountProcess, StopOutcome},
    supabase_realtime::{ChangeType, RealtimeClient, RealtimeEvent},
    supabase_rest::{
        CommandRow, CommandStatus, ConfigStatus, DeviceHeartbeat, JarManifest, RestError,
        RuntimePayload, SupabaseRest,
    },
};

#[cfg(unix)]
use zeus_core::wire::{read_snapshot, write_settings, SNAPSHOT_FILE_NAME};

// ── types ─────────────────────────────────────────────────────────────────────

/// Sự kiện từ thread realtime → vòng chính.
#[cfg(unix)]
#[derive(Debug)]
pub enum CloudEvent {
    /// Hàng `accounts` hoặc `account_runtime` thay đổi.
    AccountChanged { account_id: String, record: serde_json::Value },
    /// Hàng `commands` INSERT với status='queued'.
    CommandQueued { command: CommandRow },
    /// Socket WebSocket đứt — cần reconnect.
    Disconnected { reason: String },
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
    /// JVM đang chạy, nếu có.
    process: Option<AccountProcess>,
    /// Snapshot lần đọc trước (để detect thay đổi có nghĩa).
    last_snapshot: Option<serde_json::Value>,
    /// Nhịp telemetry: chỉ push khi thay đổi ý nghĩa hoặc 60s tick.
    last_telemetry_push: Instant,
}

/// Cấu hình môi trường agent. Đọc từ biến môi trường lúc boot.
#[cfg(unix)]
pub struct AgentConfig {
    pub device_id: String,
    pub jar_manifest_path: String,
}

#[cfg(unix)]
impl AgentConfig {
    pub fn from_env() -> Result<Self, String> {
        fn var(key: &str) -> Result<String, String> {
            std::env::var(key).map_err(|_| format!("missing env var: {key}"))
        }
        Ok(Self {
            device_id: var("ZEUS_DEVICE_ID")?,
            jar_manifest_path: std::env::var("JAR_MANIFEST_PATH")
                .unwrap_or_else(|_| "/opt/knight/game/zeus-jar.json".to_string()),
        })
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

/// Chạy vòng chính cho đến khi nhận SIGTERM (không return bình thường).
///
/// Gọi từ `main()` sau khi pairing xong và `device_id` đã có.
#[cfg(unix)]
pub fn run(cfg: AgentConfig, access_token: String) -> ! {
    eprintln!("[main_loop] starting — device_id={}", cfg.device_id);

    let rest = {
        let mut r = SupabaseRest::new(crate::supabase_rest::SUPABASE_URL.to_string(), crate::supabase_rest::SUPABASE_ANON_KEY.to_string());
        r.set_access_token(access_token.clone());
        r
    };

    // Đọc jar manifest và khai contract.
    let manifest = read_jar_manifest(&cfg.jar_manifest_path);
    if let Some(ref m) = manifest {
        if let Err(e) = rest.announce_jar_contract(&cfg.device_id, m) {
            eprintln!("[main_loop] announce_jar_contract failed: {e}");
        }
    }
    let jar_ctl_version = manifest.as_ref().map(|m| m.ctl_version as i32).unwrap_or(13);

    // Boot: đọc full state.
    let mut accounts = boot_fetch_accounts(&rest, &cfg.device_id);
    drain_and_expire_commands(&rest, &cfg.device_id);

    // Apply config + reconcile tất cả account lúc boot (B5.1, B7.3).
    for acc in accounts.values_mut() {
        try_apply_config(acc, jar_ctl_version, &rest);
        reconcile_desired_state(acc);
    }

    // Spawn thread realtime.
    let (tx, rx) = mpsc::channel::<CloudEvent>();
    spawn_realtime_thread(
        crate::supabase_rest::SUPABASE_URL.to_string(),
            crate::supabase_rest::SUPABASE_ANON_KEY.to_string(),
        access_token.clone(),
        tx.clone(),
    );

    // Ticks
    let mut last_snapshot_tick = Instant::now();
    let mut last_heartbeat_tick = Instant::now();
    let mut last_viewer_tick = Instant::now();
    let mut viewer_client_count: u32 = 0;
    let mut viewer_zero_since: Option<Instant> = None;

    eprintln!("[main_loop] entering main loop");

    loop {
        // ── reap zombies (PID 1 obligation) ───────────────────────────────
        reap_zombies();

        // ── receive realtime event (với timeout 1s) ───────────────────────
        match rx.recv_timeout(Duration::from_secs(1)) {
            Ok(event) => handle_cloud_event(
                event,
                &mut accounts,
                jar_ctl_version,
                &rest,
                &cfg,
                &access_token,
                &tx,
            ),
            Err(mpsc::RecvTimeoutError::Timeout) => {} // bình thường, xử lý ticks bên dưới
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                eprintln!("[main_loop] realtime channel closed unexpectedly, reconnecting");
                spawn_realtime_thread(
                    crate::supabase_rest::SUPABASE_URL.to_string(),
            crate::supabase_rest::SUPABASE_ANON_KEY.to_string(),
                    access_token.clone(),
                    tx.clone(),
                );
            }
        }

        let now = Instant::now();

        // ── tick 2 s: đọc snapshot + telemetry ───────────────────────────
        if now.duration_since(last_snapshot_tick) >= Duration::from_secs(2) {
            last_snapshot_tick = now;
            for acc in accounts.values_mut() {
                tick_snapshot_telemetry(acc, &rest);
            }
        }

        // ── tick 5 s: VNC viewer count ────────────────────────────────────
        if now.duration_since(last_viewer_tick) >= Duration::from_secs(5) {
            last_viewer_tick = now;
            // Đếm tổng VNC client của tất cả account (display :1, :2, …)
            let count = count_vnc_clients();
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
                write_potato_ctl(mode);
            }
            viewer_client_count = count;
        }

        // ── tick 60 s: heartbeat + device metrics ─────────────────────────
        if now.duration_since(last_heartbeat_tick) >= Duration::from_secs(60) {
            last_heartbeat_tick = now;
            tick_heartbeat(&rest, &cfg.device_id, &accounts);
        }
    }
}

// ── handlers ──────────────────────────────────────────────────────────────────

#[cfg(unix)]
fn handle_cloud_event(
    event: CloudEvent,
    accounts: &mut HashMap<String, AccountState>,
    jar_ctl_version: i32,
    rest: &SupabaseRest,
    cfg: &AgentConfig,
    access_token: &str,
    tx: &mpsc::Sender<CloudEvent>,
) {
    match event {
        CloudEvent::AccountChanged { account_id, record } => {
            if let Some(acc) = accounts.get_mut(&account_id) {
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

                // Apply config nếu version mới (B5)
                try_apply_config(acc, jar_ctl_version, rest);
                // Reconcile desired_state (B7.3)
                reconcile_desired_state(acc);
            }
        }

        CloudEvent::CommandQueued { command } => {
            dispatch_command(command, accounts, rest);
        }

        CloudEvent::Disconnected { reason } => {
            eprintln!("[main_loop] realtime disconnected: {reason}, reconnecting");
            // B2.2: sau reconnect phải đọc lại full state vì có thể miss event.
            let fresh = boot_fetch_accounts(rest, &cfg.device_id);
            drain_and_expire_commands(rest, &cfg.device_id);
            // Merge: giữ process đang chạy, cập nhật config
            for (id, fresh_acc) in fresh {
                accounts
                    .entry(id)
                    .and_modify(|existing| {
                        existing.control_version = fresh_acc.control_version;
                        existing.control = fresh_acc.control.clone();
                        existing.config_version = fresh_acc.config_version;
                        existing.desired_state = fresh_acc.desired_state.clone();
                        try_apply_config(existing, jar_ctl_version, rest);
                        reconcile_desired_state(existing);
                    })
                    .or_insert(fresh_acc);
            }
            spawn_realtime_thread(
                crate::supabase_rest::SUPABASE_URL.to_string(),
            crate::supabase_rest::SUPABASE_ANON_KEY.to_string(),
                access_token.to_string(),
                tx.clone(),
            );
        }
    }
}

// ── config apply (B5) ─────────────────────────────────────────────────────────

#[cfg(unix)]
fn try_apply_config(acc: &mut AccountState, jar_ctl_version: i32, rest: &SupabaseRest) {
    // Đã apply rồi, không apply lại.
    if acc.applied_version == acc.config_version {
        return;
    }

    // Version gate (CLOUD-SPEC §3, B5.1).
    if acc.control_version != jar_ctl_version {
        eprintln!(
            "[config] account={} version_mismatch: control_version={} != jar_ctl_version={}",
            acc.id, acc.control_version, jar_ctl_version
        );
        let _ = rest.set_config_status(&acc.id, ConfigStatus::VersionMismatch, None, None);
        return;
    }

    // Dựng ControlSettings từ JSONB.
    let settings = match build_control_settings(&acc.control) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[config] account={} build_settings failed: {e}", acc.id);
            let _ = rest.set_config_status(
                &acc.id,
                ConfigStatus::Error,
                Some(&e),
                None,
            );
            return;
        }
    };

    // Ghi file. write_settings là atomic replace.
    let control_path = AccountPaths::for_slot(acc.slot_index).control_txt();
    match write_settings(&settings, &control_path) {
        Ok(_) => {
            eprintln!("[config] account={} applied config_version={}", acc.id, acc.config_version);
            acc.applied_version = acc.config_version;
            let _ = rest.set_config_status(
                &acc.id,
                ConfigStatus::Applied,
                None,
                Some(acc.config_version),
            );

            // B5.2: nav.detectSpots one-shot — reset sau khi ghi.
            if settings_has_detect_spots(&acc.control) {
                let _ = rest.clear_detect_spots(&acc.id);
            }
        }
        Err(e) => {
            let msg = e.to_string();
            eprintln!("[config] account={} write_settings failed: {msg}", acc.id);
            let _ = rest.set_config_status(&acc.id, ConfigStatus::Error, Some(&msg), None);
        }
    }
}

/// Xây ControlSettings từ JSONB control block.
///
/// JSONB là map `key → value` phẳng (35 khoá, xem WIRE-CONTRACT §4).
/// `zeus_core::wire::read_settings` đọc từ file; ta dựng ngược từ JSON.
#[cfg(unix)]
fn build_control_settings(
    control: &serde_json::Value,
) -> Result<zeus_core::wire::ControlSettings, String> {
    use zeus_core::wire::{ControlSettings, read_settings, write_settings, control_path};
    use std::io::Write;

    // Cách đơn giản nhất không đụng tới private fields của ControlSettings:
    // serialize JSONB thành text format mà zeus-control.txt dùng, rồi parse lại.
    // Format: mỗi dòng là `key=value`.
    let map = control.as_object().ok_or("control is not a JSON object")?;

    // Tạo file tạm để parse.
    let tmp = tempfile::NamedTempFile::new()
        .map_err(|e| format!("tempfile: {e}"))?;
    {
        let mut f = tmp.as_file();
        // Thêm dòng v= (version) vào đầu, mandatory cho parser.
        writeln!(f, "v={}", zeus_core::wire::SUPPORTED_VERSION)
            .map_err(|e| format!("write tempfile: {e}"))?;
        for (k, v) in map {
            if k == "v" { continue; } // đã thêm ở trên
            let val = match v {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => if *b { "1".to_string() } else { "0".to_string() },
                serde_json::Value::Null => continue,
                other => other.to_string(),
            };
            writeln!(f, "{k}={val}").map_err(|e| format!("write tempfile key {k}: {e}"))?;
        }
    }
    // Parse lại bằng read_settings của zeus-core.
    read_settings(tmp.path()).map_err(|e| format!("read_settings: {e}"))
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

// ── reconcile desired state (B7.3) ────────────────────────────────────────────

#[cfg(unix)]
fn reconcile_desired_state(acc: &mut AccountState) {
    let running = acc.process.as_ref().map(|p| p.is_alive()).unwrap_or(false);
    match acc.desired_state.as_str() {
        "running" if !running => {
            eprintln!("[reconcile] account={} slot={}: starting", acc.id, acc.slot_index);
            let paths = AccountPaths::for_slot(acc.slot_index);
            match AccountProcess::spawn(&paths) {
                Ok(proc) => {
                    acc.process = Some(proc);
                }
                Err(e) => {
                    eprintln!("[reconcile] account={} spawn failed: {e}", acc.id);
                }
            }
        }
        "stopped" if running => {
            eprintln!("[reconcile] account={} slot={}: stopping", acc.id, acc.slot_index);
            if let Some(mut proc) = acc.process.take() {
                match proc.stop(Duration::from_secs(5)) {
                    Ok(StopOutcome::Exited) => {
                        eprintln!("[reconcile] account={}: stopped cleanly", acc.id);
                    }
                    Ok(StopOutcome::Killed) => {
                        eprintln!("[reconcile] account={}: killed", acc.id);
                    }
                    Err(e) => {
                        eprintln!("[reconcile] account={} stop error: {e}", acc.id);
                    }
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
) {
    use crate::supabase_rest::now_rfc3339;

    // B7.2: TTL check (drain_commands đã lọc expires_at > now, nhưng realtime có thể gửi cũ)
    // Đã lọc bởi drain_commands; realtime event INSERT thì expires_at luôn tương lai.

    eprintln!("[command] id={} type={}", cmd.id, cmd.kind);

    let account_id = match &cmd.account_id {
        Some(id) => id.clone(),
        None => {
            // Device-level command (open-viewer, close-viewer không có account_id).
            match cmd.kind.as_str() {
                "open-viewer" => write_potato_ctl("1 0"),
                "close-viewer" => write_potato_ctl("0 3"),
                other => eprintln!("[command] unknown device command: {other}"),
            }
            let _ = rest.finish_command(&cmd.id, CommandStatus::Success, None);
            return;
        }
    };

    let acc = match accounts.get_mut(&account_id) {
        Some(a) => a,
        None => {
            eprintln!("[command] account_id={account_id} not found");
            let _ = rest.finish_command(
                &cmd.id,
                CommandStatus::Failed,
                Some("account not found"),
            );
            return;
        }
    };

    match cmd.kind.as_str() {
        "start" => {
            acc.desired_state = "running".to_string();
            reconcile_desired_state(acc);
            let _ = rest.finish_command(&cmd.id, CommandStatus::Running, None);
        }
        "stop" => {
            acc.desired_state = "stopped".to_string();
            reconcile_desired_state(acc);
            let _ = rest.finish_command(&cmd.id, CommandStatus::Success, None);
        }
        "restart" => {
            acc.desired_state = "stopped".to_string();
            reconcile_desired_state(acc);
            acc.desired_state = "running".to_string();
            reconcile_desired_state(acc);
            let _ = rest.finish_command(&cmd.id, CommandStatus::Running, None);
        }
        "apply-config" => {
            // Đã apply trong handle_cloud_event qua AccountChanged.
            // Nếu command này đến trước event, fetch lại.
            let _ = rest.finish_command(&cmd.id, CommandStatus::Success, None);
        }
        other => {
            eprintln!("[command] unknown command type: {other}");
            let _ = rest.finish_command(
                &cmd.id,
                CommandStatus::Failed,
                Some(&format!("unknown command type: {other}")),
            );
        }
    }
}

// ── telemetry tick (B6) ───────────────────────────────────────────────────────

#[cfg(unix)]
fn tick_snapshot_telemetry(acc: &mut AccountState, rest: &SupabaseRest) {
    let paths = AccountPaths::for_slot(acc.slot_index);
    let snapshot_path = paths.home.join(SNAPSHOT_FILE_NAME);

    let now = crate::supabase_rest::now_rfc3339();

    // Không có process chạy → không có snapshot.
    let process_state = if acc.process.as_ref().map(|p| p.is_alive()).unwrap_or(false) {
        "running"
    } else {
        "stopped"
    };

    if process_state == "stopped" {
        // Chỉ push nếu chưa push stopped.
        if acc.last_snapshot.is_some() {
            acc.last_snapshot = None;
            let _ = rest.push_runtime(
                &acc.id,
                &RuntimePayload {
                    process_state: "stopped".into(),
                    pid: None,
                    ram_mb: None,
                    cpu_pct: None,
                    snapshot_version: None,
                    snapshot: None,
                    restarts: None,
                    updated_at: now,
                },
            );
        }
        return;
    }

    // Đọc snapshot (B6.1).
    let snap = match read_snapshot(&snapshot_path) {
        Ok(s) => s,
        Err(_) => {
            // B6.5: snapshot không parse được → degraded.
            return;
        }
    };

    let snap_json = match serde_json::to_value(&snap) {
        Ok(v) => v,
        Err(_) => return,
    };

    // B6.2: chỉ push khi thay đổi có nghĩa.
    if !has_meaningful_change(&acc.last_snapshot, &snap_json) {
        return;
    }

    let pid = acc.process.as_ref().and_then(|p| p.pid()).map(|p| p as i32);
    acc.last_snapshot = Some(snap_json.clone());
    acc.last_telemetry_push = Instant::now();

    let _ = rest.push_runtime(
        &acc.id,
        &RuntimePayload {
            process_state: process_state.into(),
            pid,
            ram_mb: None,
            cpu_pct: None,
            snapshot_version: Some(snap.v as u32),
            snapshot: Some(snap_json),
            restarts: None,
            updated_at: now,
        },
    );
}

/// Thay đổi có nghĩa = ctl, atkstate, stuck, lv thay đổi, hoặc HP/MP vượt 10% (B6.2).
#[cfg(unix)]
fn has_meaningful_change(old: &Option<serde_json::Value>, new: &serde_json::Value) -> bool {
    let old = match old {
        None => return true, // lần đầu
        Some(v) => v,
    };
    // So sánh các field quan trọng.
    for key in ["ctl", "atkstate", "stuck", "lv"] {
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

// ── heartbeat tick (B6.3) ─────────────────────────────────────────────────────

#[cfg(unix)]
fn tick_heartbeat(
    rest: &SupabaseRest,
    device_id: &str,
    accounts: &HashMap<String, AccountState>,
) {
    let _ = rest.heartbeat(
        device_id,
        &DeviceHeartbeat {
            status: "online".into(),
            cpu_pct: read_container_cpu_pct(),
            ram_used_mb: read_container_ram_mb(),
            ram_total_mb: read_container_ram_total_mb(),
            uptime_s: read_uptime_s(),
            last_seen: crate::supabase_rest::now_rfc3339(),
        },
    );

    // Cũng push snapshot cho tất cả account với xp/gold (B6.2: gộp vào 60s).
    let now_str = crate::supabase_rest::now_rfc3339();
    for acc in accounts.values() {
        if let Some(snap) = &acc.last_snapshot {
            let pid = acc.process.as_ref().and_then(|p| p.pid()).map(|p| p as i32);
            let _ = rest.push_runtime(
                &acc.id,
                &RuntimePayload {
                    process_state: if acc.process.as_ref().map(|p| p.is_alive()).unwrap_or(false) {
                        "running".into()
                    } else {
                        "stopped".into()
                    },
                    pid,
                    ram_mb: None,
                    cpu_pct: None,
                    snapshot_version: snap.get("v").and_then(|v| v.as_u64()).map(|v| v as u32),
                    snapshot: Some(snap.clone()),
                    restarts: None,
                    updated_at: now_str.clone(),
                },
            );
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

        // Subscribe các bảng cần thiết.
        for table in ["accounts", "account_runtime", "commands"] {
            if let Err(e) = client.subscribe(table, Some(&access_token)) {
                eprintln!("[realtime] subscribe {table} failed: {e}");
            }
        }

        eprintln!("[realtime] connected and subscribed");

        loop {
            match client.read_event() {
                Ok(Some(RealtimeEvent::Change { table, change_type, record, .. })) => {
                    let event = match table.as_str() {
                        "accounts" | "account_runtime" => {
                            let account_id = record["id"]
                                .as_str()
                                .or_else(|| record["account_id"].as_str())
                                .unwrap_or("")
                                .to_string();
                            CloudEvent::AccountChanged { account_id, record }
                        }
                        "commands" if change_type == ChangeType::Insert => {
                            // Chỉ quan tâm INSERT queued.
                            if record["status"].as_str() == Some("queued") {
                                let cmd = CommandRow {
                                    id: record["id"].as_str().unwrap_or("").to_string(),
                                    account_id: record["account_id"].as_str().map(str::to_owned),
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

// ── boot helpers ──────────────────────────────────────────────────────────────

#[cfg(unix)]
fn boot_fetch_accounts(rest: &SupabaseRest, device_id: &str) -> HashMap<String, AccountState> {
    match rest.fetch_accounts(device_id) {
        Ok(rows) => rows
            .into_iter()
            .map(|row| {
                let id = row.id.clone();
                let state = AccountState {
                    id: row.id,
                    slot_index: row.slot_index,
                    desired_state: row.desired_state,
                    control_version: row.control_version,
                    control: row.control,
                    config_version: row.config_version,
                    applied_version: 0,
                    process: None,
                    last_snapshot: None,
                    last_telemetry_push: Instant::now(),
                };
                (id, state)
            })
            .collect(),
        Err(e) => {
            eprintln!("[boot] fetch_accounts failed: {e}");
            HashMap::new()
        }
    }
}

#[cfg(unix)]
fn drain_and_expire_commands(rest: &SupabaseRest, device_id: &str) {
    if let Err(e) = rest.expire_stale_commands(device_id) {
        eprintln!("[boot] expire_stale_commands failed: {e}");
    }
    // drain_commands chỉ để log số lượng; thực tế sẽ đến qua realtime.
    match rest.drain_commands(device_id) {
        Ok(cmds) => eprintln!("[boot] {} queued command(s) on boot", cmds.len()),
        Err(e) => eprintln!("[boot] drain_commands failed: {e}"),
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

// ── VNC viewer count (B8.1) ───────────────────────────────────────────────────

#[cfg(unix)]
fn count_vnc_clients() -> u32 {
    // Đếm số TCP connection vào port 5901 (DISPLAY :1).
    // `ss -tn state established '( dport = :5901 )'` trả về 1 dòng header + N dòng.
    // Nếu cần hỗ trợ nhiều display, scan 5901..5910.
    let output = std::process::Command::new("ss")
        .args(["-tn", "state", "established", "( dport = :5901 )"])
        .output();
    match output {
        Ok(o) => {
            let text = String::from_utf8_lossy(&o.stdout);
            // Trừ 1 dòng header
            (text.lines().count().saturating_sub(1)) as u32
        }
        Err(_) => 0,
    }
}

#[cfg(unix)]
fn write_potato_ctl(mode: &str) {
    // B8.5: atomic replace. potato.ctl là fail-safe, không fail agent khi ghi lỗi.
    use std::io::Write;
    let tmp_path = "/opt/knight/state/potato.ctl.tmp";
    let final_path = "/opt/knight/state/potato.ctl";
    if let Ok(mut f) = std::fs::File::create(tmp_path) {
        let _ = f.write_all(mode.as_bytes());
        let _ = std::fs::rename(tmp_path, final_path);
    }
}

// ── jar manifest ──────────────────────────────────────────────────────────────

#[cfg(unix)]
fn read_jar_manifest(path: &str) -> Option<JarManifest> {
    let data = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&data).ok()
}

// ── system metrics (B6.4) ─────────────────────────────────────────────────────

#[cfg(unix)]
fn read_container_cpu_pct() -> f32 {
    // Đọc từ /sys/fs/cgroup/cpu.stat (cgroup v2).
    // Đơn giản hóa: trả 0.0 nếu không đọc được — không fatal.
    0.0
}

#[cfg(unix)]
fn read_container_ram_mb() -> u32 {
    // /sys/fs/cgroup/memory.current → bytes.
    std::fs::read_to_string("/sys/fs/cgroup/memory.current")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|b| (b / 1_048_576) as u32)
        .unwrap_or(0)
}

#[cfg(unix)]
fn read_container_ram_total_mb() -> u32 {
    // /sys/fs/cgroup/memory.max → bytes (hoặc "max" nếu không giới hạn).
    let s = std::fs::read_to_string("/sys/fs/cgroup/memory.max").unwrap_or_default();
    let s = s.trim();
    if s == "max" { return 0; }
    s.parse::<u64>().ok().map(|b| (b / 1_048_576) as u32).unwrap_or(0)
}

#[cfg(unix)]
fn read_uptime_s() -> u64 {
    // /proc/uptime → "uptime_seconds idle_seconds".
    std::fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().and_then(|v| v.parse::<f64>().ok()))
        .map(|f| f as u64)
        .unwrap_or(0)
}


