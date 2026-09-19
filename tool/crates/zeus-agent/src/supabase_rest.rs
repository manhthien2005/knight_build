//! Chiều GHI lên Supabase, qua PostgREST. Không bị chặn — REST là protocol có tài liệu.
//!
//! Target path: `Tool/tool/crates/zeus-agent/src/supabase_rest.rs`
//!
//! ## Vì sao có hai file cho cùng một backend
//!
//! Đọc và đi theo hai đường khác nhau, và đó là cố ý:
//!
//! | Chiều | Đường | Lý do |
//! |---|---|---|
//! | Supabase → agent | realtime WSS (`supabase_realtime.rs`) | Supabase free cho **5 GB DB egress/tháng**. Poll PostgREST mỗi 5 s tốn 0.5–1 GB/tháng *mỗi agent* ⇒ 5 GB chỉ chịu được 5–10 agent. Một socket + heartbeat thì gần như miễn phí |
//! | agent → Supabase | REST (`file này`) | Ghi không tốn egress đáng kể, và REST cho `Prefer: return=representation` để đọc lại đúng hàng vừa ghi |
//!
//! Trần scale thật của hệ là **200 concurrent realtime connection** ⇒ ~190 agent. Không phải
//! RAM, không phải DB.
//!
//! ## Khoá: anon key + device session, KHÔNG service_role
//!
//! service_role key nhúng trong image nghĩa là một node bị chiếm đọc được **cả bảng của mọi
//! user**. Agent chỉ cần anon key cộng với JWT của device session; RLS (sql/002_rls.sql) giới
//! hạn nó vào đúng device của nó.
//!
//! ## Blocking, không async
//!
//! `ureq` là client đồng bộ. Mọi call ở đây được gọi từ vòng chính — không có thread pool,
//! không có `async fn`. Nếu một call chậm, vòng chính chậm; đó là lý do mọi request phải đặt
//! timeout ngắn và **không được** nằm trong đường dẫn của tick 2 s.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Compile-time constants — anon key is public by design (all security comes from RLS).
/// See discussion in AGENT-SPEC §4.2 / B2.5.
pub const SUPABASE_URL: &str = "https://wuyxkksihkmsmwuiuvdk.supabase.co";
pub const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6Ind1eXhra3NpaGttc213dWl1dmRrIiwicm9sZSI6ImFub24iLCJpYXQiOjE3ODkxMDE1NTIsImV4cCI6MjEwNDY3NzU1Mn0.MAbQ_gPrLwh1r49f6KxFGUUFCP5Yr7c_S58f3JTvRTw";

/// Client đã cấu hình. Rẻ để clone, giữ trong state của vòng chính.
pub struct SupabaseRest {
    /// `https://<project>.supabase.co`
    base_url: String,
    anon_key: String,
    /// JWT của device session. Refresh trước khi hết hạn (~1 h).
    access_token: Option<String>,
    agent: ureq::Agent,
}

/// Một request REST phải có timeout. Không có nó, một lần Supabase treo là cả agent treo —
/// và game thì không được dừng vì cloud không trả lời.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(8);

impl SupabaseRest {
    pub fn new(base_url: String, anon_key: String) -> Self {
        Self {
            base_url,
            anon_key,
            access_token: None,
            agent: ureq::Agent::new_with_config(
                ureq::Agent::config_builder()
                    .timeout_connect(Some(Duration::from_secs(5)))
                    .timeout_global(Some(REQUEST_TIMEOUT))
                    // ureq 3 dropped the response body from `Error::StatusCode(u16)`. Keeping
                    // non-2xx on the Ok path is what still lets `RestError::Http` carry the
                    // PostgREST message - the thing that separates "RLS misconfigured" from
                    // "the row is gone".
                    .http_status_as_error(false)
                    .build(),
            ),
        }
    }

    pub fn set_access_token(&mut self, token: String) {
        self.access_token = Some(token);
    }

    /// Header chung. `apikey` là anon key (Supabase yêu cầu cả hai khi có JWT).
    ///
    /// Trả về `RestRequest` chứ không phải builder của ureq: xem doc của kiểu đó để biết vì sao
    /// tầng này phải tồn tại.
    fn request<'s>(&'s self, method: &'s str, path: &str) -> RestRequest<'s> {
        let mut request = RestRequest {
            agent: &self.agent,
            method,
            url: format!("{}{}", self.base_url, path),
            headers: Vec::new(),
        }
        .header("apikey", self.anon_key.clone());
        if let Some(token) = &self.access_token {
            request = request.header("Authorization", format!("Bearer {token}"));
        }
        request
    }

    // ── devices ───────────────────────────────────────────────────────────────

    /// Heartbeat + metrics, nhịp 60 s. **Đây là thứ giữ project Supabase free không bị pause**
    /// (free tier pause sau 7 ngày không hoạt động) — nên nó phải chạy kể cả khi không có gì
    /// để báo.
    ///
    /// `Prefer: return=minimal` vì không cần đọc lại hàng của chính mình.
    pub fn heartbeat(&self, device_id: &str, payload: &DeviceHeartbeat) -> Result<(), RestError> {
        self.request(
            "PATCH",
            &format!("/rest/v1/devices?id=eq.{device_id}"),
        )
        .prefer("return=minimal")
        .send_json(payload)?;
        Ok(())
    }

