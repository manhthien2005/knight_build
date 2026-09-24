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
    /// Tạo payload PATCH cho devices lúc boot, công bố contract và version.
    pub fn build_device_patch_payload(manifest: &JarManifest) -> serde_json::Value {
        serde_json::json!({
            "jar_sha256": manifest.jar_sha256,
            "jar_ctl_version": manifest.ctl_version,
            "jar_snapshot_version": manifest.snapshot_version,
            "jar_ctl_key_count": manifest.ctl_key_count,
            "agent_version": manifest.advertised_agent_version(),
            "status": "online",
        })
    }

    /// Công bố hợp đồng jar lúc boot. Web đọc hai cột version này để quyết định render form
    /// nào; agent đọc chúng để quyết định có được ghi control file hay không.
    ///
    /// Nguồn là `/opt/knight/game/zeus-jar.json` — **không parse bytecode**. Build script đã
    /// biết các con số; đọc lại từ class file là tự tạo thêm một nguồn có thể sai.
    pub fn announce_jar_contract(&self, device_id: &str, manifest: &JarManifest) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/devices?id=eq.{device_id}"))
            .prefer("return=minimal")
            .send_json(Self::build_device_patch_payload(manifest))?;
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

    /// Cập nhật command sang trạng thái `running` khi bắt đầu xử lý lệnh kéo dài (như detect-spots).
    pub fn mark_command_running(&self, command_id: &str) -> Result<(), RestError> {
        self.request("PATCH", &format!("/rest/v1/commands?id=eq.{command_id}"))
            .prefer("return=minimal")
            .send_json(serde_json::json!({
                "status": CommandStatus::Running.as_str(),
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

    /// Lấy danh sách các lệnh detect-spots đang ở trạng thái `running` thuộc về device này.
    /// Dùng lúc boot agent để recover các lệnh bị bỏ dở do crash/restart — R5A.
    pub fn fetch_running_detect_spots_commands(
        &self,
        device_id: &str,
    ) -> Result<Vec<CommandRow>, RestError> {
        let response = self
            .request(
                "GET",
                &format!(
                    "/rest/v1/commands?device_id=eq.{device_id}&status=eq.running&type=eq.detect-spots&order=created_at.asc"
                ),
            )
            .call()?;
        let rows: Vec<CommandRow> = response.json()?;
        Ok(filter_abandoned_detect_spots_commands(rows, device_id))
    }

    /// Thu dọn các lệnh detect-spots bị bỏ dở do agent restart.
    /// CHỈ gọi một lần duy nhất lúc khởi động process, KHÔNG gọi khi realtime reconnect.
    pub fn recover_abandoned_spot_scans(
        &self,
        device_id: &str,
    ) -> Result<u32, RestError> {
        let commands = self.fetch_running_detect_spots_commands(device_id)?;
        if commands.is_empty() {
            return Ok(0);
        }

        let mut first_error: Option<RestError> = None;
        let mut recovered_count = 0;
        for cmd in &commands {
            eprintln!(
                "[recovery] recovering abandoned running detect-spots command id={} for device={}",
                cmd.id, device_id
            );
            match self.finish_command(
                &cmd.id,
                CommandStatus::Failed,
                Some(ABANDONED_SPOT_SCAN_MESSAGE),
            ) {
                Ok(()) => {
                    recovered_count += 1;
                }
                Err(e) => {
                    eprintln!(
                        "[recovery] failed to mark abandoned detect-spots command id={} as failed: {e}",
                        cmd.id
                    );
                    if first_error.is_none() {
                        first_error = Some(e);
                    }
                }
            }
        }

        if let Some(err) = first_error {
            Err(err)
        } else {
            Ok(recovered_count)
        }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RuntimeContract {
    pub name: &'static str,
    pub jar_sha256: &'static str,
    pub ctl_version: u32,
    pub capabilities: &'static [&'static str],
}

pub const KNOWN_RUNTIME_CONTRACTS: &[RuntimeContract] = &[
    RuntimeContract {
        name: "CHARACTER_SLOT_ONLY_V13",
        jar_sha256: JarManifest::CHARACTER_SLOT_COMPATIBLE_JAR_SHA256,
        ctl_version: 13,
        capabilities: &[JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN],
    },
    RuntimeContract {
        name: "VISUAL_QOL_V14",
        jar_sha256: JarManifest::VISUAL_QOL_COMPATIBLE_JAR_SHA256,
        ctl_version: 14,
        capabilities: &[
            JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN,
            JarManifest::VISUAL_QOL_CAPABILITY_TOKEN,
        ],
    },
    RuntimeContract {
        name: "INVENTORY_CATALOG_V14",
        jar_sha256: JarManifest::INVENTORY_CATALOG_COMPATIBLE_JAR_SHA256,
        ctl_version: 14,
        capabilities: &[
            JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN,
            JarManifest::VISUAL_QOL_CAPABILITY_TOKEN,
        ],
    },
    RuntimeContract {
        name: "ENHANCEMENT_ENGINE_V14",
        jar_sha256: JarManifest::ENHANCEMENT_ENGINE_COMPATIBLE_JAR_SHA256,
        ctl_version: 14,
        capabilities: &[
            JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN,
            JarManifest::VISUAL_QOL_CAPABILITY_TOKEN,
        ],
    },
];

impl JarManifest {
    pub const CHARACTER_SLOT_CAPABILITY_TOKEN: &'static str = "character-slot-v1";
    pub const VISUAL_QOL_CAPABILITY_TOKEN: &'static str = "visual-qol-v1";

    pub const CHARACTER_SLOT_COMPATIBLE_JAR_SHA256: &'static str =
        "0bcd6917d8d87faf9fe78fa938abfe5cdf16c0153fcc876deb337d022bb036fd";
    pub const VISUAL_QOL_COMPATIBLE_JAR_SHA256: &'static str =
        "0298b431804ffe33c481a662e4be64cfa2a1409d247a54a7714038be6db91fbd";
    pub const INVENTORY_CATALOG_COMPATIBLE_JAR_SHA256: &'static str =
        "f06e4fa973c882c1be359f8fa6db609eba78b33d8fe134874ab7b11488ee5762";
    pub const ENHANCEMENT_ENGINE_COMPATIBLE_JAR_SHA256: &'static str =
        "01bbf0575badcbf5e47231c78ce6a5d848f7110650328fce7e6252b196473478";

    pub fn read_from_file(path: &str) -> Option<Self> {
        let data = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&data).ok()
    }

    pub fn capabilities(&self) -> &'static [&'static str] {
        if self.snapshot_version < 6 {
            return &[];
        }
        for contract in KNOWN_RUNTIME_CONTRACTS {
            if self.jar_sha256 == contract.jar_sha256 && self.ctl_version == contract.ctl_version {
                return contract.capabilities;
            }
        }
        &[]
    }

    pub fn is_character_slot_compatible(&self) -> bool {
        self.capabilities().contains(&Self::CHARACTER_SLOT_CAPABILITY_TOKEN)
    }

    pub fn is_visual_qol_compatible(&self) -> bool {
        self.capabilities().contains(&Self::VISUAL_QOL_CAPABILITY_TOKEN)
    }

    pub fn canonical_agent_version(&self) -> &str {
        let trimmed = self.agent_version.trim();
        if trimmed.is_empty() {
            env!("CARGO_PKG_VERSION")
        } else {
            trimmed
        }
    }

    pub fn advertised_agent_version(&self) -> String {
        let base = self.canonical_agent_version();
        let (ver, meta) = match base.split_once('+') {
            Some((v, m)) => (v, Some(m)),
            None => (base, None),
        };

        let caps = self.capabilities();
        let mut tokens: Vec<&str> = Vec::new();

        if let Some(m) = meta {
            for token in m.split('.') {
                if token == Self::CHARACTER_SLOT_CAPABILITY_TOKEN
                    || token == Self::VISUAL_QOL_CAPABILITY_TOKEN
                {
                    continue;
                }
                if !token.is_empty() && !tokens.contains(&token) {
                    tokens.push(token);
                }
            }
        }

        if caps.contains(&Self::CHARACTER_SLOT_CAPABILITY_TOKEN)
            && !tokens.contains(&Self::CHARACTER_SLOT_CAPABILITY_TOKEN)
        {
            tokens.push(Self::CHARACTER_SLOT_CAPABILITY_TOKEN);
        }
        if caps.contains(&Self::VISUAL_QOL_CAPABILITY_TOKEN)
            && !tokens.contains(&Self::VISUAL_QOL_CAPABILITY_TOKEN)
        {
            tokens.push(Self::VISUAL_QOL_CAPABILITY_TOKEN);
        }

        if tokens.is_empty() {
            ver.to_string()
        } else {
            format!("{ver}+{}", tokens.join("."))
        }
    }
}

pub fn read_jar_manifest(path: &str) -> Option<JarManifest> {
    JarManifest::read_from_file(path)
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

fn default_character_slot() -> i16 {
    1
}

pub fn validate_character_slot(slot: i16) -> Result<i16, &'static str> {
    if (1..=3).contains(&slot) {
        Ok(slot)
    } else {
        Err("character_slot must be 1, 2, or 3")
    }
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
    #[serde(default = "default_character_slot")]
    pub character_slot: i16,
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

/// Message được ghi nhận khi phát hiện lệnh detect-spots bị bỏ dở do agent restart — R5A.
pub const ABANDONED_SPOT_SCAN_MESSAGE: &str =
    "Detect Spots scan abandoned because zeus-agent restarted before completion.";

/// Quyết định xử lý cho một lệnh khi kiểm tra abandoned detect-spots lúc khởi động process — R5A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbandonedRecoveryDecision {
    /// Lệnh thuộc về node này, đang running, và là detect-spots: thu dọn và đánh dấu failed.
    RecoverAsFailed,
    /// Lệnh thuộc về node khác: không được can thiệp.
    IgnoreOtherDevice,
    /// Lệnh không phải detect-spots (start, stop, restart, apply-config...): giữ nguyên.
    IgnoreOtherCommandType,
    /// Lệnh không ở trạng thái running (queued, success, failed, expired): bỏ qua.
    IgnoreNonRunningStatus,
}

/// Đánh giá xem một lệnh có phải là detect-spots scan bị bỏ dở cần thu dọn lúc boot hay không.
pub fn evaluate_abandoned_spot_scan(
    command_device_id: Option<&str>,
    current_device_id: &str,
    command_kind: &str,
    command_status: &str,
) -> AbandonedRecoveryDecision {
    if command_device_id != Some(current_device_id) {
        return AbandonedRecoveryDecision::IgnoreOtherDevice;
    }
    if command_kind != "detect-spots" {
        return AbandonedRecoveryDecision::IgnoreOtherCommandType;
    }
    if command_status != CommandStatus::Running.as_str() {
        return AbandonedRecoveryDecision::IgnoreNonRunningStatus;
    }
    AbandonedRecoveryDecision::RecoverAsFailed
}

/// Lọc danh sách CommandRow nhận được để chỉ giữ lại các lệnh detect-spots của device này đang running.
pub fn filter_abandoned_detect_spots_commands(
    commands: Vec<CommandRow>,
    device_id: &str,
) -> Vec<CommandRow> {
    commands
        .into_iter()
        .filter(|cmd| {
            cmd.device_id.as_deref() == Some(device_id) && cmd.kind == "detect-spots"
        })
        .collect()
}

/// Hành động đối với spot scan khi realtime reconnect hoặc boot process — R5A.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectSpotScanAction {
    /// Tiến trình mới boot: chạy startup recovery cho các lệnh running bị bỏ dở.
    RunStartupAbandonedRecovery,
    /// Trong cùng tiến trình, realtime reconnect nhưng đang có pending_spot_scan: giữ nguyên scan.
    PreservePendingScan,
    /// Trong cùng tiến trình, realtime reconnect và không có pending scan: không cần làm gì.
    NoAction,
}

/// Đánh giá hành vi với detect-spots khi kết nối lại realtime hoặc boot process.
pub fn evaluate_spot_scan_reconnect_action(
    has_pending_scan: bool,
    is_startup: bool,
) -> ReconnectSpotScanAction {
    if is_startup {
        ReconnectSpotScanAction::RunStartupAbandonedRecovery
    } else if has_pending_scan {
        ReconnectSpotScanAction::PreservePendingScan
    } else {
        ReconnectSpotScanAction::NoAction
    }
}

/// Kết quả của việc thực hiện thu dọn abandoned spot scan — R5A.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AbandonedRecoveryOutcome {
    QueryFailed,
    NoneFound,
    Recovered(u32),
}

