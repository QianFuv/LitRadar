//! Access token hashing compatibility helpers.

use sha2::{Digest, Sha256};

/// Hash an access token using SHA-256 lowercase hex.
///
/// # Arguments
///
/// * `token` - Raw access token.
///
/// # Returns
///
/// Lowercase SHA-256 hex digest.
pub fn hash_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

#[cfg(test)]
mod tests {
    use super::hash_token;

    #[test]
    fn hashes_token_like_python() {
        assert_eq!(
            hash_token("token"),
            "3c469e9d6c5875d37a43f353d4f88e61fcf812c66eee3457465a40b0da4153e0"
        );
    }
}