    /// Công bố hợp đồng jar lúc boot. Web đọc hai cột version này để quyết định render form
    /// nào; agent đọc chúng để quyết định có được ghi control file hay không.
    ///
    /// Nguồn là `/opt/knight/game/zeus-jar.json` — **không parse bytecode**. Build script đã
    /// biết các con số; đọc lại từ class file là tự tạo thêm một nguồn có thể sai.
    pub fn announce_jar_contract(&self, device_id: &str, manifest: &JarManifest) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/devices?id=eq.{device_id}"))
            .prefer("return=minimal")
            .send_json(serde_json::json!({
                "jar_sha256": manifest.jar_sha256,
                "jar_ctl_version": manifest.ctl_version,
                "jar_snapshot_version": manifest.snapshot_version,
                "jar_ctl_key_count": manifest.ctl_key_count,
                "agent_version": manifest.agent_version,
                "status": "online",
            }))?;
        Ok(())
    }

    /// Công bố URL viewer khi tunnel mở. `None` để xoá.
    pub fn set_viewer(&self, device_id: &str, url: Option<(&str, &str)>) -> Result<(), RestError> {
        let body = match url {
            Some((u, expires)) => serde_json::json!({"viewer_url": u, "viewer_expires_at": expires}),
            None => serde_json::json!({"viewer_url": null, "viewer_expires_at": null}),
        };
        self.request("PATCH", &format!("/rest/v1/devices?id=eq.{device_id}"))
            .prefer("return=minimal")
            .send_json(body)?;
        Ok(())
    }

    // ── account_runtime ───────────────────────────────────────────────────────

    /// Đẩy telemetry. **Chỉ gọi khi có thay đổi ý nghĩa**, không mỗi lần poll snapshot.
    ///
    /// Thay đổi ý nghĩa = `settings_agreed` (khoá `ctl`), `attack_state`, `stuck`, `level` đổi,
    /// hoặc HP/MP vượt một ngưỡng 10%. `xp`/`gold` gộp vào nhịp 60 s. Lý do: mỗi PATCH là một
    /// hàng được broadcast tới mọi subscriber realtime, và web không cần biết XP tăng 3 permille.
    pub fn push_runtime(&self, account_id: &str, payload: &RuntimePayload) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/account_runtime?account_id=eq.{account_id}"))
            .prefer("return=minimal")
            .send_json(payload)?;
        Ok(())
    }

    /// Trạng thái apply config. `version_mismatch` là trạng thái **phải** hiện lên web:
    /// nó là dấu hiệu duy nhất của cú vỡ im lặng (jar fail-closed, JVM vẫn sống).
    pub fn set_config_status(
        &self,
        account_id: &str,
        status: ConfigStatus,
        detail: Option<&str>,
        applied_version: Option<i32>,
    ) -> Result<(), RestError> {
        let mut body = serde_json::json!({
            "config_status": status.as_str(),
            "config_error": detail,
        });
        if let Some(version) = applied_version {
            body["applied_version"] = serde_json::json!(version);
        }
        self.request("PATCH", &format!("/rest/v1/account_runtime?account_id=eq.{account_id}"))
            .prefer("return=minimal")
            .send_json(body)?;
        Ok(())
    }

    // ── commands ──────────────────────────────────────────────────────────────

    /// Đánh dấu một command. `message` là lý do người đọc được — đừng bỏ trống khi failed.
    pub fn finish_command(
        &self,
        command_id: &str,
        status: CommandStatus,
        message: Option<&str>,
    ) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/commands?id=eq.{command_id}"))
            .prefer("return=minimal")
            .send_json(serde_json::json!({
                "status": status.as_str(),
                "message": message,
                "finished_at": now_rfc3339(),
            }))?;
        Ok(())
    }

    /// Drain hàng đợi. Gọi lúc boot **và sau mỗi lần reconnect** — realtime có thể đã miss
    /// event trong lúc đứt, và một command queued không được thấy là một command không bao giờ
    /// chạy.
    ///
    /// `expires_at=gt.now()` là TTL: một command user bấm ba ngày trước không được nổ lúc boot.
    /// Command quá hạn phải được đánh dấu `expired`, không phải bỏ qua im lặng — nếu không thì
    /// web cứ hiện `queued` mãi.
    pub fn drain_commands(&self, device_id: &str) -> Result<Vec<CommandRow>, RestError> {
        let response = self
            .request(
                "GET",
                &format!(
                    "/rest/v1/commands?device_id=eq.{device_id}&status=eq.queued\
                     &expires_at=gt.{now}&order=created_at.asc",
                    now = now_rfc3339()
                ),
            )
            .call()?;
        let rows: Vec<CommandRow> = response.json()?;
        Ok(rows)
    }

    /// Đánh dấu command quá hạn để web không hiện `queued` vô thời hạn.
    pub fn expire_stale_commands(&self, device_id: &str) -> Result<u32, RestError> {
        let response = self
            .request(
                "PATCH",
                &format!(
                    "/rest/v1/commands?device_id=eq.{device_id}&status=eq.queued\
                     &expires_at=lte.{now}",
                    now = now_rfc3339()
                ),
            )
            // `return=representation` để đếm được bao nhiêu hàng đã đổi.
            .prefer("return=representation")
            .send_json(serde_json::json!({
                "status": "expired",
                "message": "TTL elapsed before the node saw this command",
                "finished_at": now_rfc3339(),
            }))?;
        let rows: Vec<serde_json::Value> = response.json()?;
        Ok(rows.len() as u32)
    }

    // ── accounts ──────────────────────────────────────────────────────────────

    /// Đọc lại full state. **Bắt buộc sau reconnect** và lúc boot.
    ///
    /// Không được giả định realtime đã giao đủ event: một socket đứt 30 s có thể bỏ lọt đúng
    /// cái `apply-config` user vừa bấm.
    pub fn fetch_accounts(&self, device_id: &str) -> Result<Vec<AccountRow>, RestError> {
        let response = self
            .request("GET", &format!("/rest/v1/accounts?device_id=eq.{device_id}&order=slot_index.asc"))
            .call()?;
        response.json()
    }

    /// `nav.detectSpots` là **one-shot**: nó là một yêu cầu chứ không phải một trạng thái, và
    /// kênh duy nhất tới mod là ghi vào settings. Nếu agent không tự đặt lại `false`, một lần
    /// bấm sẽ nổ mãi mãi mỗi chu kỳ ghi config.
    ///
    /// Ghi lại bằng RPC thay vì PATCH trực tiếp để logic "đọc-sửa-ghi" nằm một chỗ phía DB.
    pub fn clear_detect_spots(&self, account_id: &str) -> Result<(), RestError> {
        self.request("POST", "/rest/v1/rpc/clear_detect_spots")
            .send_json(serde_json::json!({"target_account": account_id}))?;
        Ok(())
    }

    // ── pairing ───────────────────────────────────────────────────────────────
    //
    // REMOVED: check_device_claimed() — GET /rest/v1/devices?id=eq.{id}&select=user_id
    //
    // That method read the `devices` table anonymously. RLS `own_devices` policy
    // (set in 003_device_auth.sql) blocks any SELECT without a valid JWT, returning:
    //   HTTP 401: "permission denied for table devices"
    //
    // The correct claim-detection mechanism is to attempt sign_in_as_device() in a
    // loop. When claim_device() runs on the dashboard, it creates the auth.users entry
    // for the device. From that point sign_in_as_device() returns a JWT.
    //
    // REMOVED: poll_claim() — this was an old design that called claim_device() RPC
    // from the agent. The actual claim is done by the web UI (authenticated user).
    //
    // See pairing.rs attempt_sign_in() for the complete error classification.

    /// Tạo hàng device chưa pair qua RPC `register_device` (SECURITY DEFINER — anon có thể gọi).
    ///
    /// Wire contract:
    ///   `pubkey_b64` = `BASE64_STANDARD.encode(pubkey_sec1_bytes)` (standard alphabet, padding)
    ///   SQL `register_device` receives this as `p_pubkey TEXT` and calls
    ///   `decode(p_pubkey, 'base64') → BYTEA` before storing in `devices.pubkey`.
    ///   Never pass raw bytes or hex here.
    pub fn create_unpaired_device(
        &self,
        pair_code: &str,
        name: &str,
        pubkey_b64: &str,
    ) -> Result<String, RestError> {
        let response = self
            .request("POST", "/rest/v1/rpc/register_device")
            .send_json(serde_json::json!({
                "p_pair_code": pair_code,
                "p_name": name,
                "p_pubkey": pubkey_b64,
            }))?;
        let value: serde_json::Value = response.json()?;
        value.as_str()
            .map(str::to_owned)
            .ok_or_else(|| RestError::Decode("register_device: no uuid in response".into()))
    }

    /// Đặt desired_state của account lên DB. Phải gọi khi nhận lệnh start/stop/restart
    /// để trạng thái trong DB khớp với RAM — Issue #97, tránh reconnect giết bot.
    pub fn set_account_desired_state(&self, account_id: &str, state: &str) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/accounts?id=eq.{account_id}"))
            .prefer("return=minimal")
            .send_json(serde_json::json!({"desired_state": state}))?;
        Ok(())
    }

    /// Đặt trạng thái device (online/offline). Gọi khi SIGTERM để web biết node đã tắt — Issue #91.
    pub fn set_device_status(&self, device_id: &str, status: &str) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/devices?id=eq.{device_id}"))
            .prefer("return=minimal")
            .send_json(serde_json::json!({"status": status}))?;
        Ok(())
    }

    /// Dang nhap bang credentials duoc derive tu pubkey (khong can env var nao).
    ///
    /// Formula phai khop chinh xac voi SQL trong 003_device_auth.sql:
    ///   device_email    = "device-" + device_id + "@zeus.internal"
    ///   device_password = hex(sha256(pubkey_sec1_bytes))[0..32]
    pub fn sign_in_as_device(
        &self,
        device_id: &str,
        pubkey_sec1: &[u8],
    ) -> Result<String, RestError> {
        use sha2::{Sha256, Digest};
        let hash = Sha256::digest(pubkey_sec1);
        let device_email = format!("device-{}@zeus.internal", device_id);
        let device_password = hex::encode(&hash)[..32].to_string();

        let response = self
            .request("POST", "/auth/v1/token?grant_type=password")
            .send_json(serde_json::json!({
                "email": device_email,
                "password": device_password,
            }))?;
        let body: serde_json::Value = response.json()?;
        body["access_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| RestError::Decode(
                format!("sign_in_as_device: no access_token (device_id={})", device_id)
            ))
    }
}

