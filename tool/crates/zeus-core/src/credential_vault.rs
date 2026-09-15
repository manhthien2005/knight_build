use std::cell::Cell;
#[cfg(windows)]
use std::io::Read;
use std::marker::PhantomData;

use uuid::Uuid;

use crate::data_root::DataRoot;
use crate::{CoreError, CoreResult};

const CREDENTIAL_VERSION: u32 = 1;
const NONCE_BYTES: usize = 12;
const TAG_BYTES: usize = 16;
#[cfg(windows)]
const KEY_BYTES: usize = 32;
#[cfg(windows)]
const KEY_FILE_BYTES: usize = 8 + 4 + KEY_BYTES;
#[cfg(windows)]
const KEY_READ_STORAGE_BYTES: usize = KEY_FILE_BYTES + 1;
#[cfg(windows)]
const KEY_FILE_NAME: &str = "vault.key";
#[cfg(windows)]
const KEY_MAGIC: &[u8; 8] = b"ZEUSVLT1";

#[cfg(all(test, windows))]
type CleanupObservation = std::sync::Arc<std::sync::Mutex<Option<Vec<u8>>>>;

#[cfg(windows)]
struct ZeroingVec {
    bytes: Vec<u8>,
    #[cfg(test)]
    cleanup_observation: Option<CleanupObservation>,
}

#[cfg(windows)]
impl ZeroingVec {
    fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            #[cfg(test)]
            cleanup_observation: None,
        }
    }

    fn zeroed(length: usize) -> Self {
        Self::new(vec![0_u8; length])
    }

    fn bounded_key_read() -> Self {
        Self::zeroed(KEY_READ_STORAGE_BYTES)
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    fn into_vec(mut self) -> Vec<u8> {
        std::mem::take(&mut self.bytes)
    }

    #[cfg(test)]
    fn with_cleanup_observation(bytes: Vec<u8>, observation: CleanupObservation) -> Self {
        Self {
            bytes,
            cleanup_observation: Some(observation),
        }
    }

    #[cfg(test)]
    fn bounded_key_read_with_cleanup_observation(observation: CleanupObservation) -> Self {
        Self::with_cleanup_observation(vec![0_u8; KEY_READ_STORAGE_BYTES], observation)
    }
}

#[cfg(windows)]
impl Drop for ZeroingVec {
    fn drop(&mut self) {
        clear_bytes_volatile(&mut self.bytes);
        #[cfg(test)]
        if let Some(observation) = &self.cleanup_observation {
            *observation.lock().expect("cleanup observation lock") = Some(self.bytes.clone());
        }
    }
}

#[cfg(windows)]
struct KeyBytes {
    bytes: [u8; KEY_BYTES],
    #[cfg(test)]
    cleanup_observation: Option<CleanupObservation>,
}

#[cfg(windows)]
impl KeyBytes {
    fn zeroed() -> Self {
        Self {
            bytes: [0_u8; KEY_BYTES],
            #[cfg(test)]
            cleanup_observation: None,
        }
    }

    fn as_array(&self) -> &[u8; KEY_BYTES] {
        &self.bytes
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.bytes
    }

    #[cfg(test)]
    fn zeroed_with_cleanup_observation(observation: CleanupObservation) -> Self {
        Self {
            bytes: [0_u8; KEY_BYTES],
            cleanup_observation: Some(observation),
        }
    }
}

#[cfg(windows)]
impl Drop for KeyBytes {
    fn drop(&mut self) {
        clear_bytes_volatile(&mut self.bytes);
        #[cfg(test)]
        if let Some(observation) = &self.cleanup_observation {
            *observation.lock().expect("cleanup observation lock") = Some(self.bytes.to_vec());
        }
    }
}

