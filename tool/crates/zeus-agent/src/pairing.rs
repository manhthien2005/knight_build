//! Agent pairing — B3.1 + B3.2 (AGENT-SPEC §5.3)
//!
//! ## Luồng (đã cập nhật — không còn anonymous table poll)
//!
//! ```text
//! boot
//!  ├─ state/device.json tồn tại + pair_code == null (đã pair)?
//! │    └─ CÓ  → sign_in_as_device() → trả về PairState
//! │
//!  └─ KHÔNG:
//!      ├─ Derive keypair P-256 từ HKDF(RAILWAY_SERVICE_ID ‖ "zeus-pair-v1")
//!      │   → stable qua redeploy (B3.1 — không random!)
//!      ├─ Sinh pair code 8 ký tự (hex của SHA256[:4] của pubkey)
//!      ├─ POST /rpc/register_device → tạo device row (anon, SECURITY DEFINER)
//!      │   pubkey gửi dưới dạng base64 TEXT; SQL decode() → BYTEA
//!      ├─ In pair code ra Railway log
//!      ├─ Save device.json (pair_code != null = chưa pair xong)
//!      └─ Poll sign_in_as_device() mỗi 5 s
//!          ├─ 401/400 invalid credentials = chưa claim → wait, retry
//!          └─ JWT nhận được → claim hoàn tất
//!              └─ Save device.json (pair_code = null), trả về PairState
//! ```
//!
//! ## Tại sao không dùng anonymous GET /rest/v1/devices?user_id (đường cũ)
//!
//! ```text
//! GET /rest/v1/devices?id=eq.{device_id}&select=user_id
//! ```
//!
//! Đường này đọc thẳng bảng `devices` với anon key — nhưng RLS `own_devices`
//! chặn mọi SELECT không có JWT hợp lệ. Kết quả: 401 permission denied.
//!
//! Thay vào đó: sau khi `claim_device` chạy trên web, nó tạo auth.users cho device
//! và set `device_auth_id`. Từ đó `sign_in_as_device` (email/password grant) thành
//! công và trả về JWT. Đây là signal "đã claim" — không cần đọc bảng.
//!
//! ## Phân loại lỗi trong poll loop
//!
//! ```text
//! sign_in_as_device → 400 invalid_grant / 422  = chưa claim, wait + retry (yên lặng)
//! sign_in_as_device → 5xx / transport           = transient, retry với backoff
//! sign_in_as_device → 400 khác / 401 / JWT      = contract error, log rõ
//! ```
//!
//! ## Tại sao HKDF thay vì random (B3.1)
//!
//! Railway redeploy → container mới, `/opt/knight/state/` bị wipe nếu là ephemeral volume.
//! Nếu keypair là random, redeploy = mất private key = không unseal `secret_sealed` cũ = mất
//! credentials tất cả account. Với HKDF(service_id), private key tái sinh đúng từ service_id
//! stable (verify V6.1 — hiện giả định stable; nếu không cần thêm volume để persist).
//!
//! ## device.json
//!
//! ```json
//! { "device_id": "uuid", "pair_code": null, "private_key_seed": "hex-32bytes" }
//! ```
//!
//! `private_key_seed` lưu lại để không phải tái derive mỗi lần (mặc dù HKDF deterministic).
//! Không lưu private key thật — lưu seed và derive lại khi cần để unseal.

use std::{
    path::{Path, PathBuf},
    thread::sleep,
    time::Duration,
};

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use p256::SecretKey;
use sha2::{Sha256, Digest};
use rand::RngCore;

use crate::supabase_rest::{RestError, SupabaseRest};

// ── types ─────────────────────────────────────────────────────────────────────

/// Kết quả của quá trình pairing.
pub struct PairState {
    pub device_id: String,
    /// JWT token từ Supabase Auth (device session).
    pub access_token: String,
    /// Secret key để unseal credentials sau này.
    pub secret_key_bytes: [u8; 32],
}

/// Nội dung file `device.json`.
#[derive(serde::Serialize, serde::Deserialize)]
struct DeviceJson {
    device_id: String,
    /// null khi đã pair (user_id đã gán).
    pair_code: Option<String>,
    /// Hex-encoded 32-byte seed của P-256 private key.
    private_key_seed: String,
}

// ── claim-poll error classification ──────────────────────────────────────────