// ── payloads ────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, serde::Serialize)]
pub struct DeviceHeartbeat {
    pub status: String,
    pub cpu_pct: f32,
    pub ram_used_mb: u32,
    pub ram_total_mb: u32,
    pub uptime_s: u64,
    pub last_seen: String,
}

/// Nội dung `zeus-jar.json`. Agent đọc file này lúc boot, không parse bytecode.
#[derive(Debug, Clone, serde::Deserialize, serde::Serialize)]
pub struct JarManifest {
    pub jar_sha256: String,
    pub jar_size: u64,
    pub ctl_version: u32,
    pub snapshot_version: u32,
    pub ctl_key_count: u32,
    pub snapshot_key_count: u32,
    pub built_at: String,
    /// sha256 của `PatchZeus.java` đã dùng. Có mặt vì hai bản patcher từng sinh ra jar hành vi
    /// khác nhau từ cùng một source (`../../cross-domain/VERIFY-RESULTS.md` §3).
    pub patcher_sha256: String,
    /// Thêm để agent tự khai, không phải để tin: một image build sai có thể khai gian, nhưng
    /// khi đó nó cũng khai sai mọi thứ khác và sẽ bị bắt ở bước khác.
    /// `#[serde(default)]` để không fail khi field này vắng mặt trong zeus-jar.json — Issue #09.
    #[serde(default)]
    pub agent_version: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimePayload {
    pub process_state: String,
    pub pid: Option<i32>,
    pub ram_mb: Option<u32>,
    pub cpu_pct: Option<f32>,
    pub snapshot_version: Option<u32>,
    /// Snapshot **nguyên văn**, không lược bớt. Web cần những khoá mà agent không hiểu
    /// (`pkrank`, `buffs`, `travel*`) và việc lọc ở đây sẽ âm thầm bỏ thông tin chẩn đoán.
    pub snapshot: Option<serde_json::Value>,
    pub restarts: Option<u32>,
    pub updated_at: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AccountRow {
    pub id: String,
    pub slot_index: i32,
    pub label: String,
    pub username: String,
    pub secret_sealed: serde_json::Value,
    pub server_index: i16,
    pub desired_state: String,
    pub control_version: i32,
    pub control: serde_json::Value,
    pub config_version: i32,
    pub runtime: serde_json::Value,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CommandRow {
    pub id: String,
    pub account_id: Option<String>,
    /// device_id để lọc multi-node — Issue #98
    pub device_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub payload: Option<serde_json::Value>,
    pub expires_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigStatus {
    Applied,
    VersionMismatch,
    Error,
}

impl ConfigStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Applied => "applied",
            Self::VersionMismatch => "version_mismatch",
            Self::Error => "error",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandStatus {
    Running,
    Success,
    Failed,
    Expired,
}

impl CommandStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Success => "success",
            Self::Failed => "failed",
            Self::Expired => "expired",
        }
    }
}

/// Đánh giá kết quả lệnh start: thành công chỉ khi process thực sự đang chạy (alive).
pub fn evaluate_start_status(is_alive: bool) -> (CommandStatus, Option<&'static str>) {
    if is_alive {
        (CommandStatus::Success, None)
    } else {
        (CommandStatus::Failed, Some("process failed to start"))
    }
}

/// Đánh giá kết quả lệnh stop: thành công khi process không còn chạy (not alive).
pub fn evaluate_stop_status(is_alive: bool) -> (CommandStatus, Option<&'static str>) {
    if !is_alive {
        (CommandStatus::Success, None)
    } else {
        (CommandStatus::Failed, Some("process failed to stop"))
    }
}

/// Đánh giá kết quả lệnh restart: thành công chỉ khi replacement process thực sự đang chạy (alive).
pub fn evaluate_restart_status(is_alive: bool) -> (CommandStatus, Option<&'static str>) {
    if is_alive {
        (CommandStatus::Success, None)
    } else {
        (CommandStatus::Failed, Some("replacement process failed to start"))
    }
}

/// Đánh giá kết quả lệnh apply-config từ outcome của `try_apply_config`.
pub fn evaluate_apply_config_status(
    result: Result<(), &'static str>,
) -> (CommandStatus, Option<&'static str>) {
    match result {
        Ok(()) => (CommandStatus::Success, None),
        Err(err) => (CommandStatus::Failed, Some(err)),
    }
}

/// Outcome of stopping a supervised child process tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopOutcome {
    AlreadyGone,
    Terminated,
    Killed,
    Failed,
}