/// Đánh giá kết quả thu dọn abandoned spot scan từ kết quả truy vấn commands.
pub fn evaluate_abandoned_recovery_outcome<T, E>(
    query_result: Result<Vec<T>, E>,
) -> AbandonedRecoveryOutcome {
    match query_result {
        Err(_) => AbandonedRecoveryOutcome::QueryFailed,
        Ok(ref list) if list.is_empty() => AbandonedRecoveryOutcome::NoneFound,
        Ok(list) => AbandonedRecoveryOutcome::Recovered(list.len() as u32),
    }
}

/// Hành vi quyết định của startup recovery barrier đối với detect-spots — R5A.1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartupBarrierAction {
    /// Barrier resolved: startup có thể tiếp tục tiến trình spawn realtime và nhận lệnh.
    ProceedToRealtime { recovered_count: u32 },
    /// Barrier unresolved: cần retry với backoff trước khi start realtime.
    Retry { attempt: u32, max_retries: u32 },
    /// Barrier thất bại sau khi hết số lần retry: dừng startup để container supervisor restart.
    FatalAbort { attempts_exhausted: u32 },
}

/// Đánh giá kết quả một attempt vượt qua startup recovery barrier.
pub fn evaluate_startup_barrier_step<E>(
    attempt_result: &Result<u32, E>,
    attempt: u32,
    max_retries: u32,
) -> StartupBarrierAction {
    match attempt_result {
        Ok(count) => StartupBarrierAction::ProceedToRealtime {
            recovered_count: *count,
        },
        Err(_) if attempt < max_retries => StartupBarrierAction::Retry {
            attempt,
            max_retries,
        },
        Err(_) => StartupBarrierAction::FatalAbort {
            attempts_exhausted: attempt,
        },
    }
}

/// Mô phỏng việc hoàn tất một batch lệnh abandoned detect-spots (cho unit tests).
pub fn simulate_abandoned_batch_finalization(
    commands: &[CommandRow],
    failing_command_id: Option<&str>,
) -> Result<u32, &'static str> {
    let mut first_error = None;
    let mut count = 0;
    for cmd in commands {
        if Some(cmd.id.as_str()) == failing_command_id {
            if first_error.is_none() {
                first_error = Some("simulated finish_command error");
            }
        } else {
            count += 1;
        }
    }
    if let Some(err) = first_error {
        Err(err)
    } else {
        Ok(count)
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

/// Parses (state, pgrp) from the text of a Linux `/proc/<pid>/stat` line.
///
/// Handles `comm` fields with spaces and nested parentheses by splitting from the rightmost `)`.
/// Field 3 is state, Field 4 is ppid, Field 5 is pgrp (process group ID).
pub fn parse_procfs_stat_pgrp_and_state(stat: &str) -> Option<(u8, i32)> {
    let (_, after_comm) = stat.rsplit_once(')')?;
    let mut tokens = after_comm.trim().split_whitespace();
    let state = tokens.next()?.as_bytes().first().copied()?;
    let _ppid = tokens.next()?;
    let pgrp = tokens.next()?.parse::<i32>().ok()?;
    Some((state, pgrp))
}

/// Result of inspecting an individual `/proc/<pid>` entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcEntryInspection {
    /// Successfully read and parsed stat
    Parsed { state: u8, pgrp: i32 },
    /// Process vanished during scan (NotFound / ESRCH) — normal race
    Vanished,
    /// Unreadable or unparseable, but verified to belong to an unrelated process group
    UnrelatedGroup(i32),
    /// Unreadable or unparseable, and belongs to or cannot be ruled out from target group
    InspectionFailed,
}

/// Decision for an individual `/proc/<pid>` entry during group liveness scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupLivenessDecision {
    /// Found confirmed live member of target group
    Alive,
    /// Inspection failed in a way that prevents declaring the group dead
    Indeterminate,
    /// No live workload found in this entry, continue scanning
    ContinueScan,
}

/// Evaluates an inspected process entry against the target process group.
pub fn evaluate_proc_entry(
    target_pgid: i32,
    inspection: &ProcEntryInspection,
) -> GroupLivenessDecision {
    match inspection {
        ProcEntryInspection::Parsed { state, pgrp } => {
            if *pgrp == target_pgid {
                if *state != b'Z' {
                    GroupLivenessDecision::Alive
                } else {
                    // Zombie in target group: not live workload
                    GroupLivenessDecision::ContinueScan
                }
            } else {
                GroupLivenessDecision::ContinueScan
            }
        }
        ProcEntryInspection::Vanished => GroupLivenessDecision::ContinueScan,
        ProcEntryInspection::UnrelatedGroup(_) => GroupLivenessDecision::ContinueScan,
        ProcEntryInspection::InspectionFailed => GroupLivenessDecision::Indeterminate,
    }
}

/// Outcome of a process-group liveness scan across `/proc`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupScanOutcome {
    /// Confirmed dead: no live workload remaining in target PGID
    ConfirmedDead,
    /// Confirmed alive: live non-zombie process in target PGID
    LiveWorkloadPresent,
    /// Uncertain / inspection failure: cannot prove dead
    Uncertain,
}

impl GroupScanOutcome {
    /// Fail-safe mapping: returns true if alive or uncertain; false ONLY if confirmed dead.
    pub fn is_alive_or_uncertain(&self) -> bool {
        matches!(self, Self::LiveWorkloadPresent | Self::Uncertain)
    }
}

/// Evaluates an entire collection of procfs inspections for a target process group.
/// Returns ConfirmedDead ONLY when there is enough evidence that no live non-zombie
/// member of target_pgid remains.
pub fn evaluate_group_liveness_scan<I>(
    target_pgid: i32,
    procfs_available: bool,
    entries: I,
) -> GroupScanOutcome
where
    I: IntoIterator<Item = ProcEntryInspection>,
{
    if !procfs_available {
        return GroupScanOutcome::Uncertain;
    }

    let mut uncertain = false;
    for entry in entries {
        match evaluate_proc_entry(target_pgid, &entry) {
            GroupLivenessDecision::Alive => return GroupScanOutcome::LiveWorkloadPresent,
            GroupLivenessDecision::Indeterminate => {
                uncertain = true;
            }
            GroupLivenessDecision::ContinueScan => {}
        }
    }

    if uncertain {
        GroupScanOutcome::Uncertain
    } else {
        GroupScanOutcome::ConfirmedDead
    }
}

/// In-memory bounded FIFO deduplication cache for command IDs.
///
/// Prevents duplicate execution when a command appears in both REST drain
/// and realtime subscription events during reconnect/boot recovery windows.
#[derive(Debug, Clone)]
pub struct CommandDedupe {
    capacity: usize,
    order: std::collections::VecDeque<String>,
    seen: std::collections::HashSet<String>,
}

impl CommandDedupe {
    pub fn new(capacity: usize) -> Self {
        let cap = capacity.max(1);
        Self {
            capacity: cap,
            order: std::collections::VecDeque::with_capacity(cap),
            seen: std::collections::HashSet::with_capacity(cap),
        }
    }

    /// Number of tracked command IDs.
    pub fn len(&self) -> usize {
        self.seen.len()
    }

    /// Whether the dedupe cache is empty.
    pub fn is_empty(&self) -> bool {
        self.seen.is_empty()
    }

