//! Agent pairing — B3.1 + B3.2 (AGENT-SPEC §5.3)
//!
//! ## Luồng
//!
//! ```text
//! boot
//!  ├─ state/device.json tồn tại + pair_code == null (đã pair)?
//! │    └─ CÓ  → trả về PairState::Paired { device_id, access_token }
//! │
//!  └─ KHÔNG:
//!      ├─ Derive keypair P-256 từ HKDF(RAILWAY_SERVICE_ID ‖ "pair-key-v1")
//!      │   → stable qua redeploy (B3.1 — không random!)
//!      ├─ Sinh pair code 8 ký tự (hex của SHA256[:4] của pubkey)
//!      ├─ POST /devices với pubkey + pair_code → tạo hàng chưa có user_id
//!      ├─ In pair code ra stdout (Railway log)
//!      └─ Poll GET /devices/{id}?select=user_id mỗi 5 s
//!          └─ user_id != null → đã pair, lấy access_token bằng device credentials
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

use crate::supabase_rest::SupabaseRest;

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

    // Nếu pair_code != null thì chưa pair xong.
    if djson.pair_code.is_some() {
        eprintln!("[pairing] found device.json with pair_code, resuming poll loop");
        return Ok(None); // Sẽ vào pair_device để poll tiếp
    }

    eprintln!("[pairing] already paired, device_id={}", djson.device_id);

    let seed_bytes = hex::decode(&djson.private_key_seed)
        .map_err(|e| format!("decode private_key_seed: {e}"))?;
    let mut key_arr = [0u8; 32];
    if seed_bytes.len() != 32 {
        return Err("private_key_seed is not 32 bytes".into());
    }
    key_arr.copy_from_slice(&seed_bytes);

    // Derive pubkey de tinh device_password
    let secret_key = p256::SecretKey::from_bytes((&key_arr).into())
        .map_err(|e| format!("restore secret key: {e}"))?;
    let pubkey_bytes = secret_key.public_key().to_sec1_bytes();

    // Refresh access token.
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

    // SEC1 uncompressed 65 bytes (verified: WebCrypto expects this, V4.3).
    let pubkey_bytes = public_key.to_sec1_bytes();
    let pubkey_b64 = BASE64.encode(&pubkey_bytes);

    // Pair code: hex của SHA256[:4] của pubkey. 8 ký tự hex, dễ nhập.
    let pair_code = {
        let hash = Sha256::digest(&pubkey_bytes);
        hex::encode(&hash[..4]).to_uppercase()
    };

    let device_name = std::env::var("ZEUS_DEVICE_NAME")
        .unwrap_or_else(|_| "knight-node".into());

    eprintln!("[pairing] ┌──────────────────────────────────┐");
    eprintln!("[pairing] │  PAIR CODE: {pair_code}              │");
    eprintln!("[pairing] │  Nhập vào web dashboard để pair   │");
    eprintln!("[pairing] └──────────────────────────────────┘");
    eprintln!("[pairing] Device name: {device_name}");

    // POST /devices → tạo hàng chưa có user_id.
    let device_id = rest
        .create_unpaired_device(&pair_code, &device_name, &pubkey_b64)
        .map_err(|e| format!("create_unpaired_device: {e}"))?;

    eprintln!("[pairing] device_id={device_id}, polling for user claim every 5s...");

    // Lưu device.json với pair_code để resume nếu agent restart.
    let djson = DeviceJson {
        device_id: device_id.clone(),
        pair_code: Some(pair_code.clone()),
        private_key_seed: hex::encode(&seed),
    };
    save_device_json(device_json_path, &djson)?;

    // B3.2: poll mỗi 5s cho đến khi user_id != null.
    loop {
        sleep(Duration::from_secs(5));
        match rest.check_device_claimed(&device_id) {
            Ok(true) => {
                eprintln!("[pairing] device claimed! obtaining session...");
                break;
            }
            Ok(false) => {
                // Chưa claim, poll tiếp.
            }
            Err(e) => {
                eprintln!("[pairing] poll error: {e}, retrying...");
            }
        }
    }

    // Claim xong → lấy access token.
    let access_token = rest
        .sign_in_as_device(&device_id, &pubkey_bytes)
        .map_err(|e| format!("sign_in_as_device after claim: {e}"))?;

    // Cập nhật device.json: pair_code = null (đã paired).
    let djson = DeviceJson {
        device_id: device_id.clone(),
        pair_code: None,
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
}