impl StopOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::AlreadyGone | Self::Terminated | Self::Killed)
    }
}

/// Postcondition decision for account state transition to "stopped".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopTransition {
    /// Process confirmed terminated or already absent: can clear process handle and runtime state.
    ConfirmedStopped,
    /// Process failed to terminate or still alive: MUST preserve process handle and state.
    FailedStillAlive,
}

/// Evaluates whether an account transition to "stopped" is confirmed complete.
pub fn evaluate_stop_transition(
    has_process: bool,
    is_alive: bool,
    stop_outcome: Option<StopOutcome>,
) -> StopTransition {
    if !has_process || !is_alive {
        return StopTransition::ConfirmedStopped;
    }
    match stop_outcome {
        Some(outcome) if outcome.is_success() => StopTransition::ConfirmedStopped,
        _ => StopTransition::FailedStillAlive,
    }
}

/// Precondition check for restarting an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPrecondition {
    Proceed,
    BlockedOldProcessAlive,
}

/// Restart must never spawn a replacement JVM until the old JVM is confirmed stopped.
pub fn check_restart_precondition(old_process_alive: bool) -> RestartPrecondition {
    if old_process_alive {
        RestartPrecondition::BlockedOldProcessAlive
    } else {
        RestartPrecondition::Proceed
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RestError {
    /// 4xx/5xx từ Supabase. `status` để phân biệt "RLS chặn" (401/403) với "hàng không tồn tại".
    #[error("supabase returned {status}: {body}")]
    Http { status: u16, body: String },
    #[error("network or timeout: {0}")]
    Transport(String),
    #[error("response was not valid JSON: {0}")]
    Decode(String),
}

// ── adapter ureq 3 ────────────────────────────────────────────────────────────────
//
// Spec viết cho ureq 2 (`AgentBuilder`, `Agent::request`, `Request::set`,
// `Error::Status(code, response)`), còn B0.4 pin `ureq = "3"` — 3.4.2. Sáu điểm API
// trong spec không biên dịch trên pin của chính nó, giống `process_unix.rs` đã gặp ở
// Task 7. Bảng đổi (đã thăm dò từng điểm trên ureq 3.4.2 thật, không đoán từ doc):
//
//   AgentBuilder::new().timeout_connect(d).timeout(d)   → Agent::config_builder()
//     .timeout_connect(Some(d)).timeout_global(Some(d)).http_status_as_error(false)
//   agent.request(method, url)                          → http::Request::builder().method(..).uri(..).body(Vec<u8>) + agent.run(..)
//   Request::set(name, value)                           → builder .header(name, value)
//   send_json(value) / call()                           → run() với body serialize sẵn
//   response.into_json::<T>()                           → body.read_to_vec + serde_json::from_slice
//   Error::Status(code, response)                       → Error::StatusCode(u16), KHÔNG kèm body
//
// `http_status_as_error(false)` là điểm thiết kế quan trọng nhất. ureq 3 đã bỏ body
// khỏi error status, mà `RestError::Http { status, body }` cần body — JSON lỗi của
// PostgREST là thứ phân biệt "RLS cấu hình sai" (401/403 + message) với "hàng không
// tồn tại" (mảng rỗng). Tắt status-as-error thì 4xx/5xx đến tay ta như Ok(Response)
// còn nguyên body; lỗi transport vẫn là Err như cũ.

/// Request đang dựng. Chỉ tồn tại để `SupabaseRest::request` có kiểu trả về.
pub struct RestRequest<'a> {
    agent: &'a ureq::Agent,
    method: &'a str,
    url: String,
    headers: Vec<(String, String)>,
}

impl<'a> RestRequest<'a> {
    /// Thêm header bất kỳ. Thay cho `.set(name, value)` của ureq 2, đã bị bỏ.
    fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.push((name.to_string(), value.into()));
        self
    }

    /// `Prefer` là header phổ biến nhất ở caller (return=minimal / return=representation)
    /// nên có shorthand riêng, tránh caller phải nhớ tên header.
    fn prefer(self, value: &str) -> Self {
        self.header("Prefer", value)
    }


    /// Gửi với JSON body. Serialize bằng `serde_json` rồi đẩy byte thô, vì
    /// `send_json` của ureq 3 cần feature `json` (thêm dep build lại; B0.4 chưa pin).
    fn send_json<T: serde::Serialize>(mut self, value: T) -> Result<RestResponse, RestError> {
        // Content-Type phải gắn trước body, và chỉ khi có body.
        self.headers
            .push(("Content-Type".to_string(), "application/json".to_string()));
        let body = serde_json::to_vec(&value).map_err(|e| RestError::Decode(e.to_string()))?;
        self.dispatch(Some(body))
    }

    /// Gửi không body (GET).
    fn call(self) -> Result<RestResponse, RestError> {
        self.dispatch(None)
    }

    /// Điểm duy nhất thực sự chạm mạng. Mọi lỗi HTTP/transport đều quy về đây.
    fn dispatch(self, body: Option<Vec<u8>>) -> Result<RestResponse, RestError> {
        let mut builder = ureq::http::Request::builder()
            .method(self.method)
            .uri(self.url);
        for (name, value) in &self.headers {
            builder = builder.header(name, value);
        }
        let request = builder
            .body(body.unwrap_or_default())
            .map_err(|e| RestError::Transport(e.to_string()))?;
        let response = self.agent.run(request).map_err(RestError::transport)?;
        let status = response.status().as_u16();
        // `http_status_as_error(false)` đưa 4xx/5xx về Ok; trạng thái vẫn kiểm tra ở đây
        // để caller nhận RestError::Http với body PostgREST trả, đúng ngữ nghĩa spec gốc.
        if (400..600).contains(&status) {
            let bytes = response
                .into_body()
                .read_to_vec()
                .map_err(RestError::transport)?;
            return Err(RestError::Http {
                status,
                body: String::from_utf8_lossy(&bytes).into_owned(),
            });
        }
        Ok(RestResponse { response })
    }
}