    /// Checks if a command ID is currently in the dedupe set.
    pub fn contains(&self, id: &str) -> bool {
        self.seen.contains(id)
    }

    /// Records a command ID. If already seen, returns false.
    /// If newly added, returns true and evicts the oldest entry when capacity is exceeded.
    pub fn record_if_new(&mut self, id: &str) -> bool {
        if self.seen.contains(id) {
            return false;
        }
        if self.order.len() >= self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.seen.remove(&oldest);
            }
        }
        self.order.push_back(id.to_string());
        self.seen.insert(id.to_string());
        true
    }
}

/// Evaluates if a command's TTL has elapsed against a reference RFC3339 timestamp.
pub fn is_command_expired(expires_at: &str, now: &str) -> bool {
    expires_at < now
}

/// Evaluates whether all required realtime table subscriptions are established.
pub fn evaluate_subscriptions(accounts_ok: bool, commands_ok: bool) -> bool {
    accounts_ok && commands_ok
}

/// Pure representation of account merge for verifying cross-platform update invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountSnapshot {
    pub id: String,
    pub slot_index: i32,
    pub username: String,
    pub server_index: u8,
    pub secret_sealed: serde_json::Value,
    pub desired_state: String,
    pub control_version: i32,
    pub control: serde_json::Value,
    pub config_version: i32,
    /// Runtime-owned field (e.g. process PID) that must NEVER be overwritten by cloud refresh
    pub live_process_pid: Option<i32>,
    /// Flag indicating this account was removed in cloud and is undergoing safe process stop/cleanup
    pub retiring: bool,
}

impl AccountSnapshot {
    /// Merges fresh cloud fields into self, strictly preserving local runtime state.
    /// A retiring account is monotonic and must never be updated or resurrected.
    pub fn merge_cloud_fields(&mut self, fresh: &AccountRow) {
        if self.retiring {
            return;
        }
        self.username = fresh.username.clone();
        self.server_index = (fresh.server_index as i32).clamp(0, 7) as u8;
        self.secret_sealed = fresh.secret_sealed.clone();
        self.desired_state = fresh.desired_state.clone();
        self.control_version = fresh.control_version;
        self.control = fresh.control.clone();
        self.config_version = fresh.config_version;
    }

    pub fn mark_retiring(&mut self) {
        self.retiring = true;
        self.desired_state = "stopped".to_string();
    }

    pub fn can_autostart(&self) -> bool {
        !self.retiring && self.desired_state == "running"
    }

    pub fn can_accept_command(&self) -> bool {
        !self.retiring
    }

    pub fn from_row(row: &AccountRow) -> Self {
        Self {
            id: row.id.clone(),
            slot_index: row.slot_index,
            username: row.username.clone(),
            server_index: (row.server_index as i32).clamp(0, 7) as u8,
            secret_sealed: row.secret_sealed.clone(),
            desired_state: row.desired_state.clone(),
            control_version: row.control_version,
            control: row.control.clone(),
            config_version: row.config_version,
            live_process_pid: None,
            retiring: false,
        }
    }
}

/// Action to take on account retirement given its process state and stop outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetirementAction {
    /// Process confirmed terminated or was never alive: safe to clear credentials and remove state.
    RemoveCleaned,
    /// Process failed to terminate or liveness is uncertain: MUST retain process tracking and local state.
    RetainPendingCleanup,
}

/// Evaluates the safe retirement action based on process existence, liveness, and stop outcome.
pub fn evaluate_retirement_action(
    has_process: bool,
    is_alive: bool,
    stop_outcome: Option<StopOutcome>,
) -> RetirementAction {
    match evaluate_stop_transition(has_process, is_alive, stop_outcome) {
        StopTransition::ConfirmedStopped => RetirementAction::RemoveCleaned,
        StopTransition::FailedStillAlive => RetirementAction::RetainPendingCleanup,
    }
}

/// Outcome of reconciling local accounts against an authoritative cloud accounts fetch result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReconciliationPlan {
    /// Cloud fetch failed: local accounts MUST NOT be altered, stopped, or removed.
    AbortedFetchFailed,
    /// Cloud fetch succeeded (even if 0 accounts): cloud is authoritative.
    Apply {
        to_insert: Vec<String>,
        to_update: Vec<String>,
        to_retire: Vec<String>,
    },
}

/// Decision whether an account may be admitted for local tracking, update, or creation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAdmissionDecision {
    Admitted,
    RejectedTombstoned,
}

/// Evaluates whether an account is admitted for creation, update, or reconciliation.
///
/// Ensures the tombstone invariant: once an account is deleted or retired, its tombstone
/// survives `AccountState` removal and permanently rejects any later creation, update,
/// or resurrection attempt.
pub fn evaluate_account_admission(
    _account_id: &str,
    is_tombstoned: bool,
) -> AccountAdmissionDecision {
    if is_tombstoned {
        AccountAdmissionDecision::RejectedTombstoned
    } else {
        AccountAdmissionDecision::Admitted
    }
}

/// Pure evaluator for account set reconciliation.
///
/// Ensures Invariant A (fetch error is not data) and Invariant B (successful empty set is authoritative).
/// Ensures Problem A invariant: retiring accounts remain retirement-monotonic, are excluded from
/// `to_update`, and are routed through `to_retire` for ongoing safe process group termination retry.
/// Ensures Tombstone invariant: tombstoned account IDs are permanently excluded from `to_insert` and `to_update`.
pub fn evaluate_account_reconciliation<'a, I, E, T>(
    local_accounts: I,
    cloud_result: Result<&[AccountRow], E>,
    tombstoned_ids: T,
) -> ReconciliationPlan
where
    I: IntoIterator<Item = (&'a str, bool)>,
    T: IntoIterator<Item = &'a str>,
{
    match cloud_result {
        Err(_) => ReconciliationPlan::AbortedFetchFailed,
        Ok(cloud_rows) => {
            let cloud_ids: std::collections::HashSet<&str> =
                cloud_rows.iter().map(|r| r.id.as_str()).collect();
            let local_map: std::collections::HashMap<&str, bool> = local_accounts.into_iter().collect();
            let tombstone_set: std::collections::HashSet<&str> = tombstoned_ids.into_iter().collect();

            let mut to_insert = Vec::new();
            let mut to_update = Vec::new();
            for row in cloud_rows {
                if evaluate_account_admission(row.id.as_str(), tombstone_set.contains(row.id.as_str()))
                    == AccountAdmissionDecision::RejectedTombstoned
                {
                    // Account is tombstoned. Under no circumstances may it be updated or inserted.
                    continue;
                }

                match local_map.get(row.id.as_str()) {
                    Some(&true) => {
                        // Account is locally retiring. Under no circumstances may it be updated,
                        // re-spawned, or resurrected into normal execution.
                    }
                    Some(&false) => {
                        to_update.push(row.id.clone());
                    }
                    None => {
                        to_insert.push(row.id.clone());
                    }
                }
            }

            let mut to_retire = Vec::new();
            for (&id, &is_retiring) in &local_map {
                // If the account is missing from cloud OR is already marked retiring OR is tombstoned,
                // it belongs in to_retire (safe retirement / retry).
                if is_retiring || tombstone_set.contains(id) || !cloud_ids.contains(id) {
                    to_retire.push(id.to_string());
                }
            }
            to_retire.sort();
            to_update.sort();
            to_insert.sort();

            ReconciliationPlan::Apply {
                to_insert,
                to_update,
                to_retire,
            }
        }
    }
}

/// Decision outcome for token refresh during reconnect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconnectTokenDecision {
    pub rest_token: String,
    pub realtime_token: String,
    pub refreshed: bool,
}

/// Evaluates token propagation for realtime reconnect.
///
/// Ensures Problem B invariant:
/// - On refresh success: new token is propagated to both REST and realtime reconnect.
/// - On refresh failure: known-good current token is preserved without being overwritten.
pub fn evaluate_reconnect_token_refresh<E>(
    current_token: &str,
    sign_in_result: Result<String, E>,
) -> ReconnectTokenDecision {
    match sign_in_result {
        Ok(new_token) => ReconnectTokenDecision {
            rest_token: new_token.clone(),
            realtime_token: new_token,
            refreshed: true,
        },
        Err(_) => ReconnectTokenDecision {
            rest_token: current_token.to_string(),
            realtime_token: current_token.to_string(),
            refreshed: false,
        },
    }
}

/// Evaluates recovery action for drained commands given a refresh result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Post-drain refresh succeeded; dispatch commands in order with updated accounts
    ProceedWithDispatch,
    /// Post-drain refresh failed; abort recovery to avoid executing against stale state
    AbortDueToRefreshFailure,
}

