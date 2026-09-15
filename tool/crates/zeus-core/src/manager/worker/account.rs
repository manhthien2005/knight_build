use std::fmt;

use crate::credential_vault::SecretBytes;

use super::{
    ManagerWorkerError, ManagerWorkerErrorCode, ManagerWorkerOperation, ManagerWorkerResult,
};

const PASSWORD_MAX_BYTES: usize = 128;
const REDACTION: &str = "ManagerAccountPassword([REDACTED])";

/// Single-owner password accepted by account mutation operations.
pub struct ManagerAccountPassword {
    secret: SecretBytes,
}

impl ManagerAccountPassword {
    pub fn try_from_utf16(
        operation: ManagerWorkerOperation,
        mut value: Vec<u16>,
    ) -> ManagerWorkerResult<Self> {
        if !matches!(
            operation,
            ManagerWorkerOperation::ImportAccount | ManagerWorkerOperation::UpdateAccount
        ) {
            clear_utf16(&mut value);
            return Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::InvalidInput,
                operation,
            ));
        }

        match take_ascii_and_clear_utf16(&mut value) {
            Ok(bytes) => Ok(Self {
                secret: SecretBytes::new(bytes),
            }),
            Err(PasswordInputError::TooLong) => Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::InputTooLong,
                operation,
            )
            .with_maximum(PASSWORD_MAX_BYTES as u32)),
            Err(PasswordInputError::Invalid) => Err(ManagerWorkerError::new(
                ManagerWorkerErrorCode::InvalidInput,
                operation,
            )),
        }
    }

    /// Consumes the single-owner password, handing the secret to the repository.
    pub(crate) fn into_secret(self) -> SecretBytes {
        self.secret
    }
}

impl fmt::Debug for ManagerAccountPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTION)
    }
}

impl fmt::Display for ManagerAccountPassword {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(REDACTION)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PasswordInputError {
    Invalid,
    TooLong,
}

fn take_ascii_and_clear_utf16(value: &mut [u16]) -> Result<Vec<u8>, PasswordInputError> {
    let mut ascii = Vec::with_capacity(value.len().min(PASSWORD_MAX_BYTES));
    let result = if value.is_empty() {
        Err(PasswordInputError::Invalid)
    } else if value.len() > PASSWORD_MAX_BYTES {
        Err(PasswordInputError::TooLong)
    } else {
        for unit in value.iter().copied() {
            if !(0x20..=0x7e).contains(&unit) {
                clear_bytes(&mut ascii);
                clear_utf16(value);
                return Err(PasswordInputError::Invalid);
            }
            ascii.push(unit as u8);
        }
        Ok(ascii)
    };
    clear_utf16(value);
    result
}

fn clear_utf16(value: &mut [u16]) {
    for unit in value {
        // SAFETY: `unit` is a valid, uniquely borrowed element of the owned source buffer.
        unsafe { std::ptr::write_volatile(unit, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

fn clear_bytes(value: &mut [u8]) {
    for byte in value {
        // SAFETY: `byte` is a valid, uniquely borrowed element of the owned secret buffer.
        unsafe { std::ptr::write_volatile(byte, 0) };
    }
    std::sync::atomic::compiler_fence(std::sync::atomic::Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::ManagerAccountPassword;
    use crate::manager::worker::{ManagerWorkerErrorCode, ManagerWorkerOperation};
    use std::error::Error;

    const SENTINEL: &str = "ManagerSecret-83!";

    #[test]
    fn manager_account_password_accepts_only_account_mutation_operations_and_printable_ascii() {
        for operation in [
            ManagerWorkerOperation::ImportAccount,
            ManagerWorkerOperation::UpdateAccount,
        ] {
            for value in ["!", SENTINEL, &"~".repeat(128)] {
                assert!(
                    ManagerAccountPassword::try_from_utf16(
                        operation,
                        value.encode_utf16().collect()
                    )
                    .is_ok()
                );
            }
        }

        for value in ["", "line\nbreak", "é", &"x".repeat(129)] {
            let error = ManagerAccountPassword::try_from_utf16(
                ManagerWorkerOperation::ImportAccount,
                value.encode_utf16().collect(),
            )
            .unwrap_err();
            assert!(matches!(
                error.code(),
                ManagerWorkerErrorCode::InvalidInput | ManagerWorkerErrorCode::InputTooLong
            ));
            if !value.is_empty() {
                assert!(!format!("{error:?}{error}").contains(value));
            }
        }

        let error = ManagerAccountPassword::try_from_utf16(
            ManagerWorkerOperation::StartProfile,
            SENTINEL.encode_utf16().collect(),
        )
        .unwrap_err();
        assert_eq!(error.code(), ManagerWorkerErrorCode::InvalidInput);
        assert!(!format!("{error:?}{error}").contains(SENTINEL));
        assert!(error.source().is_none());
    }

    #[test]
    fn manager_account_password_debug_and_display_are_fixed_redactions() {
        let secret = ManagerAccountPassword::try_from_utf16(
            ManagerWorkerOperation::ImportAccount,
            SENTINEL.encode_utf16().collect(),
        )
        .expect("valid password");
        assert_eq!(format!("{secret:?}"), "ManagerAccountPassword([REDACTED])");
        assert_eq!(format!("{secret}"), "ManagerAccountPassword([REDACTED])");
        assert!(!format!("{secret:?}{secret}").contains(SENTINEL));
    }

    #[test]
    fn manager_account_password_clears_utf16_and_owned_ascii_buffers() {
        let mut utf16 = SENTINEL.encode_utf16().collect::<Vec<_>>();
        let ascii = super::take_ascii_and_clear_utf16(&mut utf16).expect("convert sentinel");
        assert!(utf16.iter().all(|unit| *unit == 0));
        let mut owned = ascii;
        super::clear_bytes(&mut owned);
        assert!(owned.iter().all(|byte| *byte == 0));

        let mut invalid = vec![b'a' as u16, 0x00E9];
        assert!(super::take_ascii_and_clear_utf16(&mut invalid).is_err());
        assert!(invalid.iter().all(|unit| *unit == 0));
    }
}