#[cfg(windows)]
fn clear_bytes_volatile(bytes: &mut [u8]) {
    for byte in bytes {
        // SAFETY: each byte is a valid, uniquely borrowed element of its owned buffer.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EncryptedPasswordV1 {
    pub(crate) version: u32,
    pub(crate) cipher: Vec<u8>,
    pub(crate) nonce: [u8; NONCE_BYTES],
    pub(crate) tag: [u8; TAG_BYTES],
}

pub(crate) struct SecretBytes {
    bytes: Vec<u8>,
    _not_sync: PhantomData<Cell<()>>,
}

impl SecretBytes {
    pub(crate) fn new(bytes: Vec<u8>) -> Self {
        Self {
            bytes,
            _not_sync: PhantomData,
        }
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes
    }

    fn clear(&mut self) {
        for byte in &mut self.bytes {
            // SAFETY: each byte is a valid, uniquely borrowed element of this owned buffer.
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
        std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
    }

    #[cfg(test)]
    fn expose_for_test(&self) -> &[u8] {
        self.as_slice()
    }

    /// Reports whether the secret was cleared. Test-only, so no caller can probe secret length.
    #[cfg(test)]
    pub(crate) fn is_empty_for_test(&self) -> bool {
        self.bytes.is_empty()
    }

    /// Borrows the plaintext for bounds validation only. Callers must never log or copy it.
    pub(crate) fn expose_for_validation(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Drop for SecretBytes {
    fn drop(&mut self) {
        self.clear();
    }
}

pub(crate) trait CredentialCipher {
    fn encrypt(
        &self,
        account_id: Uuid,
        plaintext: &mut SecretBytes,
    ) -> CoreResult<EncryptedPasswordV1>;

    fn decrypt(&self, account_id: Uuid, value: &EncryptedPasswordV1) -> CoreResult<SecretBytes>;
}

pub(crate) enum CredentialVaultState {
    #[cfg(windows)]
    Available(PortableVault),
    #[cfg(all(test, not(windows)))]
    Test(FakeCredentialCipher),
    Unavailable,
}

impl CredentialVaultState {
    pub(crate) fn open(data_root: &DataRoot, account_rows_exist: bool) -> CoreResult<Self> {
        #[cfg(windows)]
        {
            PortableVault::open(data_root, account_rows_exist)
        }
        #[cfg(all(test, not(windows)))]
        {
            let _ = (data_root, account_rows_exist);
            Ok(Self::Test(FakeCredentialCipher { key: 0xa5 }))
        }
        #[cfg(all(not(test), not(windows)))]
        {
            let _ = (data_root, account_rows_exist);
            Ok(Self::Unavailable)
        }
    }

    /// Borrows the cipher, or reports the vault unavailable.
    ///
    /// Operations that never touch ciphertext, such as renaming an account, must not call this: spec
    /// section 8 keeps rename possible while the key is unavailable.
    pub(crate) fn cipher(&self) -> CoreResult<&dyn CredentialCipher> {
        match self {
            #[cfg(windows)]
            Self::Available(vault) => Ok(vault),
            #[cfg(all(test, not(windows)))]
            Self::Test(cipher) => Ok(cipher),
            Self::Unavailable => Err(CoreError::CredentialVaultUnavailable),
        }
    }
}

#[cfg(windows)]
pub(crate) struct PortableVault {
    key: KeyBytes,
}

#[cfg(windows)]
impl PortableVault {
    fn open(data_root: &DataRoot, account_rows_exist: bool) -> CoreResult<CredentialVaultState> {
        use std::fs;
        use std::io::ErrorKind;

        use crate::data_root::harden_existing_private_file;

        let path = data_root.path().join(KEY_FILE_NAME);
        match fs::symlink_metadata(&path) {
            Ok(_) => {
                if harden_existing_private_file(&path).is_err() {
                    return Ok(CredentialVaultState::Unavailable);
                }
                match read_key(&path) {
                    Ok(key) => Ok(CredentialVaultState::Available(Self { key })),
                    Err(_) => Ok(CredentialVaultState::Unavailable),
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound && account_rows_exist => {
                Ok(CredentialVaultState::Unavailable)
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {
                let key = create_key_atomically(data_root)?;
                Ok(CredentialVaultState::Available(Self { key }))
            }
            Err(error) => Err(CoreError::io("inspect credential key", error)),
        }
    }
}

#[cfg(windows)]
impl CredentialCipher for PortableVault {
    fn encrypt(
        &self,
        account_id: Uuid,
        plaintext: &mut SecretBytes,
    ) -> CoreResult<EncryptedPasswordV1> {
        let result = cng::encrypt(&self.key, account_id, plaintext.as_slice());
        plaintext.clear();
        result
    }

    fn decrypt(&self, account_id: Uuid, value: &EncryptedPasswordV1) -> CoreResult<SecretBytes> {
        if value.version != CREDENTIAL_VERSION {
            return Err(credential_rejected());
        }
        cng::decrypt(&self.key, account_id, value)
    }
}

#[cfg(windows)]
fn read_key(path: &std::path::Path) -> CoreResult<KeyBytes> {
    read_key_owned(path, None)
}

#[cfg(windows)]
fn read_key_owned(
    path: &std::path::Path,
    #[cfg(test)] cleanup_observation: Option<CleanupObservation>,
    #[cfg(not(test))] _cleanup_observation: Option<()>,
) -> CoreResult<KeyBytes> {
    #[cfg(test)]
    let bytes = match cleanup_observation {
        Some(observation) => ZeroingVec::bounded_key_read_with_cleanup_observation(observation),
        None => ZeroingVec::bounded_key_read(),
    };
    #[cfg(not(test))]
    let bytes = ZeroingVec::bounded_key_read();
    let file =
        std::fs::File::open(path).map_err(|error| CoreError::io("read credential key", error))?;
    read_key_from_reader_owned(file, bytes)
}

#[cfg(windows)]
fn read_key_from_reader_owned(
    mut reader: impl Read,
    mut bytes: ZeroingVec,
) -> CoreResult<KeyBytes> {
    let base_pointer = bytes.as_slice().as_ptr();
    let capacity = bytes.bytes.capacity();
    if bytes.as_slice().len() != KEY_READ_STORAGE_BYTES || capacity != KEY_READ_STORAGE_BYTES {
        return Err(credential_rejected());
    }

    let mut initialized = 0_usize;
    loop {
        if initialized == KEY_READ_STORAGE_BYTES {
            return Err(credential_rejected());
        }
        let remaining = &mut bytes.as_mut_slice()[initialized..];
        let read = match reader.read(remaining) {
            Ok(0) => break,
            Ok(read) if read <= remaining.len() => read,
            Ok(_) => return Err(credential_rejected()),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(CoreError::io("read credential key", error)),
        };
        initialized += read;
        debug_assert_eq!(bytes.as_slice().as_ptr(), base_pointer);
        debug_assert_eq!(bytes.bytes.capacity(), capacity);
    }

    let stored = &bytes.as_slice()[..initialized];
    if initialized != KEY_FILE_BYTES
        || &stored[..8] != KEY_MAGIC
        || stored[8..12] != CREDENTIAL_VERSION.to_le_bytes()
    {
        return Err(credential_rejected());
    }
    let mut key = KeyBytes::zeroed();
    key.as_mut_slice().copy_from_slice(&stored[12..]);
    Ok(key)
}

#[cfg(all(test, windows))]
fn read_key_from_reader_with_cleanup_probe(
    reader: impl Read,
    cleanup_observation: CleanupObservation,
) -> CoreResult<KeyBytes> {
    read_key_from_reader_owned(
        reader,
        ZeroingVec::bounded_key_read_with_cleanup_observation(cleanup_observation),
    )
}

#[cfg(all(test, windows))]
fn read_key_with_cleanup_probe(
    path: &std::path::Path,
    cleanup_observation: CleanupObservation,
) -> CoreResult<KeyBytes> {
    read_key_owned(path, Some(cleanup_observation))
}

#[cfg(windows)]
fn create_key_atomically(data_root: &DataRoot) -> CoreResult<KeyBytes> {
    create_key_atomically_owned(data_root, None)
}

#[cfg(windows)]
fn create_key_atomically_owned(
    data_root: &DataRoot,
    #[cfg(test)] cleanup_observation: Option<CleanupObservation>,
    #[cfg(not(test))] _cleanup_observation: Option<()>,
) -> CoreResult<KeyBytes> {
    use std::fs;
    use std::io::Write;

    use crate::data_root::create_private_truncated_file;

    #[cfg(test)]
    let mut key = match cleanup_observation {
        Some(observation) => KeyBytes::zeroed_with_cleanup_observation(observation),
        None => KeyBytes::zeroed(),
    };
    #[cfg(not(test))]
    let mut key = KeyBytes::zeroed();
    cng::system_random(key.as_mut_slice())?;
    let temp_name = format!(".vault.key.tmp-{}", Uuid::new_v4());
    let temp_path = data_root.path().join(temp_name);
    let destination = data_root.path().join(KEY_FILE_NAME);
    let publish: CoreResult<()> = (|| {
        let mut file = create_private_truncated_file(&temp_path)?;
        file.write_all(KEY_MAGIC)
            .and_then(|()| file.write_all(&CREDENTIAL_VERSION.to_le_bytes()))
            .and_then(|()| file.write_all(key.as_slice()))
            .and_then(|()| file.sync_all())
            .map_err(|error| CoreError::io("write credential key", error))?;
        drop(file);
        fs::rename(&temp_path, &destination)
            .map_err(|error| CoreError::io("publish credential key", error))?;
        Ok(())
    })();
    if publish.is_err() {
        let _ = fs::remove_file(&temp_path);
    }
    publish?;
    Ok(key)
}

#[cfg(all(test, windows))]
fn create_key_atomically_with_cleanup_probe(
    data_root: &DataRoot,
    cleanup_observation: CleanupObservation,
) -> CoreResult<KeyBytes> {
    create_key_atomically_owned(data_root, Some(cleanup_observation))
}

fn credential_rejected() -> CoreError {
    CoreError::RuntimeValidation {
        code: "CredentialUnavailable",
    }
}

#[cfg(test)]
#[allow(
    dead_code,
    reason = "opaque non-production cipher for cross-platform Core tests"
)]
pub(crate) struct FakeCredentialCipher {
    key: u8,
}

#[cfg(test)]
impl CredentialCipher for FakeCredentialCipher {
    fn encrypt(
        &self,
        account_id: Uuid,
        plaintext: &mut SecretBytes,
    ) -> CoreResult<EncryptedPasswordV1> {
        let nonce = [self.key; NONCE_BYTES];
        let cipher = plaintext
            .as_slice()
            .iter()
            .map(|byte| byte ^ self.key)
            .collect::<Vec<_>>();
        let mut tag = [0_u8; TAG_BYTES];
        tag.copy_from_slice(account_id.as_bytes());
        plaintext.clear();
        Ok(EncryptedPasswordV1 {
            version: CREDENTIAL_VERSION,
            cipher,
            nonce,
            tag,
        })
    }

    fn decrypt(&self, account_id: Uuid, value: &EncryptedPasswordV1) -> CoreResult<SecretBytes> {
        if value.version != CREDENTIAL_VERSION
            || value.nonce != [self.key; NONCE_BYTES]
            || value.tag != *account_id.as_bytes()
        {
            return Err(credential_rejected());
        }
        Ok(SecretBytes::new(
            value.cipher.iter().map(|byte| byte ^ self.key).collect(),
        ))
    }
}

#[cfg(windows)]
mod cng {
    use std::ffi::c_void;
    use std::io;
    use std::ptr;

    use uuid::Uuid;
    use windows_sys::Win32::Security::Cryptography::{
        BCRYPT_AES_ALGORITHM, BCRYPT_ALG_HANDLE, BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO,
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION, BCRYPT_CHAINING_MODE, BCRYPT_KEY_HANDLE,
        BCRYPT_OBJECT_LENGTH, BCRYPT_USE_SYSTEM_PREFERRED_RNG, BCryptCloseAlgorithmProvider,
        BCryptDecrypt, BCryptDestroyKey, BCryptEncrypt, BCryptGenRandom,
        BCryptGenerateSymmetricKey, BCryptGetProperty, BCryptOpenAlgorithmProvider,
        BCryptSetProperty,
    };

    use super::{
        CREDENTIAL_VERSION, EncryptedPasswordV1, KEY_BYTES, KeyBytes, NONCE_BYTES, SecretBytes,
        TAG_BYTES, ZeroingVec, credential_rejected,
    };
    use crate::{CoreError, CoreResult};

    pub(super) fn system_random(output: &mut [u8]) -> CoreResult<()> {
        let length = u32::try_from(output.len()).map_err(|_| credential_rejected())?;
        // SAFETY: CNG receives a valid writable output buffer for exactly `length` bytes.
        nt_success(
            unsafe {
                BCryptGenRandom(
                    ptr::null_mut(),
                    output.as_mut_ptr(),
                    length,
                    BCRYPT_USE_SYSTEM_PREFERRED_RNG,
                )
            },
            "generate credential randomness",
        )
    }

    pub(super) fn encrypt(
        key: &KeyBytes,
        account_id: Uuid,
        plaintext: &[u8],
    ) -> CoreResult<EncryptedPasswordV1> {
        let mut nonce = [0_u8; NONCE_BYTES];
        system_random(&mut nonce)?;
        let mut tag = [0_u8; TAG_BYTES];
        let aad = aad(account_id);
        let mut cipher = vec![0_u8; plaintext.len()];
        with_key(key, |handle| {
            let mut info = auth_info(&mut nonce, &aad, &mut tag);
            let mut written = 0_u32;
            // SAFETY: the key handle is live, all input/output buffers match their lengths, and
            // authenticated mode info points to nonce/AAD/tag buffers live for the entire call.
            nt_success(
                unsafe {
                    BCryptEncrypt(
                        handle,
                        plaintext.as_ptr(),
                        u32::try_from(plaintext.len()).map_err(|_| credential_rejected())?,
                        (&mut info as *mut BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO).cast::<c_void>(),
                        ptr::null_mut(),
                        0,
                        cipher.as_mut_ptr(),
                        u32::try_from(cipher.len()).map_err(|_| credential_rejected())?,
                        &mut written,
                        0,
                    )
                },
                "encrypt account credential",
            )?;
            if written as usize != cipher.len() {
                return Err(credential_rejected());
            }
            Ok(())
        })?;
        Ok(EncryptedPasswordV1 {
            version: CREDENTIAL_VERSION,
            cipher,
            nonce,
            tag,
        })
    }

    pub(super) fn decrypt(
        key: &KeyBytes,
        account_id: Uuid,
        value: &EncryptedPasswordV1,
    ) -> CoreResult<SecretBytes> {
        decrypt_into(
            key,
            account_id,
            value,
            ZeroingVec::zeroed(value.cipher.len()),
        )
    }

    #[cfg(test)]
    pub(super) fn decrypt_with_cleanup_probe(
        key: &KeyBytes,
        account_id: Uuid,
        value: &EncryptedPasswordV1,
        cleanup_observation: super::CleanupObservation,
    ) -> CoreResult<SecretBytes> {
        decrypt_into(
            key,
            account_id,
            value,
            ZeroingVec::with_cleanup_observation(
                vec![0xa5; value.cipher.len()],
                cleanup_observation,
            ),
        )
    }

    fn decrypt_into(
        key: &KeyBytes,
        account_id: Uuid,
        value: &EncryptedPasswordV1,
        mut plaintext: ZeroingVec,
    ) -> CoreResult<SecretBytes> {
        let mut nonce = value.nonce;
        let mut tag = value.tag;
        let aad = aad(account_id);
        with_key(key, |handle| {
            let mut info = auth_info(&mut nonce, &aad, &mut tag);
            let mut written = 0_u32;
            // SAFETY: the key handle is live, all input/output buffers match their lengths, and
            // authenticated mode info points to nonce/AAD/tag buffers live for the entire call.
            nt_success(
                unsafe {
                    BCryptDecrypt(
                        handle,
                        value.cipher.as_ptr(),
                        u32::try_from(value.cipher.len()).map_err(|_| credential_rejected())?,
                        (&mut info as *mut BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO).cast::<c_void>(),
                        ptr::null_mut(),
                        0,
                        plaintext.as_mut_slice().as_mut_ptr(),
                        u32::try_from(plaintext.as_slice().len())
                            .map_err(|_| credential_rejected())?,
                        &mut written,
                        0,
                    )
                },
                "decrypt account credential",
            )?;
            if written as usize != plaintext.as_slice().len() {
                return Err(credential_rejected());
            }
            Ok(())
        })?;
        Ok(SecretBytes::new(plaintext.into_vec()))
    }

    fn aad(account_id: Uuid) -> [u8; 17] {
        let mut value = [0_u8; 17];
        value[0] = CREDENTIAL_VERSION as u8;
        value[1..].copy_from_slice(account_id.as_bytes());
        value
    }

    fn auth_info(
        nonce: &mut [u8; NONCE_BYTES],
        aad: &[u8; 17],
        tag: &mut [u8; TAG_BYTES],
    ) -> BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
        BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO {
            cbSize: size_of::<BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO>() as u32,
            dwInfoVersion: BCRYPT_AUTHENTICATED_CIPHER_MODE_INFO_VERSION,
            pbNonce: nonce.as_mut_ptr(),
            cbNonce: NONCE_BYTES as u32,
            pbAuthData: aad.as_ptr().cast_mut(),
            cbAuthData: aad.len() as u32,
            pbTag: tag.as_mut_ptr(),
            cbTag: TAG_BYTES as u32,
            pbMacContext: ptr::null_mut(),
            cbMacContext: 0,
            cbAAD: 0,
            cbData: 0,
            dwFlags: 0,
        }
    }

    fn with_key<T>(
        key: &KeyBytes,
        action: impl FnOnce(BCRYPT_KEY_HANDLE) -> CoreResult<T>,
    ) -> CoreResult<T> {
        let algorithm = AlgorithmHandle::open()?;
        algorithm.enable_gcm()?;
        let object_length = algorithm.object_length()?;
        let mut key_object = ZeroingVec::zeroed(object_length);
        let key_handle =
            KeyHandle::generate(algorithm.0, key_object.as_mut_slice(), key.as_array())?;
        action(key_handle.0)
    }

    struct AlgorithmHandle(BCRYPT_ALG_HANDLE);

    impl AlgorithmHandle {
        fn open() -> CoreResult<Self> {
            let mut owned = Self(ptr::null_mut());
            // SAFETY: `owned.0` is writable and the algorithm/provider identifiers are static.
            nt_success(
                unsafe {
                    BCryptOpenAlgorithmProvider(&mut owned.0, BCRYPT_AES_ALGORITHM, ptr::null(), 0)
                },
                "open AES provider",
            )?;
            Ok(owned)
        }

        fn enable_gcm(&self) -> CoreResult<()> {
            let mode = "ChainingModeGCM\0".encode_utf16().collect::<Vec<_>>();
            // SAFETY: the provider handle is live and `mode` is a NUL-terminated UTF-16 buffer.
            nt_success(
                unsafe {
                    BCryptSetProperty(
                        self.0,
                        BCRYPT_CHAINING_MODE,
                        mode.as_ptr().cast::<u8>(),
                        u32::try_from(mode.len() * 2).map_err(|_| credential_rejected())?,
                        0,
                    )
                },
                "select AES-GCM",
            )
        }

        fn object_length(&self) -> CoreResult<usize> {
            let mut length = 0_u32;
            let mut written = 0_u32;
            // SAFETY: the provider handle is live and `length` is a four-byte output buffer.
            nt_success(
                unsafe {
                    BCryptGetProperty(
                        self.0,
                        BCRYPT_OBJECT_LENGTH,
                        (&mut length as *mut u32).cast::<u8>(),
                        size_of::<u32>() as u32,
                        &mut written,
                        0,
                    )
                },
                "read AES key-object length",
            )?;
            if written != size_of::<u32>() as u32 || length == 0 {
                return Err(credential_rejected());
            }
            Ok(length as usize)
        }
    }

    impl Drop for AlgorithmHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: this wrapper uniquely owns the provider handle.
                let _ = unsafe { BCryptCloseAlgorithmProvider(self.0, 0) };
            }
        }
    }

    struct KeyHandle(BCRYPT_KEY_HANDLE);

    impl KeyHandle {
        fn generate(
            algorithm: BCRYPT_ALG_HANDLE,
            object: &mut [u8],
            key: &[u8; KEY_BYTES],
        ) -> CoreResult<Self> {
            let mut owned = Self(ptr::null_mut());
            // SAFETY: the provider is live; object and secret buffers match their lengths; the
            // returned handle is uniquely transferred into `KeyHandle`.
            nt_success(
                unsafe {
                    BCryptGenerateSymmetricKey(
                        algorithm,
                        &mut owned.0,
                        object.as_mut_ptr(),
                        u32::try_from(object.len()).map_err(|_| credential_rejected())?,
                        key.as_ptr(),
                        KEY_BYTES as u32,
                        0,
                    )
                },
                "create AES key",
            )?;
            Ok(owned)
        }
    }

    impl Drop for KeyHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: this wrapper uniquely owns the key handle.
                let _ = unsafe { BCryptDestroyKey(self.0) };
            }
        }
    }

    fn nt_success(status: i32, operation: &'static str) -> CoreResult<()> {
        if status >= 0 {
            Ok(())
        } else {
            Err(CoreError::io(
                operation,
                io::Error::from_raw_os_error(status),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(windows)]
    use std::error::Error;
    use std::fs;
    #[cfg(windows)]
    use std::io::{self, Read, Write};
    use std::path::PathBuf;
    #[cfg(windows)]
    use std::sync::{Arc, Mutex};

    use uuid::Uuid;

    use super::{CredentialCipher, CredentialVaultState, SecretBytes};
    #[cfg(windows)]
    use super::{EncryptedPasswordV1, PortableVault};
    use crate::data_root::DataRoot;

    const SENTINEL: &[u8] = b"VaultSentinel-7!";

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            Self(std::env::temp_dir().join(format!("zeus-{label}-{}", Uuid::new_v4())))
        }

        fn data_root(&self) -> DataRoot {
            DataRoot::prepare_at(&self.0).expect("prepare private data root")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            if self.0.starts_with(std::env::temp_dir()) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }
    }

    fn secret() -> SecretBytes {
        SecretBytes::new(SENTINEL.to_vec())
    }

    #[cfg(windows)]
    fn encrypted(vault: &PortableVault, account_id: Uuid) -> EncryptedPasswordV1 {
        vault
            .encrypt(account_id, &mut secret())
            .expect("encrypt sentinel")
    }

    mod secret_ownership {
        use super::SecretBytes;

        struct MustNotSync;
        struct MustNotClone;
        trait OwnershipProof<Forbidden> {
            fn proof() {}
        }
        impl<T: ?Sized> OwnershipProof<()> for T {}
        impl<T: ?Sized + Sync> OwnershipProof<MustNotSync> for T {}
        impl<T: Clone> OwnershipProof<MustNotClone> for T {}

        fn assert_send<T: Send>() {}

        pub(super) fn prove() {
            assert_send::<SecretBytes>();
            let _ = <SecretBytes as OwnershipProof<_>>::proof;
        }
    }

    #[test]
    fn credential_vault_secret_bytes_are_send_never_sync_noncloneable_and_clear_in_place() {
        secret_ownership::prove();
        let mut secret = secret();
        secret.clear();
        assert!(secret.expose_for_test().iter().all(|byte| *byte == 0));
    }

    #[cfg(windows)]
    fn cleanup_observation() -> Arc<Mutex<Option<Vec<u8>>>> {
        Arc::new(Mutex::new(None))
    }

    #[cfg(windows)]
    fn assert_observed_zeroes(observation: &Arc<Mutex<Option<Vec<u8>>>>, expected_len: usize) {
        let observed = observation
            .lock()
            .expect("cleanup observation lock")
            .clone()
            .expect("zero-on-drop cleanup was not observed");
        assert_eq!(observed.len(), expected_len);
        assert!(observed.iter().all(|byte| *byte == 0));
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_decrypt_authentication_error_clears_destination_before_deallocation() {
        let directory = TestDirectory::new("vault-decrypt-cleanup");
        let root = directory.data_root();
        let CredentialVaultState::Available(vault) =
            CredentialVaultState::open(&root, false).expect("open vault")
        else {
            panic!("fresh vault unavailable")
        };
        let account_id = Uuid::new_v4();
        let mut value = encrypted(&vault, account_id);
        value.tag[0] ^= 1;
        let observation = cleanup_observation();

        assert!(
            super::cng::decrypt_with_cleanup_probe(
                &vault.key,
                account_id,
                &value,
                Arc::clone(&observation),
            )
            .is_err()
        );
        assert_observed_zeroes(&observation, value.cipher.len());
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_read_clears_full_file_buffer_on_success_and_validation_error() {
        let directory = TestDirectory::new("vault-read-cleanup");
        let root = directory.data_root();
        let state = CredentialVaultState::open(&root, false).expect("create vault key");
        drop(state);
        let path = directory.0.join("vault.key");

        let success_observation = cleanup_observation();
        let key = super::read_key_with_cleanup_probe(&path, Arc::clone(&success_observation))
            .expect("read valid key");
        assert_observed_zeroes(&success_observation, super::KEY_READ_STORAGE_BYTES);
        drop(key);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open key for overflow fixture")
            .write_all(&[0xa5])
            .expect("append overflow byte");
        let overflow_observation = cleanup_observation();
        assert!(
            super::read_key_with_cleanup_probe(&path, Arc::clone(&overflow_observation)).is_err()
        );
        assert_observed_zeroes(&overflow_observation, super::KEY_READ_STORAGE_BYTES);

        fs::write(&path, b"malformed-key").expect("write malformed key");
        let error_observation = cleanup_observation();
        assert!(super::read_key_with_cleanup_probe(&path, Arc::clone(&error_observation)).is_err());
        assert_observed_zeroes(&error_observation, super::KEY_READ_STORAGE_BYTES);
    }

    #[cfg(windows)]
    struct MultiChunkKeyThenError {
        bytes: &'static [u8],
        delivered: usize,
        chunk_size: usize,
        allocations: Arc<Mutex<Vec<(usize, usize)>>>,
    }

    #[cfg(windows)]
    impl Read for MultiChunkKeyThenError {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            let base_pointer = (output.as_ptr() as usize)
                .checked_sub(self.delivered)
                .expect("remaining slice begins inside one allocation");
            self.allocations
                .lock()
                .expect("allocation observation lock")
                .push((base_pointer, output.len() + self.delivered));
            if self.delivered == self.bytes.len() {
                return Err(io::Error::other("injected partial key read failure"));
            }
            let remaining = &self.bytes[self.delivered..];
            let length = output.len().min(remaining.len()).min(self.chunk_size);
            output[..length].copy_from_slice(&remaining[..length]);
            self.delivered += length;
            Ok(length)
        }
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_multi_chunk_key_read_never_grows_and_clears_initialized_bytes_on_error() {
        let partial = b"ZEUSVLT1\x01\x00\x00\x00abcdefghijklmnopqrstuvwxyz012345";
        assert_eq!(partial.len(), 44);
        let observation = cleanup_observation();
        let allocations = Arc::new(Mutex::new(Vec::new()));
        let reader = MultiChunkKeyThenError {
            bytes: partial,
            delivered: 0,
            chunk_size: 7,
            allocations: Arc::clone(&allocations),
        };

        assert!(
            super::read_key_from_reader_with_cleanup_probe(reader, Arc::clone(&observation),)
                .is_err()
        );
        let allocations = allocations.lock().expect("allocation observation lock");
        assert!(allocations.len() > 2, "reader must deliver multiple chunks");
        assert_eq!(
            allocations[0].1, 45,
            "guard must allocate exact 44+1 storage"
        );
        assert!(allocations.iter().all(|sample| *sample == allocations[0]));
        drop(allocations);
        assert_observed_zeroes(&observation, 45);
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_generated_key_clears_after_success_and_publication_failure() {
        let success_directory = TestDirectory::new("vault-generate-cleanup-success");
        let success_root = success_directory.data_root();
        let success_observation = cleanup_observation();
        let key = super::create_key_atomically_with_cleanup_probe(
            &success_root,
            Arc::clone(&success_observation),
        )
        .expect("publish generated key");
        assert!(
            success_observation
                .lock()
                .expect("success observation lock")
                .is_none()
        );
        drop(key);
        assert_observed_zeroes(&success_observation, super::KEY_BYTES);

        let failure_directory = TestDirectory::new("vault-generate-cleanup-failure");
        let failure_root = failure_directory.data_root();
        fs::create_dir(failure_directory.0.join("vault.key"))
            .expect("occupy destination with directory");
        let failure_observation = cleanup_observation();
        assert!(
            super::create_key_atomically_with_cleanup_probe(
                &failure_root,
                Arc::clone(&failure_observation),
            )
            .is_err()
        );
        assert_observed_zeroes(&failure_observation, super::KEY_BYTES);
    }

    #[cfg(not(windows))]
    #[test]
    fn credential_vault_unit_tests_use_only_the_opaque_fake_cipher() {
        let directory = TestDirectory::new("vault-fake");
        let root = directory.data_root();
        let CredentialVaultState::Test(cipher) =
            CredentialVaultState::open(&root, false).expect("open test vault")
        else {
            panic!("unit-test vault did not select opaque fake")
        };
        let account_id = Uuid::new_v4();
        let value = cipher
            .encrypt(account_id, &mut secret())
            .expect("fake encrypt");
        let plaintext = cipher.decrypt(account_id, &value).expect("fake decrypt");
        assert_eq!(plaintext.expose_for_test(), SENTINEL);
        assert!(!directory.0.join("vault.key").exists());
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_key_has_exact_layout_and_atomic_publish() {
        let directory = TestDirectory::new("vault-key-layout");
        let root = directory.data_root();
        let state = CredentialVaultState::open(&root, false).expect("open fresh vault");
        assert!(matches!(state, CredentialVaultState::Available(_)));

        let bytes = fs::read(directory.0.join("vault.key")).expect("read vault key");
        assert_eq!(bytes.len(), 44);
        assert_eq!(&bytes[..8], b"ZEUSVLT1");
        assert_eq!(&bytes[8..12], &1_u32.to_le_bytes());
        assert!(bytes[12..].iter().any(|byte| *byte != 0));
        assert!(
            fs::read_dir(&directory.0)
                .expect("read data root")
                .all(|entry| !entry
                    .expect("directory entry")
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".vault.key.tmp-"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_never_replaces_a_missing_key_when_account_rows_exist() {
        let directory = TestDirectory::new("vault-no-replacement");
        let root = directory.data_root();
        let state = CredentialVaultState::open(&root, true).expect("inspect missing vault");
        assert!(matches!(state, CredentialVaultState::Unavailable));
        assert!(!directory.0.join("vault.key").exists());
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_uses_unique_nonce_separate_tag_and_no_padding() {
        let directory = TestDirectory::new("vault-shape");
        let root = directory.data_root();
        let CredentialVaultState::Available(vault) =
            CredentialVaultState::open(&root, false).expect("open vault")
        else {
            panic!("fresh vault unavailable")
        };
        let account_id = Uuid::new_v4();
        let first = encrypted(&vault, account_id);
        let second = encrypted(&vault, account_id);
        assert_eq!(first.version, 1);
        assert_eq!(first.nonce.len(), 12);
        assert_eq!(first.tag.len(), 16);
        assert_eq!(first.cipher.len(), SENTINEL.len());
        assert_ne!(first.nonce, second.nonce);
        assert_ne!(first.cipher, second.cipher);
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_rejects_wrong_key_aad_nonce_tag_and_cipher() {
        let first_directory = TestDirectory::new("vault-tamper-a");
        let first_root = first_directory.data_root();
        let CredentialVaultState::Available(first_vault) =
            CredentialVaultState::open(&first_root, false).expect("open first vault")
        else {
            panic!("first vault unavailable")
        };
        let second_directory = TestDirectory::new("vault-tamper-b");
        let second_root = second_directory.data_root();
        let CredentialVaultState::Available(second_vault) =
            CredentialVaultState::open(&second_root, false).expect("open second vault")
        else {
            panic!("second vault unavailable")
        };
        let account_id = Uuid::new_v4();
        let original = encrypted(&first_vault, account_id);

        assert!(
            second_vault.decrypt(account_id, &original).is_err(),
            "wrong key accepted"
        );
        assert!(
            first_vault.decrypt(Uuid::new_v4(), &original).is_err(),
            "wrong AAD accepted"
        );

        let mut wrong_nonce = original.clone();
        wrong_nonce.nonce[0] ^= 1;
        assert!(
            first_vault.decrypt(account_id, &wrong_nonce).is_err(),
            "wrong nonce accepted"
        );
        let mut wrong_tag = original.clone();
        wrong_tag.tag[0] ^= 1;
        assert!(
            first_vault.decrypt(account_id, &wrong_tag).is_err(),
            "wrong tag accepted"
        );
        let mut wrong_cipher = original.clone();
        wrong_cipher.cipher[0] ^= 1;
        assert!(
            first_vault.decrypt(account_id, &wrong_cipher).is_err(),
            "wrong cipher accepted"
        );

        let error = match first_vault.decrypt(account_id, &wrong_tag) {
            Ok(_) => panic!("wrong tag accepted"),
            Err(error) => error,
        };
        let rendered = format!("{error:#?}{error}");
        assert!(
            !rendered
                .as_bytes()
                .windows(SENTINEL.len())
                .any(|bytes| bytes == SENTINEL)
        );
        if let Some(source) = error.source() {
            assert!(
                !source
                    .to_string()
                    .as_bytes()
                    .windows(SENTINEL.len())
                    .any(|bytes| bytes == SENTINEL)
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn credential_vault_copy_with_matching_key_decrypts_but_missing_foreign_and_corrupt_do_not() {
        let source_directory = TestDirectory::new("vault-copy-source");
        let source_root = source_directory.data_root();
        let CredentialVaultState::Available(source_vault) =
            CredentialVaultState::open(&source_root, false).expect("open source vault")
        else {
            panic!("source vault unavailable")
        };
        let account_id = Uuid::new_v4();
        let value = encrypted(&source_vault, account_id);

        let copied_directory = TestDirectory::new("vault-copy-target");
        let copied_root = copied_directory.data_root();
        fs::copy(
            source_directory.0.join("vault.key"),
            copied_directory.0.join("vault.key"),
        )
        .expect("copy vault key");
        let CredentialVaultState::Available(copied_vault) =
            CredentialVaultState::open(&copied_root, true).expect("open copied vault")
        else {
            panic!("copied matching vault unavailable")
        };
        let decrypted = copied_vault
            .decrypt(account_id, &value)
            .expect("decrypt copied credential");
        assert_eq!(decrypted.expose_for_test(), SENTINEL);

        fs::remove_file(copied_directory.0.join("vault.key")).expect("remove copied key");
        assert!(matches!(
            CredentialVaultState::open(&copied_root, true).expect("inspect missing key"),
            CredentialVaultState::Unavailable
        ));

        fs::write(copied_directory.0.join("vault.key"), b"corrupt").expect("write corrupt key");
        assert!(matches!(
            CredentialVaultState::open(&copied_root, true).expect("inspect corrupt key"),
            CredentialVaultState::Unavailable
        ));

        fs::copy(
            source_directory.0.join("vault.key"),
            copied_directory.0.join("vault.key"),
        )
        .expect("restore copied key");
        let mut foreign = fs::read(copied_directory.0.join("vault.key")).expect("read key");
        foreign[12] ^= 1;
        fs::write(copied_directory.0.join("vault.key"), foreign).expect("write foreign key");
        let CredentialVaultState::Available(foreign_vault) =
            CredentialVaultState::open(&copied_root, true).expect("open foreign vault")
        else {
            panic!("well-formed foreign key should load")
        };
        assert!(foreign_vault.decrypt(account_id, &value).is_err());
    }
}