pub fn evaluate_recovery_precondition<E: ?Sized>(refresh_result: Result<&[AccountRow], &E>) -> RecoveryAction {
    match refresh_result {
        Ok(_) => RecoveryAction::ProceedWithDispatch,
        Err(_) => RecoveryAction::AbortDueToRefreshFailure,
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

    #[test]
    fn test_parse_procfs_stat_pgrp_and_state() {
        // Standard stat line
        let stat1 = "1234 (java) S 1 1000 1000 0 -1 4194304";
        assert_eq!(parse_procfs_stat_pgrp_and_state(stat1), Some((b'S', 1000)));

        // comm containing spaces and nested parentheses
        let stat2 = "5678 (Web Content (worker)) R 1234 5000 5000 0 -1 4194304";
        assert_eq!(parse_procfs_stat_pgrp_and_state(stat2), Some((b'R', 5000)));

        // Zombie process
        let stat3 = "9999 (defunct_worker) Z 1234 1000 1000 0 -1 4194304";
        assert_eq!(parse_procfs_stat_pgrp_and_state(stat3), Some((b'Z', 1000)));

        // Malformed line
        assert_eq!(parse_procfs_stat_pgrp_and_state("invalid content"), None);
        assert_eq!(parse_procfs_stat_pgrp_and_state("1234 ()"), None);
    }

    #[test]
    fn test_group_liveness_decision_semantics() {
        let target_pgid = 1000;

        // Case A: Confirmed dead (e.g. ESRCH fast path or empty procfs scan)
        assert_eq!(
            evaluate_group_liveness_scan(target_pgid, true, vec![]),
            GroupScanOutcome::ConfirmedDead
        );
        assert!(!evaluate_group_liveness_scan(target_pgid, true, vec![]).is_alive_or_uncertain());

        // Case B: Target live non-zombie process found -> Alive
        assert_eq!(
            evaluate_group_liveness_scan(
                target_pgid,
                true,
                vec![ProcEntryInspection::Parsed {
                    state: b'S',
                    pgrp: target_pgid,
                }]
            ),
            GroupScanOutcome::LiveWorkloadPresent
        );
        assert!(evaluate_group_liveness_scan(
            target_pgid,
            true,
            vec![ProcEntryInspection::Parsed {
                state: b'S',
                pgrp: target_pgid,
            }]
        ).is_alive_or_uncertain());

        // Case C: Only target zombies found -> ConfirmedDead (no live workload)
        assert_eq!(
            evaluate_group_liveness_scan(
                target_pgid,
                true,
                vec![ProcEntryInspection::Parsed {
                    state: b'Z',
                    pgrp: target_pgid,
                }]
            ),
            GroupScanOutcome::ConfirmedDead
        );
        assert!(!evaluate_group_liveness_scan(
            target_pgid,
            true,
            vec![ProcEntryInspection::Parsed {
                state: b'Z',
                pgrp: target_pgid,
            }]
        ).is_alive_or_uncertain());

        // Case D: Global procfs inspection unavailable while group not disproven -> Uncertain (must NOT declare dead)
        assert_eq!(
            evaluate_group_liveness_scan(target_pgid, false, vec![]),
            GroupScanOutcome::Uncertain
        );
        assert!(evaluate_group_liveness_scan(target_pgid, false, vec![]).is_alive_or_uncertain());

        // Case E: A PID disappears during scan (Vanished) -> Normal race, scan continues, confirmed dead if no live workload
        assert_eq!(
            evaluate_group_liveness_scan(
                target_pgid,
                true,
                vec![
                    ProcEntryInspection::Vanished,
                    ProcEntryInspection::Parsed {
                        state: b'Z',
                        pgrp: target_pgid,
                    }
                ]
            ),
            GroupScanOutcome::ConfirmedDead
        );

        // Case F: Non-race inspection error on target or undetermined PID -> Uncertain (must NOT declare dead)
        assert_eq!(
            evaluate_group_liveness_scan(
                target_pgid,
                true,
                vec![
                    ProcEntryInspection::Vanished,
                    ProcEntryInspection::InspectionFailed,
                ]
            ),
            GroupScanOutcome::Uncertain
        );
        assert!(evaluate_group_liveness_scan(
            target_pgid,
            true,
            vec![ProcEntryInspection::InspectionFailed]
        ).is_alive_or_uncertain());

        // Unrelated group inspection failure is safely ignored
        assert_eq!(
            evaluate_group_liveness_scan(
                target_pgid,
                true,
                vec![
                    ProcEntryInspection::UnrelatedGroup(2000),
                    ProcEntryInspection::Vanished,
                ]
            ),
            GroupScanOutcome::ConfirmedDead
        );
    }

    #[test]
    fn test_command_recovery_and_dedupe_semantics() {
        // Case A: Recovered command list preserves source order
        let cmd1 = CommandRow {
            id: "cmd-1".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "start".to_string(),
            payload: None,
            expires_at: "2026-09-14T07:50:00Z".to_string(),
        };
        let cmd2 = CommandRow {
            id: "cmd-2".to_string(),
            account_id: Some("acc-2".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "stop".to_string(),
            payload: None,
            expires_at: "2026-09-14T07:55:00Z".to_string(),
        };
        let cmd3 = CommandRow {
            id: "cmd-3".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "restart".to_string(),
            payload: None,
            expires_at: "2026-09-14T08:00:00Z".to_string(),
        };
        let drained = vec![cmd1.clone(), cmd2.clone(), cmd3.clone()];
        let order_processed: Vec<String> = drained.iter().map(|c| c.id.clone()).collect();
        assert_eq!(order_processed, vec!["cmd-1", "cmd-2", "cmd-3"]);

        // Case B: Recovered command ID followed by matching realtime event executes once
        let mut dedupe = CommandDedupe::new(3);
        assert!(dedupe.record_if_new(&cmd1.id), "drained command must be recorded");
        assert!(
            !dedupe.record_if_new(&cmd1.id),
            "matching realtime duplicate must be suppressed"
        );

        // Case C: Unrelated normal realtime command still executes
        assert!(
            dedupe.record_if_new(&cmd2.id),
            "unrelated realtime command must execute"
        );

        // Case D: Dedupe cache is bounded / evicts oldest entry
        assert!(dedupe.record_if_new(&cmd3.id));
        assert_eq!(dedupe.len(), 3);
        assert!(dedupe.contains("cmd-1"));
        assert!(dedupe.contains("cmd-2"));
        assert!(dedupe.contains("cmd-3"));

        // Inserting 4th command evicts "cmd-1"
        let cmd4_id = "cmd-4";
        assert!(dedupe.record_if_new(cmd4_id));
        assert_eq!(dedupe.len(), 3);
        assert!(!dedupe.contains("cmd-1"), "oldest entry must be evicted");
        assert!(dedupe.contains("cmd-2"));
        assert!(dedupe.contains("cmd-3"));
        assert!(dedupe.contains("cmd-4"));

        // Case E: Expired command does not become valid merely because it was recovered
        let now_str = "2026-09-14T07:46:12Z";
        let expired_cmd = CommandRow {
            id: "cmd-expired".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "start".to_string(),
            payload: None,
            expires_at: "2026-09-14T07:46:10Z".to_string(),
        };
        let valid_cmd = CommandRow {
            id: "cmd-valid".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "start".to_string(),
            payload: None,
            expires_at: "2026-09-14T07:46:15Z".to_string(),
        };
        assert!(is_command_expired(&expired_cmd.expires_at, now_str));
        assert!(!is_command_expired(&valid_cmd.expires_at, now_str));

        // Case F: Realtime ready is emitted only after successful subscriptions
        assert!(!evaluate_subscriptions(false, false));
        assert!(!evaluate_subscriptions(true, false));
        assert!(!evaluate_subscriptions(false, true));
        assert!(evaluate_subscriptions(true, true));
    }

    #[test]
    fn test_account_state_barrier_and_merge_semantics() {
        use std::collections::HashMap;

        // Case A: new account before recovered Start
        let mut in_memory_accounts: HashMap<String, AccountSnapshot> = HashMap::new();
        let row_a = AccountRow {
            id: "acc-new".to_string(),
            slot_index: 0,
            label: "Acc New".to_string(),
            username: "newbie".to_string(),
            secret_sealed: serde_json::json!({"sealed": "secret"}),
            server_index: 1,
            desired_state: "stopped".to_string(),
            control_version: 1,
            control: serde_json::json!({"opt": 1}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        assert!(!in_memory_accounts.contains_key(&row_a.id));

        // Post-drain refresh contains row_a
        in_memory_accounts.insert(row_a.id.clone(), AccountSnapshot::from_row(&row_a));
        assert!(in_memory_accounts.contains_key("acc-new"));
        let acc_a = in_memory_accounts.get("acc-new").unwrap();
        assert_eq!(acc_a.username, "newbie");

        // Case B: credential update before recovered Restart
        let mut acc_b = AccountSnapshot {
            id: "acc-b".to_string(),
            slot_index: 1,
            username: "old_user".to_string(),
            server_index: 0,
            secret_sealed: serde_json::json!({"version": 1}),
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            live_process_pid: Some(4001),
            retiring: false,
        };
        let row_b_updated = AccountRow {
            id: "acc-b".to_string(),
            slot_index: 1,
            label: "Acc B".to_string(),
            username: "updated_user".to_string(),
            secret_sealed: serde_json::json!({"version": 2}),
            server_index: 3,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        acc_b.merge_cloud_fields(&row_b_updated);
        assert_eq!(acc_b.username, "updated_user");
        assert_eq!(acc_b.server_index, 3);
        assert_eq!(acc_b.secret_sealed, serde_json::json!({"version": 2}));

        // Case C: config update before recovered apply-config
        let row_c_updated = AccountRow {
            id: "acc-b".to_string(),
            slot_index: 1,
            label: "Acc B".to_string(),
            username: "updated_user".to_string(),
            secret_sealed: serde_json::json!({"version": 2}),
            server_index: 3,
            desired_state: "running".to_string(),
            control_version: 5,
            control: serde_json::json!({"speed": 10}),
            config_version: 4,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        acc_b.merge_cloud_fields(&row_c_updated);
        assert_eq!(acc_b.control_version, 5);
        assert_eq!(acc_b.control, serde_json::json!({"speed": 10}));
        assert_eq!(acc_b.config_version, 4);

        // Case D: refresh failure aborts recovery without dispatching or failing commands
        let err: Result<&[AccountRow], &str> = Err(&"network timeout");
        assert_eq!(
            evaluate_recovery_precondition(err),
            RecoveryAction::AbortDueToRefreshFailure
        );
        let ok_rows = vec![row_a];
        let ok_res: Result<&[AccountRow], &str> = Ok(&ok_rows);
        assert_eq!(
            evaluate_recovery_precondition(ok_res),
            RecoveryAction::ProceedWithDispatch
        );

        // Case E: existing live account preserves process handle during refresh
        assert_eq!(acc_b.live_process_pid, Some(4001));
    }

    #[test]
    fn test_fetch_error_is_not_authoritative_empty() {
        let local_accounts = [("acc-1", false), ("acc-2", false)];
        let err: Result<&[AccountRow], &str> = Err("network failure");
        let plan = evaluate_account_reconciliation(local_accounts, err, []);
        assert_eq!(plan, ReconciliationPlan::AbortedFetchFailed);
    }

    #[test]
    fn test_successful_empty_cloud_set_retires_all_local_accounts() {
        let local_accounts = [("acc-1", false), ("acc-2", false)];
        let empty_rows: &[AccountRow] = &[];
        let ok_empty: Result<&[AccountRow], &str> = Ok(empty_rows);
        let plan = evaluate_account_reconciliation(local_accounts, ok_empty, []);
        assert_eq!(
            plan,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec![],
                to_retire: vec!["acc-1".to_string(), "acc-2".to_string()],
            }
        );
    }

    #[test]
    fn test_partial_cloud_deletion_retires_only_missing() {
        let local_accounts = [("acc-a", false), ("acc-b", false), ("acc-c", false)];
        let cloud_rows = vec![
            AccountRow {
                id: "acc-a".to_string(),
                slot_index: 0,
                label: "Acc A".to_string(),
                username: "user_a".to_string(),
                secret_sealed: serde_json::json!({}),
                server_index: 0,
                desired_state: "running".to_string(),
                control_version: 1,
                control: serde_json::json!({}),
                config_version: 1,
                runtime: serde_json::json!({}),
                character_slot: 1,
            },
            AccountRow {
                id: "acc-c".to_string(),
                slot_index: 2,
                label: "Acc C".to_string(),
                username: "user_c".to_string(),
                secret_sealed: serde_json::json!({}),
                server_index: 2,
                desired_state: "stopped".to_string(),
                control_version: 1,
                control: serde_json::json!({}),
                config_version: 1,
                runtime: serde_json::json!({}),
                character_slot: 1,
            },
        ];
        let ok_res: Result<&[AccountRow], &str> = Ok(&cloud_rows);
        let plan = evaluate_account_reconciliation(local_accounts, ok_res, []);
        assert_eq!(
            plan,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec!["acc-a".to_string(), "acc-c".to_string()],
                to_retire: vec!["acc-b".to_string()],
            }
        );
    }

    #[test]
    fn test_failed_stop_retains_process_and_tracking() {
        // Account has a running process, stop fails
        let action = evaluate_retirement_action(true, true, Some(StopOutcome::Failed));
        assert_eq!(action, RetirementAction::RetainPendingCleanup);
    }

    #[test]
    fn test_successful_stop_allows_removal() {
        // Stop terminated cleanly
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::Terminated)),
            RetirementAction::RemoveCleaned
        );
        // Stop killed after escalation
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::Killed)),
            RetirementAction::RemoveCleaned
        );
        // Already gone
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::AlreadyGone)),
            RetirementAction::RemoveCleaned
        );
        // No process was running
        assert_eq!(
            evaluate_retirement_action(false, false, None),
            RetirementAction::RemoveCleaned
        );
    }

    #[test]
    fn test_retiring_account_cannot_autostart_or_accept_command() {
        let mut acc = AccountSnapshot {
            id: "acc-del".to_string(),
            slot_index: 1,
            username: "del_user".to_string(),
            server_index: 0,
            secret_sealed: serde_json::json!({}),
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            live_process_pid: Some(5555),
            retiring: false,
        };
        assert!(acc.can_autostart());
        assert!(acc.can_accept_command());

        acc.mark_retiring();
        assert_eq!(acc.desired_state, "stopped");
        assert!(acc.retiring);
        assert!(!acc.can_autostart());
        assert!(!acc.can_accept_command());
    }

    #[test]
    fn test_tombstone_admission_evaluation() {
        assert_eq!(
            evaluate_account_admission("acc-live", false),
            AccountAdmissionDecision::Admitted
        );
        assert_eq!(
            evaluate_account_admission("acc-dead", true),
            AccountAdmissionDecision::RejectedTombstoned
        );
    }

    #[test]
    fn test_tombstone_lifecycle_cases_f1_through_f6() {
        use std::collections::HashSet;

        let row_a = AccountRow {
            id: "acc-a".to_string(),
            slot_index: 0,
            label: "Acc A".to_string(),
            username: "user_a".to_string(),
            secret_sealed: serde_json::json!({}),
            server_index: 0,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        let row_b = AccountRow {
            id: "acc-b".to_string(),
            slot_index: 1,
            label: "Acc B".to_string(),
            username: "user_b".to_string(),
            secret_sealed: serde_json::json!({}),
            server_index: 1,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };

        // F1: local A is retiring, stale snapshot contains A, Stop fails
        // -> A remains owned, tombstone remains, no resurrection
        let mut tombstones = HashSet::new();
        tombstones.insert("acc-a".to_string());
        let local_f1 = [("acc-a", true)];
        let cloud_rows_f1 = vec![row_a.clone()];
        let plan_f1 = evaluate_account_reconciliation(
            local_f1,
            Ok::<&[AccountRow], &str>(&cloud_rows_f1),
            tombstones.iter().map(|s| s.as_str()),
        );
        assert_eq!(
            plan_f1,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec![],
                to_retire: vec!["acc-a".to_string()],
            }
        );
        // Stop fails -> process kept, account still retiring, tombstone intact
        let action_f1 = evaluate_retirement_action(true, true, Some(StopOutcome::Failed));
        assert_eq!(action_f1, RetirementAction::RetainPendingCleanup);
        assert!(tombstones.contains("acc-a"));

        // F2 (HARD GATE): local A is retiring, stale snapshot contains A, Stop succeeds
        // -> AccountState removed, tombstone remains, stale snapshot in same pass MUST NOT reinsert A
        let local_f2 = [("acc-a", true)];
        let cloud_rows_f2 = vec![row_a.clone()];
        let plan_f2 = evaluate_account_reconciliation(
            local_f2,
            Ok::<&[AccountRow], &str>(&cloud_rows_f2),
            tombstones.iter().map(|s| s.as_str()),
        );
        assert_eq!(
            plan_f2,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec![],
                to_retire: vec!["acc-a".to_string()],
            }
        );
        // Stop succeeds -> AccountState is removed, but tombstone remains
        let action_f2 = evaluate_retirement_action(true, true, Some(StopOutcome::Terminated));
        assert_eq!(action_f2, RetirementAction::RemoveCleaned);
        assert!(tombstones.contains("acc-a"));
        // Admission evaluation on fresh row A during the same merge pass must REJECT
        assert_eq!(
            evaluate_account_admission("acc-a", tombstones.contains("acc-a")),
            AccountAdmissionDecision::RejectedTombstoned
        );

        // F3: A has already been successfully retired and removed. Later stale snapshot contains A
        // -> MUST NOT insert
        let local_f3 = [("acc-b", false)];
        let cloud_rows_f3 = vec![row_a.clone(), row_b.clone()];
        let plan_f3 = evaluate_account_reconciliation(
            local_f3,
            Ok::<&[AccountRow], &str>(&cloud_rows_f3),
            tombstones.iter().map(|s| s.as_str()),
        );
        assert_eq!(
            plan_f3,
            ReconciliationPlan::Apply {
                to_insert: vec![], // acc-a is NOT inserted!
                to_update: vec!["acc-b".to_string()],
                to_retire: vec![],
            }
        );
        assert_eq!(
            evaluate_account_admission("acc-a", tombstones.contains("acc-a")),
            AccountAdmissionDecision::RejectedTombstoned
        );

        // F4: A has already been successfully retired and removed. Later AccountChanged(A)
        // -> MUST NOT insert
        assert_eq!(
            evaluate_account_admission("acc-a", tombstones.contains("acc-a")),
            AccountAdmissionDecision::RejectedTombstoned
        );

        // F5: A has already been successfully retired and removed. Later AccountAdded(A)
        // -> MUST NOT insert
        assert_eq!(
            evaluate_account_admission("acc-a", tombstones.contains("acc-a")),
            AccountAdmissionDecision::RejectedTombstoned
        );

        // F6: Missed DELETE during disconnect: local A,B, authoritative cloud contains only A
        // -> B becomes tombstoned -> safe retirement -> later stale snapshot containing B cannot resurrect B
        let mut tombstones_f6 = HashSet::<String>::new();
        let local_f6_initial = [("acc-a", false), ("acc-b", false)];
        let cloud_authoritative_a = vec![row_a.clone()];
        let plan_f6_reconnect = evaluate_account_reconciliation(
            local_f6_initial,
            Ok::<&[AccountRow], &str>(&cloud_authoritative_a),
            tombstones_f6.iter().map(|s| s.as_str()),
        );
        assert_eq!(
            plan_f6_reconnect,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec!["acc-a".to_string()],
                to_retire: vec!["acc-b".to_string()],
            }
        );
        // B is tombstoned and retired safely
        tombstones_f6.insert("acc-b".to_string());
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::Terminated)),
            RetirementAction::RemoveCleaned
        );
        // Later stale snapshot arrives with B
        let cloud_stale_with_b = vec![row_a.clone(), row_b.clone()];
        let plan_f6_stale = evaluate_account_reconciliation(
            [("acc-a", false)],
            Ok::<&[AccountRow], &str>(&cloud_stale_with_b),
            tombstones_f6.iter().map(|s| s.as_str()),
        );
        assert_eq!(
            plan_f6_stale,
            ReconciliationPlan::Apply {
                to_insert: vec![], // B is NOT in to_insert!
                to_update: vec!["acc-a".to_string()],
                to_retire: vec![],
            }
        );
        assert_eq!(
            evaluate_account_admission("acc-b", tombstones_f6.contains("acc-b")),
            AccountAdmissionDecision::RejectedTombstoned
        );
    }

    #[test]
    fn test_case_h_reconnect_token_refresh() {
        let current_token = "jwt-current-known-good-v1";

        // Subcase H1: Successful device authentication returns refreshed token
        let sign_in_ok: Result<String, &str> = Ok("jwt-refreshed-v2-token".to_string());
        let decision_ok = evaluate_reconnect_token_refresh(current_token, sign_in_ok);
        assert!(decision_ok.refreshed);
        assert_eq!(decision_ok.rest_token, "jwt-refreshed-v2-token");
        assert_eq!(decision_ok.realtime_token, "jwt-refreshed-v2-token");

        // Subcase H2: Failed refresh preserves known-good token and does not pretend success
        let sign_in_err: Result<String, &str> = Err("503 Service Unavailable");
        let decision_err = evaluate_reconnect_token_refresh(current_token, sign_in_err);
        assert!(!decision_err.refreshed);
        assert_eq!(decision_err.rest_token, current_token);
        assert_eq!(decision_err.realtime_token, current_token);
    }

    #[test]
    fn test_case_g_credential_server_update_preserves_running_process() {
        let mut live_acc = AccountSnapshot {
            id: "acc-a".to_string(),
            slot_index: 0,
            username: "user_a".to_string(),
            server_index: 0,
            secret_sealed: serde_json::json!({"version": 1}),
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            live_process_pid: Some(4444),
            retiring: false,
        };
        let row_a_new = AccountRow {
            id: "acc-a".to_string(),
            slot_index: 0,
            label: "Acc A".to_string(),
            username: "user_a_updated".to_string(),
            secret_sealed: serde_json::json!({"version": 2}),
            server_index: 5,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        live_acc.merge_cloud_fields(&row_a_new);
        assert_eq!(live_acc.username, "user_a_updated");
        assert_eq!(live_acc.server_index, 5);
        assert_eq!(live_acc.secret_sealed, serde_json::json!({"version": 2}));
        assert_eq!(live_acc.live_process_pid, Some(4444), "JVM PID must not be altered");
    }

    #[test]
    fn test_acceptance_matrix_cases_a_through_h() {
        // Case A: local A,B + fetch error -> neither retired
        let local_accounts = [("acc-a", false), ("acc-b", false)];
        let err: Result<&[AccountRow], &str> = Err("transient 503 network error");
        let plan_a = evaluate_account_reconciliation(local_accounts, err, []);
        assert_eq!(plan_a, ReconciliationPlan::AbortedFetchFailed);

        // Case B: local A,B + successful cloud [] -> A,B selected for safe retirement
        let empty_cloud: &[AccountRow] = &[];
        let ok_b: Result<&[AccountRow], &str> = Ok(empty_cloud);
        let plan_b = evaluate_account_reconciliation(local_accounts, ok_b, []);
        assert_eq!(
            plan_b,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec![],
                to_retire: vec!["acc-a".to_string(), "acc-b".to_string()],
            }
        );

        // Case C: running account DELETE + Stop failure -> process handle retained, retiring=true, blocked
        let mut acc_c = AccountSnapshot {
            id: "acc-a".to_string(),
            slot_index: 0,
            username: "user_a".to_string(),
            server_index: 0,
            secret_sealed: serde_json::json!({}),
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            live_process_pid: Some(9999),
            retiring: false,
        };
        let action_c = evaluate_retirement_action(true, true, Some(StopOutcome::Failed));
        assert_eq!(action_c, RetirementAction::RetainPendingCleanup);
        acc_c.mark_retiring();
        assert!(acc_c.retiring);
        assert_eq!(acc_c.desired_state, "stopped");
        assert_eq!(acc_c.live_process_pid, Some(9999));
        assert!(!acc_c.can_autostart());
        assert!(!acc_c.can_accept_command());

        // Case D: running account DELETE + Stop success -> local state removed only after confirmed stop
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::Terminated)),
            RetirementAction::RemoveCleaned
        );
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::Killed)),
            RetirementAction::RemoveCleaned
        );
        assert_eq!(
            evaluate_retirement_action(true, true, Some(StopOutcome::AlreadyGone)),
            RetirementAction::RemoveCleaned
        );
        assert_eq!(
            evaluate_retirement_action(false, false, None),
            RetirementAction::RemoveCleaned
        );

        // Case E: missed DELETE during disconnect -> reconnect snapshot missing B -> B safely retires
        let row_a = AccountRow {
            id: "acc-a".to_string(),
            slot_index: 0,
            label: "Acc A".to_string(),
            username: "user_a".to_string(),
            secret_sealed: serde_json::json!({}),
            server_index: 0,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        let cloud_only_a = vec![row_a.clone()];
        let ok_e: Result<&[AccountRow], &str> = Ok(&cloud_only_a);
        let plan_e = evaluate_account_reconciliation(local_accounts, ok_e, []);
        assert_eq!(
            plan_e,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec!["acc-a".to_string()],
                to_retire: vec!["acc-b".to_string()],
            }
        );

        // Case F: retiring account + later/stale snapshot containing same ID -> no resurrection
        let local_with_retiring = [("acc-a", true), ("acc-b", false)];
        let row_b = AccountRow {
            id: "acc-b".to_string(),
            slot_index: 1,
            label: "Acc B".to_string(),
            username: "user_b".to_string(),
            secret_sealed: serde_json::json!({}),
            server_index: 1,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        let cloud_stale_both = vec![row_a.clone(), row_b.clone()];
        let plan_f = evaluate_account_reconciliation(
            local_with_retiring,
            Ok::<&[AccountRow], &str>(&cloud_stale_both),
            ["acc-a"],
        );
        assert_eq!(
            plan_f,
            ReconciliationPlan::Apply {
                to_insert: vec![],
                to_update: vec!["acc-b".to_string()],
                to_retire: vec!["acc-a".to_string()],
            }
        );
        // acc_c is retiring:
        acc_c.merge_cloud_fields(&row_a);
        assert!(acc_c.retiring);
        assert_eq!(acc_c.desired_state, "stopped");
        assert_eq!(acc_c.live_process_pid, Some(9999));
        assert!(!acc_c.can_autostart());
        assert!(!acc_c.can_accept_command());

        // Case G: credential/server UPDATE while JVM running -> metadata updated, PID preserved
        let mut live_acc = AccountSnapshot {
            id: "acc-a".to_string(),
            slot_index: 0,
            username: "user_a".to_string(),
            server_index: 0,
            secret_sealed: serde_json::json!({"version": 1}),
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            live_process_pid: Some(1234),
            retiring: false,
        };
        let row_a_update = AccountRow {
            id: "acc-a".to_string(),
            slot_index: 0,
            label: "Acc A".to_string(),
            username: "user_a_new".to_string(),
            secret_sealed: serde_json::json!({"version": 2}),
            server_index: 4,
            desired_state: "running".to_string(),
            control_version: 1,
            control: serde_json::json!({}),
            config_version: 1,
            runtime: serde_json::json!({}),
            character_slot: 1,
        };
        live_acc.merge_cloud_fields(&row_a_update);
        assert_eq!(live_acc.username, "user_a_new");
        assert_eq!(live_acc.server_index, 4);
        assert_eq!(live_acc.secret_sealed, serde_json::json!({"version": 2}));
        assert_eq!(live_acc.live_process_pid, Some(1234));

        // Case H: reconnect token refresh -> exact newly returned JWT is token supplied to realtime
        let decision_h1 = evaluate_reconnect_token_refresh("token-v1", Ok::<String, &str>("token-v2".to_string()));
        assert!(decision_h1.refreshed);
        assert_eq!(decision_h1.rest_token, "token-v2");
        assert_eq!(decision_h1.realtime_token, "token-v2");

        let decision_h2 = evaluate_reconnect_token_refresh("token-v1", Err("timeout"));
        assert!(!decision_h2.refreshed);
        assert_eq!(decision_h2.rest_token, "token-v1");
        assert_eq!(decision_h2.realtime_token, "token-v1");
    }

    #[test]
    fn test_abandoned_spot_scan_recovery_semantics_a1_through_a7() {
        let dev_self = "dev-100";
        let dev_other = "dev-200";

        // A1: Fresh agent process sees device-owned running detect-spots command
        let decision_a1 =
            evaluate_abandoned_spot_scan(Some(dev_self), dev_self, "detect-spots", "running");
        assert_eq!(decision_a1, AbandonedRecoveryDecision::RecoverAsFailed);
        assert_eq!(
            ABANDONED_SPOT_SCAN_MESSAGE,
            "Detect Spots scan abandoned because zeus-agent restarted before completion."
        );

        // A2: Fresh process sees running command for another device
        let decision_a2 =
            evaluate_abandoned_spot_scan(Some(dev_other), dev_self, "detect-spots", "running");
        assert_eq!(decision_a2, AbandonedRecoveryDecision::IgnoreOtherDevice);

        // A3: Fresh process sees running non-detect-spots command
        let decision_a3 =
            evaluate_abandoned_spot_scan(Some(dev_self), dev_self, "start", "running");
        assert_eq!(
            decision_a3,
            AbandonedRecoveryDecision::IgnoreOtherCommandType
        );

        // A4: Fresh process sees queued detect-spots command
        let decision_a4 =
            evaluate_abandoned_spot_scan(Some(dev_self), dev_self, "detect-spots", "queued");
        assert_eq!(
            decision_a4,
            AbandonedRecoveryDecision::IgnoreNonRunningStatus
        );

        // Batch filter testing A1 - A4 together
        let cmds = vec![
            CommandRow {
                id: "cmd-a1".to_string(),
                account_id: Some("acc-1".to_string()),
                device_id: Some(dev_self.to_string()),
                kind: "detect-spots".to_string(),
                payload: None,
                expires_at: "2026-09-21T10:00:00Z".to_string(),
            },
            CommandRow {
                id: "cmd-a2".to_string(),
                account_id: Some("acc-2".to_string()),
                device_id: Some(dev_other.to_string()),
                kind: "detect-spots".to_string(),
                payload: None,
                expires_at: "2026-09-21T10:00:00Z".to_string(),
            },
            CommandRow {
                id: "cmd-a3".to_string(),
                account_id: Some("acc-1".to_string()),
                device_id: Some(dev_self.to_string()),
                kind: "start".to_string(),
                payload: None,
                expires_at: "2026-09-21T10:00:00Z".to_string(),
            },
        ];

        let selected = filter_abandoned_detect_spots_commands(cmds, dev_self);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].id, "cmd-a1");

        // A5: Same process has pending_spot_scan and realtime reconnects -> running command remains active
        // When realtime reconnects, startup recovery is NOT invoked; active pending scan is preserved
        let reconnect_action_with_pending = evaluate_spot_scan_reconnect_action(true, false);
        assert_eq!(
            reconnect_action_with_pending,
            ReconnectSpotScanAction::PreservePendingScan
        );
        let reconnect_action_no_pending = evaluate_spot_scan_reconnect_action(false, false);
        assert_eq!(
            reconnect_action_no_pending,
            ReconnectSpotScanAction::NoAction
        );
        // Process startup with no pending scan triggers startup recovery
        let startup_action = evaluate_spot_scan_reconnect_action(false, true);
        assert_eq!(
            startup_action,
            ReconnectSpotScanAction::RunStartupAbandonedRecovery
        );

        // A6: Agent restart loses pending state and stale sidecar files exist
        // Test that sidecar files are wiped and no result is published
        let temp_dir = tempfile::tempdir().unwrap();
        let home = temp_dir.path();
        // Simulate leftover sidecar files from crashed previous run
        std::fs::write(
            home.join(crate::spot_scan::SPOT_REQUEST_FILE_NAME),
            b"old req",
        )
        .unwrap();
        std::fs::write(
            home.join(crate::spot_scan::SPOT_RESULT_PAYLOAD_FILE_NAME),
            b"{\"scan_id\":\"cmd-old\"}",
        )
        .unwrap();
        std::fs::write(
            home.join(crate::spot_scan::SPOT_RESULT_READY_FILE_NAME),
            b"ready",
        )
        .unwrap();

        assert!(
            home.join(crate::spot_scan::SPOT_RESULT_READY_FILE_NAME)
                .exists()
        );
        assert!(
            home.join(crate::spot_scan::SPOT_RESULT_PAYLOAD_FILE_NAME)
                .exists()
        );
        assert!(home.join(crate::spot_scan::SPOT_REQUEST_FILE_NAME).exists());

        // Startup cleanup wipes all sidecar files
        crate::spot_scan::clean_spot_files(home);
        assert!(
            !home
                .join(crate::spot_scan::SPOT_RESULT_READY_FILE_NAME)
                .exists()
        );
        assert!(
            !home
                .join(crate::spot_scan::SPOT_RESULT_PAYLOAD_FILE_NAME)
                .exists()
        );
        assert!(!home.join(crate::spot_scan::SPOT_REQUEST_FILE_NAME).exists());

        // Polling returns NoReadyMarker -> no stale result can ever be published
        match crate::spot_scan::poll_spot_result(home, "cmd-old") {
            crate::spot_scan::SpotPollOutcome::NoReadyMarker => {}
            other => panic!("expected NoReadyMarker after cleanup, got {:?}", other),
        }

        // A7: Recovery query/API error follows recoverable error conventions and does not fabricate success
        let outcome_err = evaluate_abandoned_recovery_outcome::<(), _>(Err("network timeout"));
        assert_eq!(outcome_err, AbandonedRecoveryOutcome::QueryFailed);

        let outcome_success_none = evaluate_abandoned_recovery_outcome::<String, &str>(Ok(vec![]));
        assert_eq!(outcome_success_none, AbandonedRecoveryOutcome::NoneFound);

        let outcome_success_recovered =
            evaluate_abandoned_recovery_outcome::<String, &str>(Ok(vec!["cmd-1".to_string()]));
        assert_eq!(
            outcome_success_recovered,
            AbandonedRecoveryOutcome::Recovered(1)
        );
    }

    #[test]
    fn test_startup_recovery_barrier_semantics_b1_through_b9() {
        // B1: Recovery query succeeds and no abandoned rows exist -> barrier passes
        let barrier_b1 = evaluate_startup_barrier_step(&Ok::<u32, &str>(0), 1, 5);
        assert_eq!(
            barrier_b1,
            StartupBarrierAction::ProceedToRealtime {
                recovered_count: 0
            }
        );

        // B2: Recovery query succeeds and all abandoned rows are failed successfully -> barrier passes
        let barrier_b2 = evaluate_startup_barrier_step(&Ok::<u32, &str>(3), 1, 5);
        assert_eq!(
            barrier_b2,
            StartupBarrierAction::ProceedToRealtime {
                recovered_count: 3
            }
        );

        // B3: Recovery query fails transiently -> barrier does not pass, requires retry
        let barrier_b3_r1 = evaluate_startup_barrier_step(&Err::<u32, &str>("network timeout"), 1, 5);
        assert_eq!(
            barrier_b3_r1,
            StartupBarrierAction::Retry {
                attempt: 1,
                max_retries: 5
            }
        );
        // When max attempts exhausted, fatal abort prevents command consumption
        let barrier_b3_exhausted =
            evaluate_startup_barrier_step(&Err::<u32, &str>("network timeout"), 5, 5);
        assert_eq!(
            barrier_b3_exhausted,
            StartupBarrierAction::FatalAbort {
                attempts_exhausted: 5
            }
        );

        // B4: One abandoned command finalization fails -> recovery reports incomplete/failure and startup barrier remains closed
        let cmd_a = CommandRow {
            id: "cmd-a".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "detect-spots".to_string(),
            payload: None,
            expires_at: "2026-09-21T12:00:00Z".to_string(),
        };
        let cmd_b = CommandRow {
            id: "cmd-b".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "detect-spots".to_string(),
            payload: None,
            expires_at: "2026-09-21T12:00:00Z".to_string(),
        };
        let cmd_c = CommandRow {
            id: "cmd-c".to_string(),
            account_id: Some("acc-2".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "detect-spots".to_string(),
            payload: None,
            expires_at: "2026-09-21T12:00:00Z".to_string(),
        };
        let batch = vec![cmd_a.clone(), cmd_b.clone(), cmd_c.clone()];
        let res_b4 = simulate_abandoned_batch_finalization(&batch, Some("cmd-b"));
        assert!(res_b4.is_err(), "partial recovery must report error, not false success");
        let barrier_b4 = evaluate_startup_barrier_step(&res_b4, 1, 5);
        assert_eq!(
            barrier_b4,
            StartupBarrierAction::Retry {
                attempt: 1,
                max_retries: 5
            }
        );

        // B5: Retry after partial recovery -> already-terminal rows (A and C) skipped, remaining running row (B) retried
        let retry_batch = vec![cmd_b.clone()];
        let res_b5 = simulate_abandoned_batch_finalization(&retry_batch, None);
        assert_eq!(res_b5, Ok(1));
        let barrier_b5 = evaluate_startup_barrier_step(&res_b5, 2, 5);
        assert_eq!(
            barrier_b5,
            StartupBarrierAction::ProceedToRealtime {
                recovered_count: 1
            }
        );

        // B6: Successful retry after transient failure -> barrier passes exactly once and realtime startup may proceed
        let attempt_1_err = evaluate_startup_barrier_step(&Err::<u32, &str>("503 service unavailable"), 1, 5);
        assert_eq!(
            attempt_1_err,
            StartupBarrierAction::Retry {
                attempt: 1,
                max_retries: 5
            }
        );
        let attempt_2_ok = evaluate_startup_barrier_step(&Ok::<u32, &str>(2), 2, 5);
        assert_eq!(
            attempt_2_ok,
            StartupBarrierAction::ProceedToRealtime {
                recovered_count: 2
            }
        );

        // B7: Same-process realtime reconnect with pending scan -> abandoned recovery is not invoked and pending scan survives
        let reconnect_with_pending = evaluate_spot_scan_reconnect_action(true, false);
        assert_eq!(
            reconnect_with_pending,
            ReconnectSpotScanAction::PreservePendingScan
        );
        let reconnect_without_pending = evaluate_spot_scan_reconnect_action(false, false);
        assert_eq!(
            reconnect_without_pending,
            ReconnectSpotScanAction::NoAction
        );

        // B8: Normal queued command recovery after barrier -> existing queued drain behavior remains unchanged
        let queued_cmd = CommandRow {
            id: "cmd-q".to_string(),
            account_id: Some("acc-1".to_string()),
            device_id: Some("dev-1".to_string()),
            kind: "detect-spots".to_string(),
            payload: None,
            expires_at: "2026-09-21T12:30:00Z".to_string(),
        };
        assert_eq!(
            evaluate_abandoned_spot_scan(queued_cmd.device_id.as_deref(), "dev-1", &queued_cmd.kind, "queued"),
            AbandonedRecoveryDecision::IgnoreNonRunningStatus
        );

        // B9: R3B1 normal Detect Spots success/timeout -> existing scan lifecycle remains unchanged
        assert_eq!(crate::spot_scan::SpotScanStatus::Completed.as_str(), "completed");
        assert_eq!(crate::spot_scan::SpotScanStatus::Timeout.as_str(), "timeout");
    }

    #[test]
    fn test_character_slot_deserialization_and_validation() {
        let legacy_json = serde_json::json!({
            "id": "acc-legacy",
            "slot_index": 0,
            "label": "bot1",
            "username": "user1",
            "secret_sealed": {},
            "server_index": 0,
            "desired_state": "running",
            "control_version": 13,
            "control": {},
            "config_version": 1,
            "runtime": {}
        });
        let parsed_legacy: AccountRow = serde_json::from_value(legacy_json).unwrap();
        assert_eq!(parsed_legacy.character_slot, 1, "legacy/missing character_slot deserializes to 1");

        let slot2_json = serde_json::json!({
            "id": "acc-slot2",
            "slot_index": 0,
            "label": "bot2",
            "username": "user2",
            "secret_sealed": {},
            "server_index": 0,
            "desired_state": "running",
            "control_version": 13,
            "control": {},
            "config_version": 1,
            "runtime": {},
            "character_slot": 2
        });
        let parsed_slot2: AccountRow = serde_json::from_value(slot2_json).unwrap();
        assert_eq!(parsed_slot2.character_slot, 2, "character_slot=2 is accepted");

        let slot3_json = serde_json::json!({
            "id": "acc-slot3",
            "slot_index": 0,
            "label": "bot3",
            "username": "user3",
            "secret_sealed": {},
            "server_index": 0,
            "desired_state": "running",
            "control_version": 13,
            "control": {},
            "config_version": 1,
            "runtime": {},
            "character_slot": 3
        });
        let parsed_slot3: AccountRow = serde_json::from_value(slot3_json).unwrap();
        assert_eq!(parsed_slot3.character_slot, 3, "character_slot=3 is accepted");

        assert!(validate_character_slot(1).is_ok());
        assert!(validate_character_slot(2).is_ok());
        assert!(validate_character_slot(3).is_ok());
        assert!(validate_character_slot(0).is_err(), "character_slot=0 must be rejected");
        assert!(validate_character_slot(4).is_err(), "character_slot=4 must be rejected");
        assert!(validate_character_slot(-1).is_err(), "negative character_slot must be rejected");
    }

    #[test]
    fn test_character_slot_runtime_capability_advertisement() {
        let compatible_manifest = JarManifest {
            jar_sha256: "0bcd6917d8d87faf9fe78fa938abfe5cdf16c0153fcc876deb337d022bb036fd".to_string(),
            jar_size: 1137800,
            ctl_version: 13,
            snapshot_version: 6,
            ctl_key_count: 35,
            snapshot_key_count: 48,
            built_at: "2026-09-22T16:51:38Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };

        // 1. Valid compatible JAR does advertise capability with the stable token
        assert!(compatible_manifest.is_character_slot_compatible());
        let advertised = compatible_manifest.advertised_agent_version();
        assert_eq!(advertised, "0.1.0+character-slot-v1");
        assert!(advertised.contains(JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN));

        // 2. Old runtime JAR SHA (5048b590...) lacks the capability
        let old_manifest = JarManifest {
            jar_sha256: "5048b590a98f23989291f7e87985ca2e77616273bdfec703fdc3827d519ca127".to_string(),
            jar_size: 1137406,
            ctl_version: 13,
            snapshot_version: 6,
            ctl_key_count: 35,
            snapshot_key_count: 48,
            built_at: "2026-09-22T15:34:49Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        assert!(!old_manifest.is_character_slot_compatible());
        let old_advertised = old_manifest.advertised_agent_version();
        assert_eq!(old_advertised, "0.1.0");
        assert!(!old_advertised.contains(JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN));

        // 3. Invalid JAR SHA does not advertise capability
        let mut invalid_jar = compatible_manifest.clone();
        invalid_jar.jar_sha256 = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        assert!(!invalid_jar.is_character_slot_compatible());
        assert!(!invalid_jar.advertised_agent_version().contains(JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN));

        // 4. Invalid ctl_version does not advertise capability
        let mut invalid_ctl = compatible_manifest.clone();
        invalid_ctl.ctl_version = 12;
        assert!(!invalid_ctl.is_character_slot_compatible());
        assert!(!invalid_ctl.advertised_agent_version().contains(JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN));

        // 5. No duplicate token across repeated announcements (deterministic)
        let mut already_advertised = compatible_manifest.clone();
        already_advertised.agent_version = "0.1.0+character-slot-v1".to_string();
        assert_eq!(already_advertised.advertised_agent_version(), "0.1.0+character-slot-v1");
        assert_eq!(already_advertised.advertised_agent_version().matches(JarManifest::CHARACTER_SLOT_CAPABILITY_TOKEN).count(), 1);

        // 6. devices PATCH payload uses ONLY existing columns and preserves jar_sha256
        let patch_payload = SupabaseRest::build_device_patch_payload(&compatible_manifest);
        let patch_obj = patch_payload.as_object().expect("payload must be a JSON object");
        let allowed_columns = [
            "id", "pair_code", "pubkey", "agent_version", "status", "last_seen",
            "jar_sha256", "jar_ctl_version", "jar_snapshot_version", "jar_ctl_key_count",
            "viewer_url", "viewer_expires_at"
        ];
        for key in patch_obj.keys() {
            assert!(allowed_columns.contains(&key.as_str()), "Key {key} is not an existing devices column");
        }
        assert_eq!(patch_obj.get("jar_sha256").and_then(|v| v.as_str()), Some("0bcd6917d8d87faf9fe78fa938abfe5cdf16c0153fcc876deb337d022bb036fd"));
        assert_eq!(patch_obj.get("agent_version").and_then(|v| v.as_str()), Some("0.1.0+character-slot-v1"));
        assert_eq!(patch_obj.get("status").and_then(|v| v.as_str()), Some("online"));

        // 7. Test loading actual repository zeus-jar.json
        if let Some(loaded_manifest) = read_jar_manifest("../../../vendor/game/zeus-jar.json") {
            assert_eq!(loaded_manifest.jar_sha256, JarManifest::ENHANCEMENT_ENGINE_COMPATIBLE_JAR_SHA256);
            assert_eq!(loaded_manifest.ctl_version, 14);
            assert_eq!(loaded_manifest.ctl_key_count, 37);
            assert!(loaded_manifest.is_character_slot_compatible());
            assert!(loaded_manifest.is_visual_qol_compatible());
            assert_eq!(loaded_manifest.advertised_agent_version(), "0.1.0+character-slot-v1.visual-qol-v1");
        }
    }

    #[test]
    fn test_visual_qol_capability_advertisement() {
        let v14_manifest = JarManifest {
            jar_sha256: JarManifest::VISUAL_QOL_COMPATIBLE_JAR_SHA256.to_string(),
            jar_size: 1137800,
            ctl_version: 14,
            snapshot_version: 6,
            ctl_key_count: 37,
            snapshot_key_count: 48,
            built_at: "2026-09-23T00:00:00Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        // 1. QoL v14 JAR advertises both tokens
        assert!(v14_manifest.is_visual_qol_compatible());
        assert!(v14_manifest.is_character_slot_compatible());
        assert_eq!(
            v14_manifest.advertised_agent_version(),
            "0.1.0+character-slot-v1.visual-qol-v1"
        );

        // 1b. Inventory Catalog v14 JAR also advertises both tokens and neither advertises enhancement-queue-v1
        let inventory_manifest = JarManifest {
            jar_sha256: JarManifest::INVENTORY_CATALOG_COMPATIBLE_JAR_SHA256.to_string(),
            jar_size: 1139652,
            ctl_version: 14,
            snapshot_version: 6,
            ctl_key_count: 37,
            snapshot_key_count: 48,
            built_at: "2026-09-24T04:14:20Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        assert!(inventory_manifest.is_visual_qol_compatible());
        assert!(inventory_manifest.is_character_slot_compatible());
        assert_eq!(
            inventory_manifest.advertised_agent_version(),
            "0.1.0+character-slot-v1.visual-qol-v1"
        );
        assert!(!inventory_manifest.advertised_agent_version().contains("enhancement-queue"));

        // 1c. Enhancement Engine v14 JAR also advertises both tokens and neither advertises enhancement-queue-v1
        let enhancement_manifest = JarManifest {
            jar_sha256: JarManifest::ENHANCEMENT_ENGINE_COMPATIBLE_JAR_SHA256.to_string(),
            jar_size: 1145162,
            ctl_version: 14,
            snapshot_version: 6,
            ctl_key_count: 37,
            snapshot_key_count: 48,
            built_at: "2026-09-24T10:24:22Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        assert!(enhancement_manifest.is_visual_qol_compatible());
        assert!(enhancement_manifest.is_character_slot_compatible());
        assert_eq!(
            enhancement_manifest.advertised_agent_version(),
            "0.1.0+character-slot-v1.visual-qol-v1"
        );
        assert!(!enhancement_manifest.advertised_agent_version().contains("enhancement-queue"));

        // 2. Exact known v13 JAR advertises character-slot-v1 ONLY, never visual-qol-v1
        let v13_manifest = JarManifest {
            jar_sha256: JarManifest::CHARACTER_SLOT_COMPATIBLE_JAR_SHA256.to_string(),
            jar_size: 1137800,
            ctl_version: 13,
            snapshot_version: 6,
            ctl_key_count: 35,
            snapshot_key_count: 48,
            built_at: "2026-09-22T16:51:38Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        assert!(v13_manifest.is_character_slot_compatible());
        assert!(!v13_manifest.is_visual_qol_compatible());
        assert_eq!(
            v13_manifest.advertised_agent_version(),
            "0.1.0+character-slot-v1"
        );
        assert!(!v13_manifest.advertised_agent_version().contains("visual-qol-v1"));

        // 3. Unknown JAR SHA never advertises either capability even if ctl_version matches
        let unknown_manifest = JarManifest {
            jar_sha256: "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff".to_string(),
            jar_size: 1137800,
            ctl_version: 14,
            snapshot_version: 6,
            ctl_key_count: 37,
            snapshot_key_count: 48,
            built_at: "2026-09-23T00:00:00Z".to_string(),
            patcher_sha256: "89cac9ea1e3485efa757eea68b43d31dfbc374577de81cc3ea25bbb037d81a3c".to_string(),
            agent_version: "".to_string(),
        };
        assert!(!unknown_manifest.is_character_slot_compatible());
        assert!(!unknown_manifest.is_visual_qol_compatible());
        assert_eq!(unknown_manifest.advertised_agent_version(), "0.1.0");

        // 4. Deterministic multi-token serialization without duplicate tokens
        let mut already_advertised = v14_manifest.clone();
        already_advertised.agent_version = "0.1.0+visual-qol-v1.character-slot-v1".to_string();
        assert_eq!(
            already_advertised.advertised_agent_version(),
            "0.1.0+character-slot-v1.visual-qol-v1"
        );
        assert_eq!(
            already_advertised.advertised_agent_version().matches("character-slot-v1").count(),
            1
        );
        assert_eq!(
            already_advertised.advertised_agent_version().matches("visual-qol-v1").count(),
            1
        );
    }
}