/// Response đã qua cổng 2xx. `json()` là điểm duy nhất parse body.
pub struct RestResponse {
    response: ureq::http::Response<ureq::Body>,
}

impl RestResponse {
    /// Thay `into_json::<T>()` của ureq 2. Parse lỗi thì `RestError::Decode`,
    /// không phải lỗi mạng — vòng chính cần phân biệt hai cái này để log đúng.
    fn json<T: serde::de::DeserializeOwned>(self) -> Result<T, RestError> {
        let bytes = self
            .response
            .into_body()
            .read_to_vec()
            .map_err(RestError::transport)?;
        serde_json::from_slice(&bytes).map_err(|e| RestError::Decode(e.to_string()))
    }
}

impl RestError {
    /// ureq::Error là enum non-exhaustive; cờ status-as-error đã tắt nên biến thể
    /// StatusCode gần như không xuất hiện, nhưng wildcard vẫn phải đón.
    fn transport(error: ureq::Error) -> Self {
        Self::Transport(error.to_string())
    }
}

// ── helpers ──────────────────────────────────────────────────────────────────────

/// RFC3339 UTC, ví dụ `2026-09-14T07:46:12Z`.
///
/// PostgREST so sánh `timestamptz` bằng chuỗi, nên định dạng phải đúng từng byte. Hai chi tiết
/// có tải:
///
/// * **Phải kết thúc bằng `Z`, không phải `+00:00`.** Giá trị này được nhúng thẳng vào query
///   string (`drain_commands` dựng `expires_at=gt.{now}`), và trong query thì `+` bị decode
///   thành dấu cách. Offset `+00:00` sẽ thành ` 00:00` và PostgREST trả về một lỗi khó hiểu
///   hoặc — tệ hơn — một phép so sánh sai. `Z` không có ký tự nào cần escape.
/// * **Không thêm `time`/`chrono`.** Cả hai đều chưa có trong `Cargo.lock`, nên kéo vào là mở
///   lại việc verify B0.4 (`panic = "abort"`) chỉ để đổi lấy một định dạng ngày. `zeus-core`
///   cũng dùng `SystemTime` trần (`rms.rs`), nên đây là theo convention sẵn có.
///
/// Đồng hồ trước 1970 làm `duration_since` lỗi. Không có fallback im lặng nào an toàn ở đây:
/// một `now` sai khiến `expires_at > now` khớp cả những command đã hết hạn từ lâu, tức là cho
/// nổ đúng thứ B7.2 cấm. Vậy nên fail to, giống các `.expect()` khác trong crate cho trạng
/// thái bất khả thi.
pub fn now_rfc3339() -> String {
    rfc3339_from_unix(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("container clock is before 1970 — every TTL comparison would be wrong")
            .as_secs(),
    )
}

