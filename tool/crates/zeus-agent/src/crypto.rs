//! Đã hoàn thành — cả bốn tham số V4.3 đã đo và có test pin.
//!
//! Target path: `Tool/tool/crates/zeus-agent/src/crypto.rs`
//!
//! ## Ràng buộc duy nhất về bảo mật của dự án
//!
//! Chủ dự án đã chốt: **bỏ qua bảo mật VPS** — noVNC không password, URL public, node bị chiếm
//! thì chấp nhận. Nhưng **mật khẩu account game không được lộ**. Hai điều đó xung đột trực tiếp,
//! vì plaintext *buộc* phải có mặt trên node:
//!
//! `bs.c()` — constructor màn login của client — đọc record store `user_pass` và tự submit
//! opcode 1. Agent không drive login; nó chỉ seed hai record store **trước khi JVM start**
//! (`zeus_core::wire::seed_credentials`). Vậy plaintext phải tồn tại trên đĩa, ở đúng chỗ client
//! đọc được.
//!
//! Điều đó nghĩa là "mã hoá mật khẩu trong DB" kiểu mặc định **không đủ**. Nếu agent giữ
//! service_role key và `SELECT` cả bảng, thì một node bị chiếm đọc được mật khẩu của **mọi**
//! user — vi phạm đúng ràng buộc trên. Envelope encryption theo device khớp chính xác threat
//! model đã nêu:
//!
//! | Ai lộ | Mất gì |
//! |---|---|
//! | Supabase | **0 mật khẩu** — DB chỉ thấy ciphertext |
//! | một node | chỉ account của **chính user đó** trên **chính device đó** |
//!
//! ## Bốn tham số đã chốt (V4.3, 2026-09-14)
//!
//! Cơ chế là P-256 ECDH → HKDF-SHA256 → AES-256-GCM. Bốn chỗ mà các thư viện lệch nhau đã được
//! đo bằng một test vector hai chiều thật (WebCrypto seal → Rust unseal), và đều có test pin:
//!
//! 1. **Encoding public key** — SEC1 uncompressed, 65 byte, mở đầu `0x04`. Không phải raw, không
//!    phải DER/SPKI. `p256::PublicKey::from_sec1_bytes` / `to_sec1_bytes`.
//! 2. **HKDF salt** — **zero-length** (`Some(&[])`), tức là có mặt nhưng rỗng. "Vắng mặt" và
//!    "rỗng" cho ra key khác nhau, và rỗng là cái WebCrypto `importKey("HKDF", …, salt=empty)`
//!    dùng.
//! 3. **`info`** — đúng 7 byte ASCII `zeus-v1`, **không có `\0` cuối**.
//! 4. **AES-GCM tag** — **kèm trong** `ct` ở 16 byte cuối, không tách thành trường riêng. Đó là
//!    layout của cả RustCrypto `aes-gcm` lẫn WebCrypto `encrypt`.
//!
//! Vector gốc nằm trong `../../cross-domain/VERIFY-SPEC.md` V4.3 và được tái lập trong
//! `webcrypto_seal_unseals_in_rust`. Đổi bất kỳ tham số nào là test đó fail.
//!
//! ## Vì sao P-256 chứ không phải X25519 sealed box
//!
//! `libsodium` sealed box là lựa chọn gọn hơn về mặt mật mã, nhưng WebCrypto **không** có
//! X25519 — sẽ phải ship WASM libsodium vào web, thêm một dependency nặng và một bề mặt tấn công.
//! P-256 ECDH + HKDF + AES-GCM đều là WebCrypto **native** ở mọi browser, và Rust có
//! `p256` + `hkdf` + `aes-gcm` (RustCrypto). Đánh đổi: tự lắp ráp thay vì một hàm sealed_box —
//! chính là lý do cần test vector.
//!
//! ## Vì sao không tái dùng `credential_vault.rs` như plan cũ nói
//!
//! `../../cross-domain/BUILD-PLAN-V1.md` và `../AGENT-SPEC.md` B4.2 nói "tái dùng khung AES-GCM + `ZeroingVec`".
//! **Đã verify là không dùng được:** trên Linux, `credential_vault.rs` chỉ còn
//! `is_empty_for_test()` và `expose_for_validation()` là cross-platform; `open()`, `cipher()` và
//! `SecretBytes::new()` đều nằm sau `#[cfg(windows)]` vì khoá lấy từ DPAPI. Ba warning dead-code
//! của `cargo check -p zeus-core --lib` trên Linux chính là bằng chứng
//! (`../../cross-domain/VERIFY-RESULTS.md` §6.2).
//!
//! Vậy: `ZeroingVec` là **một ý tưởng đáng chép** (zero hoá bộ nhớ khi drop), không phải một
//! module để gọi. File này viết mới.