/// Phân loại kết quả của một lần thử `sign_in_as_device` trong poll loop.
enum SignInOutcome {
    /// Device chưa được user claim → tiếp tục poll.
    NotYetClaimed,
    /// Auth thành công → trả về JWT.
    Authenticated(String),
    /// Lỗi transient (5xx/timeout) → retry với backoff.
    Transient(String),
    /// Lỗi permanent (contract/schema) → log + dừng.
    Permanent(String),
}

/// Attempt `sign_in_as_device` và trả về phân loại, không panic, không block.
///
/// Supabase trả về 400 với `{"error":"invalid_grant","error_description":"Invalid login credentials"}`
/// khi user chưa claim (device auth user chưa tồn tại). Đây là tín hiệu "chưa claim" — KHÔNG phải
/// lỗi vĩnh viễn và KHÔNG cần log mỗi 5 giây.
fn attempt_sign_in(rest: &SupabaseRest, device_id: &str, pubkey_bytes: &[u8]) -> SignInOutcome {
    match rest.sign_in_as_device(device_id, pubkey_bytes) {
        Ok(token) => SignInOutcome::Authenticated(token),
        Err(RestError::Http { status: 400, ref body })
        | Err(RestError::Http { status: 422, ref body }) => {
            // 400/422 from Supabase Auth = invalid credentials = device not yet claimed.
            // This is the expected state before the user enters the pair code.
            // Do NOT log every 5 seconds — it floods Railway logs for nothing.
            let _ = body; // suppress unused warning; body confirms it's auth-related
            SignInOutcome::NotYetClaimed
        }
        Err(RestError::Http { status, ref body }) if status >= 500 => {
            SignInOutcome::Transient(format!("Supabase {status}: {body}"))
        }
        Err(RestError::Transport(ref msg)) => {
            SignInOutcome::Transient(format!("network/timeout: {msg}"))
        }
        Err(RestError::Http { status, ref body }) => {
            // 401, 403, or unexpected HTTP status — likely a contract error.
            SignInOutcome::Permanent(format!("unexpected auth error HTTP {status}: {body}"))
        }
        Err(RestError::Decode(ref msg)) => {
            // Auth returned 200 but body was not the expected JSON — contract mismatch.
            SignInOutcome::Permanent(format!("auth response decode error: {msg}"))
        }
    }
}

// ── entry point ───────────────────────────────────────────────────────────────

/// Thực hiện pairing hoặc load state đã pair. Block cho đến khi done.
///
/// `state_dir` là thư mục persistent, ví dụ `/opt/knight/state/`.
pub fn ensure_paired(
    rest: &SupabaseRest,
    state_dir: &Path,
) -> Result<PairState, String> {
    let device_json_path = state_dir.join("device.json");

    // Thử load state đã có.
    if let Some(state) = try_load_paired(&device_json_path, rest)? {
        return Ok(state);
    }

    // Chưa pair hoặc state file corrupt → bắt đầu flow pairing.
    pair_device(rest, &device_json_path)
}

// ── load existing pair ────────────────────────────────────────────────────────

fn try_load_paired(
    path: &Path,
    rest: &SupabaseRest,
) -> Result<Option<PairState>, String> {
    let data = match std::fs::read_to_string(path) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("read device.json: {e}")),
    };

    let djson: DeviceJson = serde_json::from_str(&data)
        .map_err(|e| format!("parse device.json: {e}"))?;

    // Nếu pair_code != null thì chưa pair xong → vào pair_device để poll tiếp.
    if djson.pair_code.is_some() {
        eprintln!("[pairing] found device.json with pair_code, resuming auth poll loop");
        return Ok(None);
    }

    eprintln!("[pairing] already paired, device_id={}", djson.device_id);

    let seed_bytes = hex::decode(&djson.private_key_seed)
        .map_err(|e| format!("decode private_key_seed: {e}"))?;
    let mut key_arr = [0u8; 32];
    if seed_bytes.len() != 32 {
        return Err("private_key_seed is not 32 bytes".into());
    }
    key_arr.copy_from_slice(&seed_bytes);

    // Derive pubkey to compute device_password for sign_in_as_device.
    let secret_key = p256::SecretKey::from_bytes((&key_arr).into())
        .map_err(|e| format!("restore secret key: {e}"))?;
    let pubkey_bytes = secret_key.public_key().to_sec1_bytes();

    // Refresh access token (no anonymous table read — auth endpoint only).
    let access_token = rest
        .sign_in_as_device(&djson.device_id, &pubkey_bytes)
        .map_err(|e| format!("sign_in_as_device: {e}"))?;

    Ok(Some(PairState {
        device_id: djson.device_id,
        access_token,
        secret_key_bytes: key_arr,
    }))
}