/// RFC3339 UTC cho thời điểm `offset_secs` giây từ bây giờ.
///
/// Dùng để tính `expires_at` cho viewer URL (now + 15 phút = offset 900).
/// Không tính có thể thiếu import `time`/`chrono` — cùng thuật toán với `now_rfc3339`.
pub fn rfc3339_offset_from_now(offset_secs: u64) -> String {
    rfc3339_from_unix(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("container clock is before 1970")
            .as_secs()
            .saturating_add(offset_secs),
    )
}

/// Phần thuần của `now_rfc3339`, tách ra để test được ngày cụ thể mà không đóng băng đồng hồ.
fn rfc3339_from_unix(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let secs_of_day = secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        secs_of_day / 3_600,
        (secs_of_day % 3_600) / 60,
        secs_of_day % 60,
    )
}

/// Ngày dương lịch từ số ngày kể từ 1970-01-01, theo thuật toán civil_from_days của Howard
/// Hinnant. Chia theo era 400 năm nên đúng cho cả ngày âm (trước 1970) và mọi năm nhuận,
/// không cần bảng tra.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097) as u64; // [0, 146096]
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153; // [0, 11], March-based
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    // Year bắt đầu từ tháng Ba trong công thức trên, nên Jan/Feb thuộc năm sau.
    (year + i64::from(month <= 2), month, day)
}