use std::path::Path;

use aes_gcm::aead::{Aead, AeadCore};
use aes_gcm::{Aes256Gcm, KeyInit};
use base64::Engine;
use hkdf::Hkdf;
use p256::elliptic_curve::ops::Reduce;
use p256::{FieldBytes, NonZeroScalar, PublicKey, SecretKey, U256};
use rand_core::RngCore;
use sha2::Sha256;

/// `SealedSecret.alg`. Anything else is refused, never guessed — same fail-closed philosophy as
/// the control file. Value is fixed by CLOUD-SPEC §4 and confirmed by the V4.3 measurement.
pub const SEALING_ALGORITHM: &str = "ecdh-p256-hkdf-sha256-aes256gcm";

/// HKDF info for the shared secret. V4.3 measured: 7 ASCII bytes, hex `7a6575732d7631`, and
/// crucially **no trailing NUL**.
pub const HKDF_INFO: &[u8] = b"zeus-v1";

/// Separate info label for deriving the device key from the pair secret, so a pair secret can
/// never be replayed as a session key or vice versa.
const DEVICE_KEY_INFO: &[u8] = b"zeus-device-key-v1";

/// AES-GCM nonce length. V4.3 measured: 12 bytes.
pub const NONCE_LEN: usize = 12;

/// `aes_gcm::Nonce` is aliased over the *NonceSize*, not the cipher (aead's own `Nonce<A>` is the
/// one parameterised by cipher), so it must be spelled this way.
type Nonce12 = aes_gcm::Nonce<<Aes256Gcm as AeadCore>::NonceSize>;

fn b64_decode(field: &str, value: &str) -> Result<Vec<u8>, CryptoError> {
    base64::engine::general_purpose::STANDARD
        .decode(value.trim())
        .map_err(|e| CryptoError::Malformed(format!("{field} is not valid base64: {e}")))
}

pub(crate) fn b64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// ECDH then HKDF-SHA256. V4.3 confirmed both sides reach the identical 32-byte key with
/// salt = explicitly empty and info = `zeus-v1`.
fn derive_shared(private_key: &SecretKey, peer_pub: &PublicKey) -> [u8; 32] {
    let shared = p256::ecdh::diffie_hellman(private_key.to_nonzero_scalar(), peer_pub.as_affine());
    let hk = Hkdf::<Sha256>::new(Some(&[]), shared.raw_secret_bytes().as_slice());
    let mut okm = [0u8; 32];
    hk.expand(HKDF_INFO, &mut okm)
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    okm
}

/// Maps 32 bytes onto a valid non-zero P-256 scalar.
///
/// Plain `Reduce` (mod n), **not** `reduce_nonzero`: the latter adds one before reducing, so
/// `reduce_nonzero(x)` != `SecretKey::from_bytes(x)` and, worse, is not idempotent — feeding a
/// stored `to_bytes()` back in yields a *different* key. That breaks V4.3 interop and the
/// redeploy stability `derive` exists for. Plain `Reduce` satisfies both properties it needs
/// (probe-verified): it matches the WebCrypto/`from_bytes` vector byte-for-byte, and it
/// round-trips `to_bytes()` exactly for every realistic seed. Exactly two inputs reduce to
/// zero — the all-zero buffer and the curve order `n` itself, both of probability ~2^-256 — so
/// those fall back to scalar 1 instead of an unusable key.
fn identity_from_scalar_bytes(bytes: &[u8; 32]) -> DeviceIdentity {
    let scalar = <p256::Scalar as Reduce<U256>>::reduce(&U256::from_be_slice(bytes));
    let private_key: SecretKey = match NonZeroScalar::new(scalar).into_option() {
        Some(nonzero) => nonzero.into(),
        None => {
            let mut one = [0u8; 32];
            one[31] = 1;
            SecretKey::from_bytes(&FieldBytes::from(one)).expect("scalar 1 is always valid")
        }
    };
    let public_key_sec1 = private_key.public_key().to_sec1_bytes().to_vec();
    DeviceIdentity {
        private_key,
        public_key_sec1,
    }
}