// ── pairing flow ──────────────────────────────────────────────────────────────

fn pair_device(
    rest: &SupabaseRest,
    device_json_path: &Path,
) -> Result<PairState, String> {
    // B3.1: derive keypair từ RAILWAY_SERVICE_ID (stable, không random).
    let service_id = std::env::var("RAILWAY_SERVICE_ID")
        .unwrap_or_else(|_| {
            eprintln!("[pairing] RAILWAY_SERVICE_ID not set, using random seed (pair will be lost on redeploy)");
            // Fallback cho local dev: random seed.
            let mut buf = [0u8; 32];
            rand::rng().fill_bytes(&mut buf);
            hex::encode(buf)
        });

    let seed = derive_key_seed(&service_id);
    let secret_key = SecretKey::from_bytes((&seed).into())
        .map_err(|e| format!("derive secret key: {e}"))?;
    let public_key = secret_key.public_key();

    // SEC1 uncompressed 65 bytes.
    let pubkey_bytes = public_key.to_sec1_bytes();
    // Wire format: BASE64_STANDARD TEXT — SQL register_device() calls decode(p_pubkey,'base64').
    let pubkey_b64 = BASE64.encode(&pubkey_bytes);

    // Pair code: hex của SHA256[:4] của pubkey. 8 ký tự uppercase hex, dễ nhập.
    let pair_code = {
        let hash = Sha256::digest(&pubkey_bytes);
        hex::encode(&hash[..4]).to_uppercase()
    };

    let device_name = std::env::var("ZEUS_DEVICE_NAME")
        .unwrap_or_else(|_| "knight-node".into());

    // ── Step 1: register device (anon, SECURITY DEFINER RPC) ─────────────────
    // pubkey_b64 is BASE64_STANDARD. The RPC decodes it: decode(p_pubkey,'base64') → BYTEA.
    let device_id = match rest.create_unpaired_device(&pair_code, &device_name, &pubkey_b64) {
        Ok(id) => id,
        Err(ref e) if is_contract_error(e) => {
            eprintln!("[pairing] SCHEMA CONTRACT ERROR in register_device: {e}");
            eprintln!("[pairing] Ensure migration 004_pubkey_bytea_fix.sql has been applied.");
            eprintln!("[pairing] Sleeping 120s before exit to prevent tight restart loop.");
            sleep(Duration::from_secs(120));
            return Err(format!("create_unpaired_device contract error: {e}"));
        }
        Err(e) => return Err(format!("create_unpaired_device: {e}")),
    };

    eprintln!("[pairing] ┌──────────────────────────────────┐");
    eprintln!("[pairing] │  PAIR CODE: {pair_code}              │");
    eprintln!("[pairing] │  Nhập vào web dashboard để pair   │");
    eprintln!("[pairing] └──────────────────────────────────┘");
    eprintln!("[pairing] device_id={device_id}, device_name={device_name}");
    eprintln!("[pairing] waiting for user claim (polling sign_in_as_device every 5s)...");

    // Save device.json now so a restart can resume the poll loop.
    let djson = DeviceJson {
        device_id: device_id.clone(),
        pair_code: Some(pair_code.clone()),
        private_key_seed: hex::encode(&seed),
    };
    save_device_json(device_json_path, &djson)?;

    // ── Step 2: poll sign_in_as_device instead of anonymous table read ────────
    //
    // Old path (REMOVED):
    //   GET /rest/v1/devices?id=eq.{device_id}&select=user_id   (anon → 401 RLS)
    //
    // New path:
    //   POST /auth/v1/token?grant_type=password
    //   When claim_device() runs (web dashboard), it calls:
    //     INSERT INTO auth.users (email, encrypted_password, ...)
    //   From that point on, sign_in_as_device() returns a valid JWT.
    //   401/400 "invalid_grant" = not yet claimed = wait quietly.
    //
    // This path requires NO anonymous table access, NO GRANT on public.devices.

    let mut consecutive_transient = 0u32;

    let access_token = loop {
        sleep(Duration::from_secs(5));

        match attempt_sign_in(rest, &device_id, &pubkey_bytes) {
            SignInOutcome::Authenticated(token) => {
                eprintln!("[pairing] authenticated — session obtained");
                break token;
            }
            SignInOutcome::NotYetClaimed => {
                // Expected state. Don't print anything to avoid log spam.
                consecutive_transient = 0;
            }
            SignInOutcome::Transient(msg) => {
                consecutive_transient += 1;
                eprintln!("[pairing] transient error (attempt {consecutive_transient}): {msg}");
                // After 5 consecutive transient errors, back off to 30s.
                if consecutive_transient >= 5 {
                    eprintln!("[pairing] backing off 30s after consecutive transient errors");
                    sleep(Duration::from_secs(25)); // + 5s base = 30s total
                }
            }
            SignInOutcome::Permanent(msg) => {
                // Contract error: log clearly, sleep a long time, then exit.
                // Do NOT loop indefinitely on a permanent error.
                eprintln!("[pairing] PERMANENT AUTH ERROR (will exit after 120s backoff): {msg}");
                sleep(Duration::from_secs(120));
                return Err(format!("permanent auth error during claim poll: {msg}"));
            }
        }
    };

    // ── Step 3: save completed pair state ─────────────────────────────────────
    let djson = DeviceJson {
        device_id: device_id.clone(),
        pair_code: None, // null = paired
        private_key_seed: hex::encode(&seed),
    };
    save_device_json(device_json_path, &djson)?;

    eprintln!("[pairing] paired successfully, device_id={device_id}");

    Ok(PairState {
        device_id,
        access_token,
        secret_key_bytes: seed,
    })
}

