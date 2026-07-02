//! Password hashing compatibility with the Python backend.

use pbkdf2::pbkdf2_hmac;
use sha2::Sha256;
use subtle::ConstantTimeEq;

/// PBKDF2 iteration count used by the existing Python backend.
pub const PBKDF2_ITERATIONS: u32 = 260_000;
const HASH_BYTES: usize = 32;

/// Hash a password using PBKDF2-HMAC-SHA256 and return lowercase hex.
///
/// # Arguments
///
/// * `password` - Plain-text password.
/// * `salt` - Stored salt text.
///
/// # Returns
///
/// Lowercase hex-encoded password hash.
pub fn hash_password(password: &str, salt: &str) -> String {
    let mut output = [0_u8; HASH_BYTES];
    pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        salt.as_bytes(),
        PBKDF2_ITERATIONS,
        &mut output,
    );
    hex::encode(output)
}

/// Verify a password against a stored lowercase PBKDF2 hex digest.
///
/// # Arguments
///
/// * `password` - Plain-text password.
/// * `salt` - Stored salt text.
/// * `expected_hash` - Stored lowercase hex digest.
///
/// # Returns
///
/// True when the password matches.
pub fn verify_password(password: &str, salt: &str, expected_hash: &str) -> bool {
    let actual_hash = hash_password(password, salt);
    actual_hash
        .as_bytes()
        .ct_eq(expected_hash.as_bytes())
        .into()
}

#[cfg(test)]
mod tests {
    use super::{hash_password, verify_password};

    #[test]
    fn hashes_password_like_python() {
        let expected = "8a55c2131c3ecfe2c702d8b8a1f01c0b8f619a9d697d5d9c8d9764e8221fe25e";

        assert_eq!(hash_password("secret123", "salt"), expected);
        assert!(verify_password("secret123", "salt", expected));
        assert!(!verify_password("wrong", "salt", expected));
    }
}