/// Ciphertext như nó nằm trong `accounts.secret_sealed` (sql/001_schema.sql).
///
/// Shape này **đã chốt** — nó khớp những gì WebCrypto có thể phát ra và những gì RustCrypto có
/// `alg`, `info` and all three field encodings are settled — see the V4.3 vector test.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SealedSecret {
    /// Nhận dạng bộ tham số. Agent **từ chối** unseal nếu không biết chuỗi này, thay vì đoán —
    /// cùng triết lý fail-closed với control file.
    pub alg: String,
    /// HKDF info, để nguyên văn cho dễ đối chiếu giữa hai bên.
    pub info: String,
    /// Public key ephemeral của browser, base64.
    pub eph_pub: String,
    /// Nonce AES-GCM, base64, 12 bytes (`NONCE_LEN`).
    pub nonce: String,
    /// Ciphertext, base64, **with the 16-byte AES-GCM tag appended** (RustCrypto `aes-gcm`
    /// layout, which is what WebCrypto's `encrypt` also emits).
    pub ct: String,
}

/// Plaintext sau khi unseal.
///
/// Sống trong RAM giữa `unseal` và `seed_credentials`, rồi phải bị zero. Không log, không ghi
/// file tạm, và **không bao giờ đưa vào argv** — `/proc/<pid>/cmdline` đọc được bởi mọi user
/// trong container, nên một tham số dòng lệnh là một chỗ rò.
pub struct PlaintextCredentials {
    pub username: String,
    pub password: String,
}

impl PlaintextCredentials {
    /// Overwrites the in-memory plaintext with zeros. Zeroing before the buffer is freed is enough
    /// for this threat model (not a cold-boot defence). Extracted as a method so it can be asserted
    /// directly on live data — reading the bytes *after* drop observes freed memory that the
    /// allocator may already have reused.
    fn zeroize(&mut self) {
        // SAFETY: `String::as_mut_vec` is unsafe because the caller may write non-UTF-8 bytes.
        // Writing `0` is U+0000, which *is* valid UTF-8, so the String's invariant holds after
        // this returns. Length is preserved (no truncate/extend), so the allocation stays sound.
        unsafe {
            self.username.as_mut_vec().iter_mut().for_each(|b| *b = 0);
            self.password.as_mut_vec().iter_mut().for_each(|b| *b = 0);
        }
    }
}

impl Drop for PlaintextCredentials {
    fn drop(&mut self) {
        self.zeroize();
    }
}

pub struct DeviceIdentity {
    pub private_key: p256::SecretKey,
    /// SEC1 uncompressed, 65 byte. Đây là thứ đẩy lên `devices.pubkey`.
    pub public_key_sec1: Vec<u8>,
}

impl DeviceIdentity {
    /// Derives the keypair from a stable seed rather than generating one, so a redeploy does not
    /// force every user to re-pair. V4.3 settled the HKDF parameters and the byte-to-scalar
    /// mapping; what remains open is V6.1 — whether `RAILWAY_SERVICE_ID` itself survives a
    /// redeploy. Because that is unproven, `load_or_derive` persists the key to disk and prefers
    /// the stored copy, which makes this function a fallback rather than the load-bearing path.
    pub fn derive(service_id: &str, pair_secret: &[u8]) -> Result<Self, CryptoError> {
        if service_id.is_empty() {
            return Err(CryptoError::KeyUnavailable(
                "RAILWAY_SERVICE_ID is empty".to_string(),
            ));
        }
        if pair_secret.is_empty() {
            return Err(CryptoError::KeyUnavailable("pair secret is empty".to_string()));
        }
        // pair_secret as the HKDF salt and service_id as the IKM: the secret is the high-entropy
        // part, and the service id only separates one node from another.
        let hk = Hkdf::<Sha256>::new(Some(pair_secret), service_id.as_bytes());
        let mut ikm = [0u8; 32];
        hk.expand(DEVICE_KEY_INFO, &mut ikm)
            .map_err(|e| CryptoError::KeyUnavailable(e.to_string()))?;
        let identity = identity_from_scalar_bytes(&ikm);
        ikm.iter_mut().for_each(|b| *b = 0);
        Ok(identity)
    }

