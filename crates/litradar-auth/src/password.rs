//! Versioned password hashing with legacy PBKDF2 compatibility.

use std::error::Error;
use std::fmt;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version, ARGON2ID_IDENT};
use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// PBKDF2 iteration count used by the legacy Python backend.
pub const PBKDF2_ITERATIONS: u32 = 260_000;
/// Argon2id memory cost in KiB.
pub const ARGON2_MEMORY_KIB: u32 = 19_456;
/// Argon2id time cost.
pub const ARGON2_TIME_COST: u32 = 2;
/// Argon2id parallelism cost.
pub const ARGON2_PARALLELISM: u32 = 1;
/// Minimum character count for newly created or replaced passwords.
pub const MIN_PASSWORD_LENGTH: usize = 12;

const HASH_BYTES: usize = 32;
const ARGON2_SALT_BYTES: usize = 16;
#[cfg(test)]
const DUMMY_PASSWORD: &str = "litradar-auth-dummy-password";
#[cfg(test)]
const DUMMY_PASSWORD_SALT: &[u8; ARGON2_SALT_BYTES] = b"litradar-dummy!!";
const DUMMY_PASSWORD_PHC: &str = "$argon2id$v=19$m=19456,t=2,p=1$bGl0cmFkYXItZHVtbXkhIQ$ZYJd4/sBFG46tq7/498a19Zgzc/4MglD6AaOL5G1bdM";

/// Failure while creating a new password hash.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordError {
    /// Operating-system cryptographic randomness was unavailable.
    EntropyUnavailable,
    /// The configured Argon2id operation failed.
    HashingFailed,
}

impl fmt::Display for PasswordError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EntropyUnavailable => "Operating-system cryptographic randomness is unavailable",
            Self::HashingFailed => "Password hashing failed",
        })
    }
}

impl Error for PasswordError {}

/// Result of checking a password against a stored hash representation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordVerification {
    /// Password did not match or the stored representation was invalid.
    Invalid,
    /// Password matched the current Argon2id PHC representation.
    ValidCurrent,
    /// Password matched the legacy PBKDF2 hex representation.
    ValidLegacy,
}

/// Return whether a new password satisfies the current creation policy.
///
/// Existing stored passwords are intentionally not checked by this function during login.
///
/// # Arguments
///
/// * `password` - Proposed new password.
///
/// # Returns
///
/// True when the password contains at least the required number of Unicode characters.
pub fn is_valid_new_password(password: &str) -> bool {
    password.chars().count() >= MIN_PASSWORD_LENGTH
}

/// Hash a password as an Argon2id PHC string with a fresh OS-generated salt.
///
/// # Arguments
///
/// * `password` - Plain-text password.
///
/// # Returns
///
/// Versioned PHC string using the declared Argon2id parameters.
pub fn hash_password(password: &str) -> Result<String, PasswordError> {
    let mut salt = [0_u8; ARGON2_SALT_BYTES];
    getrandom::fill(&mut salt).map_err(|_| PasswordError::EntropyUnavailable)?;
    hash_password_with_salt(password, &salt)
}

/// Hash a password using the legacy PBKDF2-HMAC-SHA256 representation.
///
/// # Arguments
///
/// * `password` - Plain-text password.
/// * `salt` - Legacy stored salt text.
///
/// # Returns
///
/// Lowercase hex-encoded legacy digest.
pub fn hash_legacy_password(password: &str, salt: &str) -> String {
    record_kdf_invocation();
    let mut output = [0_u8; HASH_BYTES];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        salt.as_bytes(),
        PBKDF2_ITERATIONS,
        &mut output,
    );
    hex::encode(output)
}

/// Verify a password against either current PHC or legacy PBKDF2 storage.
///
/// # Arguments
///
/// * `password` - Plain-text password.
/// * `legacy_salt` - Legacy salt column, empty for PHC rows.
/// * `stored_hash` - PHC string or legacy lowercase hex digest.
///
/// # Returns
///
/// Match result including whether a successful legacy row needs upgrading.
pub fn verify_password(
    password: &str,
    legacy_salt: &str,
    stored_hash: &str,
) -> PasswordVerification {
    if stored_hash.starts_with("$argon2") {
        return verify_argon2_password(password, stored_hash);
    }
    let actual_hash = hash_legacy_password(password, legacy_salt);
    if bool::from(actual_hash.as_bytes().ct_eq(stored_hash.as_bytes())) {
        PasswordVerification::ValidLegacy
    } else {
        PasswordVerification::Invalid
    }
}

/// Execute the fixed Argon2id dummy verification used for unknown usernames.
///
/// # Arguments
///
/// * `password` - Submitted plain-text password.
pub(crate) fn verify_dummy_password(password: &str) {
    let _ = verify_argon2_password(password, DUMMY_PASSWORD_PHC);
}