/// Base64 alphabet **chuẩn** (`+/`, có padding `=`), không phải URL-safe (`-_`).
///
/// Phía Rust decode bằng `general_purpose::STANDARD` trong `crypto.rs`, và `devices.pubkey`
/// nằm trong body JSON chứ không trong path, nên alphabet chuẩn là đúng. Wrapper mỏng này tồn
/// tại để tên hàm nói rõ alphabet nào — đoán sai alphabet là một lỗi im lặng ở tầng crypto.
pub fn base64_standard(bytes: &[u8]) -> String {
    crate::crypto::b64_encode(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RLS chặn (401/403) và "hàng không tồn tại" (404 / mảng rỗng) là hai tình huống khác nhau
    /// và vòng chính phản ứng khác nhau: cái trước là cấu hình sai và phải log to, cái sau là
    /// account đã bị xoá và chỉ cần quên nó đi.
    #[test]
    fn http_errors_keep_the_status_code() {
        let error = RestError::Http {
            status: 401,
            body: r#"{"message":"JWT expired"}"#.into(),
        };
        assert!(matches!(error, RestError::Http { status: 401, .. }));
        assert!(error.to_string().contains("401"));
    }

    #[test]
    fn config_status_strings_match_the_schema_check_constraint() {
        // sql/001_schema.sql ràng buộc `config_status` bằng CHECK. Nếu hai bên lệch, mọi lần
        // ghi telemetry sẽ 4xx — nên assert ở đây rẻ hơn nhiều so với phát hiện trong log.
        assert_eq!(ConfigStatus::Applied.as_str(), "applied");
        assert_eq!(ConfigStatus::VersionMismatch.as_str(), "version_mismatch");
        assert_eq!(ConfigStatus::Error.as_str(), "error");
    }

    #[test]
    fn command_status_strings_match_the_schema_check_constraint() {
        assert_eq!(CommandStatus::Running.as_str(), "running");
        assert_eq!(CommandStatus::Success.as_str(), "success");
        assert_eq!(CommandStatus::Failed.as_str(), "failed");
        assert_eq!(CommandStatus::Expired.as_str(), "expired");
    }

    /// `desired_state` là `String` chứ không phải enum, và đó là **cố ý**: tầng deserialize giữ
    /// nguyên văn chuỗi để vòng chính tự map sang enum và từ chối giá trị lạ bằng một lỗi có
    /// tên. Nếu `AccountRow` deserialize thẳng thành enum thì serde sẽ báo lỗi generic
    /// "unknown variant" mà không nói được account nào; tệ hơn, một `#[serde(default)]` sẽ biến
    /// `"paused"` thành `"stopped"` — tức là **tắt** một account người dùng muốn chạy.
    ///
    /// Tên cũ của test này (`account_row_rejects_an_unknown_desired_state`) nói ngược với điều
    /// nó assert, nên ai đọc log fail sẽ đi tìm chỗ reject ở sai tầng.
    #[test]
    fn account_row_keeps_an_unknown_desired_state_verbatim() {
        let row = r#"{
            "id":"a","slot_index":0,"label":"acc","username":"u",
            "secret_sealed":{},"server_index":0,"desired_state":"paused",
            "control_version":13,"control":{},"config_version":1,"runtime":{}
        }"#;
        let parsed: AccountRow = serde_json::from_str(row).unwrap();
        assert_eq!(parsed.desired_state, "paused");
    }

    /// Chuỗi này được nhúng thẳng vào **query string** (`expires_at=gt.{now}`), nên nó không
    /// được chứa ký tự nào cần percent-encode. `Z` an toàn; offset `+00:00` thì không — `+`
    /// trong query decode thành dấu cách, và TTL sẽ so sánh với một chuỗi PostgREST không hiểu.
    #[test]
    fn rfc3339_is_url_safe_so_the_ttl_filter_survives() {
        let now = now_rfc3339();
        assert!(now.ends_with('Z'), "phải kết thúc bằng Z: {now}");
        assert!(
            !now.contains(['+', '%', ' ']),
            "chứa ký tự cần escape, sẽ vỡ khi nhúng vào query: {now}"
        );
        // Đúng shape cố định 20 byte: 4-2-2T2:2:2Z, mọi trường zero-pad.
        assert_eq!(now.len(), 20, "RFC3339 UTC phải dài đúng 20 ký tự: {now}");
        assert_eq!(now.chars().nth(10), Some('T'));
    }

    /// Bảng ngày lấy từ `date -u`, gồm hai chỗ mà thuật toán civil_from_days dễ sai nhất:
    /// ngày nhuận, và **quy tắc thế kỷ** — 2000 chia hết 400 nên nhuận, 2100 chia hết 100 mà
    /// không chia hết 400 nên KHÔNG nhuận. Sai chỗ này thì `expires_at > now` lệch đúng một
    /// ngày mỗi lần qua ranh giới, và biểu hiện ra là command chạy muộn hoặc không bao giờ chạy.
    #[test]
    fn rfc3339_matches_known_epoch_values_including_the_century_rule() {
        let cases: &[(u64, &str)] = &[
            (0, "1970-01-01T00:00:00Z"),
            (86_399, "1970-01-01T23:59:59Z"),
            (86_400, "1970-01-02T00:00:00Z"),
            (946_684_800, "2000-01-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),  // 2000 chia hết 400 -> nhuận
            (1_709_078_400, "2024-02-28T00:00:00Z"),
            (1_709_164_800, "2024-02-29T00:00:00Z"), // 2024 chia hết 4 -> nhuận
            (1_709_251_200, "2024-03-01T00:00:00Z"),
            (1_767_582_245, "2026-01-05T03:04:05Z"), // zero-pad tháng/ngày/giờ đơn
            (1_789_371_972, "2026-09-14T07:46:12Z"),
            (4_107_456_000, "2100-02-28T00:00:00Z"),
            (4_107_542_400, "2100-03-01T00:00:00Z"), // 2100 KHÔNG nhuận: 28/2 -> 1/3
        ];
        for (secs, expected) in cases {
            assert_eq!(
                rfc3339_from_unix(*secs),
                *expected,
                "epoch {secs} phải là {expected}"
            );
        }
        // Cùng một phát biểu dưới dạng số học: 2100 nhảy từ 28/2 thẳng sang 1/3.
        assert_eq!(
            rfc3339_from_unix(4_107_456_000 + 86_400),
            "2100-03-01T00:00:00Z"
        );
    }

    /// `devices.pubkey` đi qua base64 ở **cả hai phía**: web ghi, agent đọc. Sai alphabet là một
    /// lỗi im lặng ở tầng crypto, biểu hiện ra là "unseal ra rác".
    #[test]
    fn base64_standard_uses_the_standard_alphabet_and_pads() {
        // '+/' chứng minh đây là alphabet chuẩn, không phải URL-safe ('-_').
        assert_eq!(base64_standard(&[0xff, 0xfe, 0xfd]), "//79");
        // Padding '=' phải có, vì độ dài không chia hết 3.
        assert_eq!(base64_standard(&[0x00]), "AA==");
        assert_eq!(base64_standard(&[0x00, 0x00]), "AAA=");
        assert_eq!(base64_standard(&[]), "");
        // Và nó phải là đúng hàm mà `crypto.rs` dùng để decode — một encoder thứ hai lệch
        // alphabet là cách nhanh nhất để phá interop mà test vẫn xanh.
        let sec1 = [0x04u8; 65];
        assert_eq!(base64_standard(&sec1), crate::crypto::b64_encode(&sec1));
    }

    #[test]
    fn test_start_status_evaluation() {
        assert_eq!(evaluate_start_status(true), (CommandStatus::Success, None));
        assert_eq!(
            evaluate_start_status(false),
            (CommandStatus::Failed, Some("process failed to start"))
        );
    }

    #[test]
    fn test_stop_status_evaluation() {
        assert_eq!(evaluate_stop_status(false), (CommandStatus::Success, None));
        assert_eq!(
            evaluate_stop_status(true),
            (CommandStatus::Failed, Some("process failed to stop"))
        );
    }

    #[test]
    fn test_restart_status_evaluation() {
        assert_eq!(evaluate_restart_status(true), (CommandStatus::Success, None));
        assert_eq!(
            evaluate_restart_status(false),
            (CommandStatus::Failed, Some("replacement process failed to start"))
        );
        // Restart must NEVER report Running
        assert_ne!(evaluate_restart_status(true).0, CommandStatus::Running);
        assert_ne!(evaluate_restart_status(false).0, CommandStatus::Running);
    }

    #[test]
    fn test_apply_config_status_evaluation() {
        assert_eq!(evaluate_apply_config_status(Ok(())), (CommandStatus::Success, None));
        assert_eq!(
            evaluate_apply_config_status(Err("config version mismatch")),
            (CommandStatus::Failed, Some("config version mismatch"))
        );
        assert_eq!(
            evaluate_apply_config_status(Err("config was not applied")),
            (CommandStatus::Failed, Some("config was not applied"))
        );
    }

    #[test]
    fn test_stop_outcome_success() {
        assert!(StopOutcome::AlreadyGone.is_success());
        assert!(StopOutcome::Terminated.is_success());
        assert!(StopOutcome::Killed.is_success());
        assert!(!StopOutcome::Failed.is_success());
    }

    #[test]
    fn test_stop_transition_evaluation() {
        // Idempotent: No process handle -> ConfirmedStopped
        assert_eq!(
            evaluate_stop_transition(false, false, None),
            StopTransition::ConfirmedStopped
        );
        // Process handle existed but was already dead -> ConfirmedStopped
        assert_eq!(
            evaluate_stop_transition(true, false, None),
            StopTransition::ConfirmedStopped
        );
        // Process was alive and stopped cleanly by SIGTERM -> ConfirmedStopped
        assert_eq!(
            evaluate_stop_transition(true, true, Some(StopOutcome::Terminated)),
            StopTransition::ConfirmedStopped
        );
        // Process was alive and confirmed already gone -> ConfirmedStopped
        assert_eq!(
            evaluate_stop_transition(true, true, Some(StopOutcome::AlreadyGone)),
            StopTransition::ConfirmedStopped
        );
        // Process was alive and confirmed stopped after SIGKILL escalation -> ConfirmedStopped
        assert_eq!(
            evaluate_stop_transition(true, true, Some(StopOutcome::Killed)),
            StopTransition::ConfirmedStopped
        );
        // Process was alive but stop failed -> FailedStillAlive (do not clear handle or state)
        assert_eq!(
            evaluate_stop_transition(true, true, Some(StopOutcome::Failed)),
            StopTransition::FailedStillAlive
        );
        // Process was alive but no outcome returned -> FailedStillAlive
        assert_eq!(
            evaluate_stop_transition(true, true, None),
            StopTransition::FailedStillAlive
        );
    }

    #[test]
    fn test_restart_precondition() {
        // Old process confirmed dead/stopped -> proceed with replacement spawn
        assert_eq!(
            check_restart_precondition(false),
            RestartPrecondition::Proceed
        );
        // Old process still alive -> block replacement spawn
        assert_eq!(
            check_restart_precondition(true),
            RestartPrecondition::BlockedOldProcessAlive
        );
    }
}