    /// Reads `device.key` (raw 32-byte scalar, mode 0600) when present; otherwise derives and
    /// writes it. The stored copy wins so that a redeploy keeps the same key — and therefore the
    /// same pairing — even if `RAILWAY_SERVICE_ID` proves unstable (V6.1).
    pub fn load_or_derive(
        state_dir: &Path,
        service_id: &str,
        pair_secret: &[u8],
    ) -> Result<Self, CryptoError> {
        let key_path = state_dir.join("device.key");

        match std::fs::read(&key_path) {
            Ok(existing) => {
                if existing.len() != 32 {
                    return Err(CryptoError::KeyUnavailable(format!(
                        "{} is {} bytes, expected 32",
                        key_path.display(),
                        existing.len()
                    )));
                }
                let mut bytes = [0u8; 32];
                bytes.copy_from_slice(&existing);
                let identity = identity_from_scalar_bytes(&bytes);
                bytes.iter_mut().for_each(|b| *b = 0);
                Ok(identity)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let identity = Self::derive(service_id, pair_secret)?;
                std::fs::create_dir_all(state_dir).map_err(|e| {
                    CryptoError::KeyUnavailable(format!("cannot create {}: {e}", state_dir.display()))
                })?;
                // Create 0600 BEFORE writing, so the key is never briefly world-readable.
                // cfg wraps only the OpenOptions construction; both branches yield a File.
                let mut file = {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::OpenOptionsExt;
                        std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .mode(0o600)
                            .open(&key_path)
                    }
                    #[cfg(not(unix))]
                    std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&key_path)
                }
                .map_err(|e| {
                    CryptoError::KeyUnavailable(format!(
                        "cannot create {}: {e}",
                        key_path.display()
                    ))
                })?;
                use std::io::Write;
                file.write_all(&identity.private_key.to_bytes()).map_err(|e| {
                    CryptoError::KeyUnavailable(format!(
                        "cannot write {}: {e}",
                        key_path.display()
                    ))
                })?;
                Ok(identity)
            }
            Err(e) => Err(CryptoError::KeyUnavailable(format!(
                "cannot read {}: {e}",
                key_path.display()
            ))),
        }
    }
}

/// ECDH + HKDF + AES-GCM decrypt.
///
/// All four parameters were settled by the recorded V4.3 vector and are pinned by
/// `webcrypto_seal_unseals_in_rust`: SEC1 uncompressed 65-byte public key, zero-length HKDF salt,
/// `info` = `zeus-v1` (7 ASCII bytes, no NUL), and the AES-GCM tag appended inside `ct`.
pub fn unseal(
    identity: &DeviceIdentity,
    sealed: &SealedSecret,
) -> Result<PlaintextCredentials, CryptoError> {
    let mut plaintext = decrypt_sealed(identity, sealed)?;
    let parsed = parse_credentials(&plaintext);
    // The decrypted buffer held the password in the clear; wipe it before returning.
    plaintext.iter_mut().for_each(|b| *b = 0);
    parsed
}

/// Validates the parameter set and runs the AEAD, returning the raw plaintext bytes.
///
/// Split out from `unseal` so the V4.3 interop vector — which sealed the bare string
/// `matkhau-test-123`, not credential JSON — can be asserted against the crypto layer directly.
fn decrypt_sealed(
    identity: &DeviceIdentity,
    sealed: &SealedSecret,
) -> Result<Vec<u8>, CryptoError> {
    if sealed.alg != SEALING_ALGORITHM {
        return Err(CryptoError::UnknownAlgorithm(sealed.alg.clone()));
    }
    if sealed.info.as_bytes() != HKDF_INFO {
        return Err(CryptoError::Malformed(format!(
            "info is {:?}, expected {:?}",
            sealed.info,
            String::from_utf8_lossy(HKDF_INFO)
        )));
    }

    let eph_pub_bytes = b64_decode("eph_pub", &sealed.eph_pub)?;
    let eph_pub = PublicKey::from_sec1_bytes(&eph_pub_bytes)
        .map_err(|e| CryptoError::Malformed(format!("eph_pub is not a SEC1 point: {e}")))?;

    let nonce_bytes = b64_decode("nonce", &sealed.nonce)?;
    if nonce_bytes.len() != NONCE_LEN {
        return Err(CryptoError::Malformed(format!(
            "nonce is {} bytes, expected {NONCE_LEN}",
            nonce_bytes.len()
        )));
    }
    let nonce = Nonce12::try_from(nonce_bytes.as_slice())
        .map_err(|_| CryptoError::Malformed("nonce has the wrong length".to_string()))?;

    let ct = b64_decode("ct", &sealed.ct)?;

    let mut key = derive_shared(&identity.private_key, &eph_pub);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| CryptoError::Malformed("derived key is not 32 bytes".to_string()))?;
    // A tag mismatch and a wrong key are indistinguishable here, and both mean "do not proceed".
    let result = cipher
        .decrypt(&nonce, ct.as_slice())
        .map_err(|_| CryptoError::AuthenticationFailed);
    key.iter_mut().for_each(|b| *b = 0);
    result
}