fn hash_password_with_salt(password: &str, salt_bytes: &[u8]) -> Result<String, PasswordError> {
    record_kdf_invocation();
    let salt = SaltString::encode_b64(salt_bytes).map_err(|_| PasswordError::HashingFailed)?;
    argon2_context()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| PasswordError::HashingFailed)
}

fn verify_argon2_password(password: &str, stored_hash: &str) -> PasswordVerification {
    let Ok(parsed_hash) = PasswordHash::new(stored_hash) else {
        return PasswordVerification::Invalid;
    };
    if !has_expected_argon2_parameters(&parsed_hash) {
        return PasswordVerification::Invalid;
    }
    record_kdf_invocation();
    if argon2_context()
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_ok()
    {
        PasswordVerification::ValidCurrent
    } else {
        PasswordVerification::Invalid
    }
}

fn has_expected_argon2_parameters(hash: &PasswordHash<'_>) -> bool {
    if hash.algorithm != ARGON2ID_IDENT || hash.version != Some(u32::from(Version::V0x13)) {
        return false;
    }
    let Ok(params) = Params::try_from(hash) else {
        return false;
    };
    params.m_cost() == ARGON2_MEMORY_KIB
        && params.t_cost() == ARGON2_TIME_COST
        && params.p_cost() == ARGON2_PARALLELISM
        && params.output_len() == Some(HASH_BYTES)
        && params.keyid().is_empty()
        && params.data().is_empty()
        && hash.salt.is_some()
        && hash.hash.is_some()
}

fn argon2_context() -> Argon2<'static> {
    let params = Params::new(
        ARGON2_MEMORY_KIB,
        ARGON2_TIME_COST,
        ARGON2_PARALLELISM,
        Some(HASH_BYTES),
    )
    .expect("declared Argon2id parameters should be valid");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

#[cfg(test)]
thread_local! {
    static KDF_INVOCATIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn record_kdf_invocation() {
    KDF_INVOCATIONS.with(|count| count.set(count.get() + 1));
}

#[cfg(not(test))]
fn record_kdf_invocation() {}

#[cfg(test)]
pub(crate) mod test_support {
    //! KDF invocation observations for behavior-focused tests.

    /// Reset the current test thread's KDF invocation count.
    pub(crate) fn reset_kdf_invocations() {
        super::KDF_INVOCATIONS.with(|count| count.set(0));
    }

    /// Return the current test thread's KDF invocation count.
    pub(crate) fn kdf_invocations() -> usize {
        super::KDF_INVOCATIONS.with(std::cell::Cell::get)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        hash_legacy_password, hash_password, hash_password_with_salt, is_valid_new_password,
        verify_dummy_password, verify_password, PasswordVerification, ARGON2_MEMORY_KIB,
        ARGON2_PARALLELISM, ARGON2_TIME_COST, DUMMY_PASSWORD, DUMMY_PASSWORD_PHC,
        DUMMY_PASSWORD_SALT, MIN_PASSWORD_LENGTH,
    };
    use crate::password::test_support::{kdf_invocations, reset_kdf_invocations};

    #[test]
    fn hashes_and_verifies_declared_argon2id_phc() {
        let hash = hash_password("correct horse battery staple")
            .expect("Argon2id password hash should succeed");

        assert!(hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"));
        assert_eq!(
            verify_password("correct horse battery staple", "", &hash),
            PasswordVerification::ValidCurrent
        );
        assert_eq!(
            verify_password("wrong password", "", &hash),
            PasswordVerification::Invalid
        );
        assert_eq!(ARGON2_MEMORY_KIB, 19_456);
        assert_eq!(ARGON2_TIME_COST, 2);
        assert_eq!(ARGON2_PARALLELISM, 1);
    }

    #[test]
    fn verifies_legacy_pbkdf2_without_changing_compatibility() {
        let expected = "8a55c2131c3ecfe2c702d8b8a1f01c0b8f619a9d697d5d9c8d9764e8221fe25e";

        assert_eq!(hash_legacy_password("secret123", "salt"), expected);
        assert_eq!(
            verify_password("secret123", "salt", expected),
            PasswordVerification::ValidLegacy
        );
        assert_eq!(
            verify_password("wrong", "salt", expected),
            PasswordVerification::Invalid
        );
    }

    #[test]
    fn dummy_phc_is_fixed_valid_and_invokes_argon2id() {
        let generated = hash_password_with_salt(DUMMY_PASSWORD, DUMMY_PASSWORD_SALT)
            .expect("fixed dummy PHC should generate");
        assert_eq!(generated, DUMMY_PASSWORD_PHC);

        reset_kdf_invocations();
        verify_dummy_password("submitted password");
        assert_eq!(kdf_invocations(), 1);
    }

    #[test]
    fn auth_password_policy_counts_unicode_characters() {
        assert!(!is_valid_new_password("short"));
        assert!(is_valid_new_password(&"密".repeat(MIN_PASSWORD_LENGTH)));
    }
}