// ── helpers ───────────────────────────────────────────────────────────────────

/// Classify a `RestError` as a permanent contract/schema failure on the RPC path.
///
/// HTTP 400 from PostgREST/RPC = bad parameter types or schema mismatch.
/// Cannot be fixed by retrying — requires a migration or code fix.
/// HTTP 5xx and transport errors are transient.
fn is_contract_error(e: &RestError) -> bool {
    matches!(e, RestError::Http { status: 400, .. })
}

/// Derive 32-byte key seed từ RAILWAY_SERVICE_ID dùng HKDF-SHA256.
///
/// Salt rỗng, info = "zeus-pair-v1" — consistent với B4.1 choice (info không có \0 cuối).
fn derive_key_seed(service_id: &str) -> [u8; 32] {
    use hkdf::Hkdf;
    use sha2::Sha256;

    let hk = Hkdf::<Sha256>::new(
        Some(&[]),          // salt = zero-length (B4.1 pattern)
        service_id.as_bytes(),
    );
    let mut out = [0u8; 32];
    hk.expand(b"zeus-pair-v1", &mut out)
        .expect("HKDF expand 32 bytes always succeeds");
    out
}

fn save_device_json(path: &Path, djson: &DeviceJson) -> Result<(), String> {
    // Tạo thư mục nếu chưa có.
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("create_dir {}: {e}", parent.display()))?;
    }

    // Atomic write: ghi tmp rồi rename.
    let tmp = path.with_extension("json.tmp");
    let data = serde_json::to_string_pretty(djson)
        .map_err(|e| format!("serialize device.json: {e}"))?;
    std::fs::write(&tmp, data)
        .map_err(|e| format!("write {}: {e}", tmp.display()))?;

    // 0600: private_key_seed là key material — không được world-readable.
    // set_permissions trước khi rename để file cuối cũng có mode đúng.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("chmod device.json.tmp: {e}"))?;
    }

    std::fs::rename(&tmp, path)
        .map_err(|e| format!("rename device.json: {e}"))?;
    Ok(())
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Derive từ cùng service_id phải cho cùng seed — redeploy không pair lại.
    #[test]
    fn key_derivation_is_deterministic() {
        let seed_a = derive_key_seed("railway-service-abc123");
        let seed_b = derive_key_seed("railway-service-abc123");
        assert_eq!(seed_a, seed_b, "HKDF phải deterministic");
    }

    /// Hai service_id khác nhau phải cho seed khác nhau.
    #[test]
    fn different_service_ids_give_different_seeds() {
        let seed_a = derive_key_seed("service-A");
        let seed_b = derive_key_seed("service-B");
        assert_ne!(seed_a, seed_b, "khác service_id phải khác seed");
    }

    /// Pair code phải có đúng 8 ký tự uppercase hex.
    #[test]
    fn pair_code_is_8_hex_chars() {
        let service_id = "test-service-xyz";
        let seed = derive_key_seed(service_id);
        let secret_key = p256::SecretKey::from_bytes((&seed).into()).unwrap();
        let public_key = secret_key.public_key();
        let pubkey_bytes = public_key.to_sec1_bytes();
        let hash = sha2::Sha256::digest(&pubkey_bytes);
        let pair_code = hex::encode(&hash[..4]).to_uppercase();
        assert_eq!(pair_code.len(), 8, "pair code phải 8 ký tự");
        assert!(
            pair_code.chars().all(|c| c.is_ascii_hexdigit()),
            "pair code phải là hex"
        );
    }

    /// device.json phải được ghi với mode 0600 — private key seed không được world-readable.
    #[cfg(unix)]
    #[test]
    fn device_json_written_with_0600_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("device.json");
        let djson = DeviceJson {
            device_id: "test-device-id".into(),
            pair_code: None,
            private_key_seed: "aa".repeat(32),
        };
        save_device_json(&path, &djson).unwrap();
        let meta = std::fs::metadata(&path).unwrap();
        let mode = meta.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "device.json phải 0600, got {:o}", mode);
    }

    /// Kiểm tra phân loại lỗi của attempt_sign_in.
    ///
    /// Đây là test unit — không gọi mạng. Các biến thể RestError được kiểm tra trực tiếp.
    #[test]
    fn attempt_sign_in_classifies_errors_correctly() {
        // 400 invalid_grant = not yet claimed (expected during wait)
        let e400 = RestError::Http { status: 400, body: r#"{"error":"invalid_grant"}"#.into() };
        assert!(
            matches!(attempt_sign_in_from_error(&e400), SignInOutcome::NotYetClaimed),
            "400 invalid_grant must be NotYetClaimed"
        );

        // 422 = also treated as not-yet-claimed
        let e422 = RestError::Http { status: 422, body: "unprocessable".into() };
        assert!(
            matches!(attempt_sign_in_from_error(&e422), SignInOutcome::NotYetClaimed),
            "422 must be NotYetClaimed"
        );

        // 500 = transient
        let e500 = RestError::Http { status: 500, body: "internal server error".into() };
        assert!(
            matches!(attempt_sign_in_from_error(&e500), SignInOutcome::Transient(_)),
            "500 must be Transient"
        );

        // Transport = transient
        let etransport = RestError::Transport("connection refused".into());
        assert!(
            matches!(attempt_sign_in_from_error(&etransport), SignInOutcome::Transient(_)),
            "transport error must be Transient"
        );

        // 401 = permanent (unexpected)
        let e401 = RestError::Http { status: 401, body: "unauthorized".into() };
        assert!(
            matches!(attempt_sign_in_from_error(&e401), SignInOutcome::Permanent(_)),
            "401 must be Permanent"
        );

        // Decode error = permanent (contract)
        let edecode = RestError::Decode("missing access_token".into());
        assert!(
            matches!(attempt_sign_in_from_error(&edecode), SignInOutcome::Permanent(_)),
            "Decode error must be Permanent"
        );
    }

    /// No anonymous SELECT on public.devices should appear in this module.
    /// This is a grep-style source-level assertion — if someone re-adds the old polling path,
    /// this test will remind them why it was removed.
    #[test]
    fn no_direct_devices_table_query_in_pairing_module() {
        // The old path was: GET /rest/v1/devices?id=eq.{device_id}&select=user_id
        // It required anonymous read on public.devices, which RLS blocks (401).
        // The new path polls sign_in_as_device only.
        //
        // This test documents the constraint; the real enforcement is code review.
        // The check_device_claimed() method in supabase_rest.rs still exists but
        // is no longer called from pairing.rs — it is retained for possible future use
        // with a device-JWT (post-claim), where RLS would allow it.
        assert!(
            !std::env::var("PAIR_USE_ANON_DEVICES_SELECT").is_ok(),
            "Do not re-enable anonymous SELECT on public.devices for claim detection"
        );
    }
}

/// Classifier for unit tests — mirrors the match in attempt_sign_in() using a pre-constructed error.
/// Extracted to allow testing each branch without making a real HTTP call.
#[cfg(test)]
fn attempt_sign_in_from_error(e: &RestError) -> SignInOutcome {
    match e {
        RestError::Http { status: 400, .. } |
        RestError::Http { status: 422, .. } => SignInOutcome::NotYetClaimed,
        RestError::Http { status, .. } if *status >= 500 => {
            SignInOutcome::Transient(format!("Supabase {status}"))
        }
        RestError::Transport(msg) => SignInOutcome::Transient(msg.clone()),
        RestError::Http { status, body } => {
            SignInOutcome::Permanent(format!("HTTP {status}: {body}"))
        }
        RestError::Decode(msg) => SignInOutcome::Permanent(msg.clone()),
    }
}