/// The plaintext contract from CLOUD-SPEC §4: `{"username":"…","password":"…"}`.
#[derive(serde::Deserialize)]
struct CredentialJson {
    username: String,
    password: String,
}

fn parse_credentials(plaintext: &[u8]) -> Result<PlaintextCredentials, CryptoError> {
    let parsed: CredentialJson = serde_json::from_slice(plaintext).map_err(|e| {
        CryptoError::Malformed(format!("plaintext is not credential JSON: {e}"))
    })?;
    Ok(PlaintextCredentials {
        username: parsed.username,
        password: parsed.password,
    })
}

/// Chiều ngược lại, để test: agent seal bằng public key của chính nó rồi unseal.
///
/// Không dùng trong production (browser mới là bên seal), nhưng **bắt buộc** phải có để test
/// round-trip mà không cần mở DevTools mỗi lần.
pub fn seal_for_test(
    recipient_pub_sec1: &[u8],
    plaintext: &PlaintextCredentials,
) -> Result<SealedSecret, CryptoError> {
    let recipient = PublicKey::from_sec1_bytes(recipient_pub_sec1)
        .map_err(|e| CryptoError::Malformed(format!("recipient pubkey is not SEC1: {e}")))?;

    // Ephemeral key from 32 random bytes reduced to a non-zero scalar. Deliberately NOT
    // `SecretKey::random(&mut rand::rngs::OsRng)`: p256 0.14 pulls rand_core 0.10 while rand 0.9
    // pulls 0.9, so rand's OsRng does not satisfy p256's CryptoRng bound. Reducing random bytes
    // gives the same uniform-ish scalar without depending on either RNG's trait plumbing.
    let mut eph_bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut eph_bytes);
    let eph = identity_from_scalar_bytes(&eph_bytes);
    eph_bytes.iter_mut().for_each(|b| *b = 0);

    let mut key = derive_shared(&eph.private_key, &recipient);

    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce12::try_from(nonce_bytes.as_slice())
        .map_err(|_| CryptoError::Malformed("internal: bad nonce length".to_string()))?;

    let mut body = serde_json::to_vec(&serde_json::json!({
        "username": plaintext.username,
        "password": plaintext.password,
    }))
    .map_err(|e| CryptoError::Malformed(format!("cannot encode credentials: {e}")))?;

    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|_| CryptoError::Malformed("derived key is not 32 bytes".to_string()))?;
    let ct = cipher
        .encrypt(&nonce, body.as_slice())
        .map_err(|_| CryptoError::Malformed("AES-GCM encrypt failed".to_string()))?;

    // Three buffers held the password or the key; wipe all of them.
    body.iter_mut().for_each(|b| *b = 0);
    key.iter_mut().for_each(|b| *b = 0);

    Ok(SealedSecret {
        alg: SEALING_ALGORITHM.to_string(),
        info: String::from_utf8_lossy(HKDF_INFO).into_owned(),
        eph_pub: b64_encode(&eph.public_key_sec1),
        nonce: b64_encode(&nonce_bytes),
        ct: b64_encode(&ct),
    })
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CryptoError {
    /// `alg` không phải bộ tham số agent này biết. Không đoán, không fallback.
    #[error("unknown sealing algorithm: {0}")]
    UnknownAlgorithm(String),
    #[error("malformed sealed secret: {0}")]
    Malformed(String),
    #[error("device key mismatch or corrupted ciphertext")]
    AuthenticationFailed,
    #[error("device key material unreadable: {0}")]
    KeyUnavailable(String),
}

/// Ghi plaintext vào record store, rồi đảm bảo nó không còn ở đâu khác.
///
/// Đây là chỗ **duy nhất** plaintext chạm đĩa, và nó chạm vì client đòi hỏi
/// (`bs.c()` đọc `user_pass`). Đường dẫn này phải ngắn nhất có thể: unseal → seed → drop.
pub fn seed_then_forget(
    microemu_home: &Path,
    credentials: PlaintextCredentials,
    server_index: u8,
) -> zeus_core::wire::CoreResult<()> {
    let result = zeus_core::wire::seed_credentials(
        microemu_home,
        &credentials.username,
        &credentials.password,
        server_index,
    );
    // `credentials` drop ở cuối hàm, và `Drop` zero hoá cả hai chuỗi. Không copy ra ngoài,
    // không giữ lại trong state của account.
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The recorded V4.3 vector: a browser sealed `matkhau-test-123` with WebCrypto to a fixed
    /// device key; Rust must recover it byte-for-byte. The plaintext is a bare string (not
    /// credential JSON), so this asserts the crypto layer via `decrypt_sealed`, not `unseal`.
    /// Transcribed from hex so the vector matches `cross-domain/VERIFY-SPEC.md` V4.3 exactly.
    #[test]
    fn webcrypto_seal_unseals_in_rust() {
        // Device secret used to produce the vector (V4.3, 2026-09-14).
        let secret = hex::decode(
            "112233445566778899aabbccddeeff00112233445566778899aabbccddeeff01",
        )
        .expect("valid hex");
        let mut sk = [0u8; 32];
        sk.copy_from_slice(&secret);
        let identity = identity_from_scalar_bytes(&sk);

        let sealed = SealedSecret {
            alg: SEALING_ALGORITHM.to_string(),
            info: String::from_utf8_lossy(HKDF_INFO).into_owned(),
            eph_pub: b64_encode(
                &hex::decode("040c9522221875ee9591d018541b7fdcff452d6142e4f9ac1ee1aa530cbc0113b394cf4a97228e0c032d05ead9437706f8acf725f9a913ebbd8adebbd9b70dc0e4").unwrap(),
            ),
            nonce: b64_encode(&hex::decode("041cfcf85de0906589eb082f").unwrap()),
            ct: b64_encode(
                &hex::decode("09e004f6c548e81d7f62bf1481414c67d89193848f2b4152c69787e395b48f1e").unwrap(),
            ),
        };

        let mut plaintext = decrypt_sealed(&identity, &sealed).expect("V4.3 vector must unseal");
        assert_eq!(
            String::from_utf8_lossy(&plaintext),
            "matkhau-test-123",
            "WebCrypto seal did not unseal to the recorded plaintext"
        );
        plaintext.iter_mut().for_each(|b| *b = 0);
    }

    /// The reverse direction the spec's `seal_for_test` exists for: agent seals credential JSON
    /// to its own public key, then unseals it back. Closes the loop on the credential-JSON shape
    /// that `unseal` (not just `decrypt_sealed`) enforces.
    #[test]
    fn seal_then_unseal_round_trips_credentials() {
        let identity = DeviceIdentity::derive("svc-fixed", b"pair-secret-with-padding-0123456789")
            .expect("derive");
        let creds = PlaintextCredentials {
            username: "acc1".to_string(),
            password: "hunter2".to_string(),
        };
        let sealed = seal_for_test(&identity.public_key_sec1, &creds).expect("seal");
        assert_eq!(sealed.alg, SEALING_ALGORITHM);

        let back = unseal(&identity, &sealed).expect("unseal");
        assert_eq!(back.username, "acc1");
        assert_eq!(back.password, "hunter2");
    }

    /// An unknown `alg` must be refused, never guessed — fail-closed, like the control file.
    #[test]
    fn an_unknown_algorithm_is_refused_not_guessed() {
        let identity = DeviceIdentity::derive("svc", b"pair-secret-with-padding-0123456789")
            .expect("derive");
        let sealed = SealedSecret {
            alg: "something-invented".into(),
            info: String::from_utf8_lossy(HKDF_INFO).into_owned(),
            eph_pub: b64_encode(&identity.public_key_sec1),
            nonce: b64_encode(&[0u8; NONCE_LEN]),
            ct: b64_encode(&[0u8; 32]),
        };
        assert!(matches!(
            decrypt_sealed(&identity, &sealed),
            Err(CryptoError::UnknownAlgorithm(_))
        ));
    }

    /// A wrong `info` is also refused — it changes the derived key, so unsealing with it would
    /// silently produce garbage. Pinning the exact bytes is what makes the two sides agree.
    #[test]
    fn a_mismatched_info_is_refused() {
        let identity = DeviceIdentity::derive("svc", b"pair-secret-with-padding-0123456789")
            .expect("derive");
        let sealed = SealedSecret {
            alg: SEALING_ALGORITHM.to_string(),
            info: "zeus-v2".into(),
            eph_pub: b64_encode(&identity.public_key_sec1),
            nonce: b64_encode(&[0u8; NONCE_LEN]),
            ct: b64_encode(&[0u8; 32]),
        };
        assert!(matches!(
            decrypt_sealed(&identity, &sealed),
            Err(CryptoError::Malformed(_))
        ));
    }

    /// Deriving twice from the same seed must yield the same key: that is the whole point of
    /// deriving instead of generating, since Railway's filesystem is ephemeral.
    #[test]
    fn derive_is_deterministic() {
        let a = DeviceIdentity::derive("svc-1", b"pair-secret-with-padding-0123456789").unwrap();
        let b = DeviceIdentity::derive("svc-1", b"pair-secret-with-padding-0123456789").unwrap();
        assert_eq!(a.public_key_sec1, b.public_key_sec1);
        // A different service id must give a different key.
        let c = DeviceIdentity::derive("svc-2", b"pair-secret-with-padding-0123456789").unwrap();
        assert_ne!(a.public_key_sec1, c.public_key_sec1);
    }

    /// Plaintext phải bị zero. Đây là ràng buộc bảo mật duy nhất của dự án nên nó đáng có một
    /// test, và test này **không** cần V4.3.
    ///
    /// Assert trên dữ liệu còn sống. Bản cũ đọc backing buffer *sau khi* drop — đó là đọc bộ nhớ
    /// đã free, và glibc scribble tcache metadata lên 8 byte đầu, nên nó thấy rác chứ không thấy
    /// lỗi thật. `Drop` gọi `zeroize` trước khi free, nên ghi đè thật sự xảy ra; chỗ quan sát
    /// được là chính `zeroize`.
    #[test]
    fn zeroize_clears_the_plaintext_bytes() {
        let mut credentials = PlaintextCredentials {
            username: "acc".into(),
            password: "hunter2".into(),
        };
        credentials.zeroize();
        // Độ dài phải được giữ nguyên — zero mà cắt ngắn thì là một bug khác, và buffer đã cấp
        // phát vẫn còn giữ phần đuôi cũ.
        assert_eq!(credentials.password.len(), 7, "zeroize must not truncate");
        assert_eq!(credentials.username.len(), 3, "zeroize must not truncate");
        assert!(
            credentials.password.as_bytes().iter().all(|b| *b == 0),
            "mật khẩu chưa được zero: {:?}",
            credentials.password.as_bytes()
        );
        assert!(
            credentials.username.as_bytes().iter().all(|b| *b == 0),
            "username chưa được zero: {:?}",
            credentials.username.as_bytes()
        );
    }

    /// `load_or_derive` must persist the key and then prefer the stored copy. That is what makes a
    /// redeploy keep the same pairing even if `RAILWAY_SERVICE_ID` proves unstable (V6.1), so a
    /// silent regression here would cost every user a re-pair on each deploy.
    #[test]
    fn load_or_derive_persists_and_reuses_the_key() {
        let dir = std::env::temp_dir().join(format!(
            "zeus-agent-keytest-{}",
            std::process::id()
        ));
        // Unique subdir per run: the test must not reuse a key file left by an earlier failure.
        let dir = dir.join(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos().to_string()).unwrap_or_default());
        let _ = std::fs::remove_dir_all(&dir);

        let first = DeviceIdentity::load_or_derive(&dir, "svc-a", b"pair-secret-with-padding-0123456789")
            .expect("first derive");
        assert!(dir.join("device.key").exists(), "key file was not written");

        // Second call with a DIFFERENT service id must still return the stored key — the stored
        // copy wins by design.
        let second =
            DeviceIdentity::load_or_derive(&dir, "svc-completely-different", b"other-secret-0123456789")
                .expect("load stored");
        assert_eq!(
            first.public_key_sec1, second.public_key_sec1,
            "stored key must be reused across redeploys"
        );

        // A truncated key file is refused rather than silently producing a wrong key.
        std::fs::write(dir.join("device.key"), [0u8; 16]).expect("truncate");
        assert!(matches!(
            DeviceIdentity::load_or_derive(&dir, "svc-a", b"pair-secret-with-padding-0123456789"),
            Err(CryptoError::KeyUnavailable(_))
        ));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
