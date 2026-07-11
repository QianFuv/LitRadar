//! Versioned authenticated encryption for persisted integration credentials.

use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use zeroize::Zeroizing;

use crate::open_sqlite_connection;

const ENVELOPE_PREFIX: &str = "litradarenc:v1:";
const SECRET_KEY_BYTES: usize = 32;

/// Codec for versioned encrypted secret envelopes.
#[derive(Clone)]
pub struct SecretCodec {
    key: Arc<Zeroizing<[u8; SECRET_KEY_BYTES]>>,
}

impl SecretCodec {
    /// Load an exact 32-byte deployment key from a file.
    ///
    /// # Arguments
    ///
    /// * `path` - Path to the raw binary key file.
    ///
    /// # Returns
    ///
    /// Codec initialized with zeroizing key material.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SecretError> {
        let bytes = Zeroizing::new(fs::read(path).map_err(SecretError::KeyFile)?);
        if bytes.len() != SECRET_KEY_BYTES {
            return Err(SecretError::InvalidKeyLength(bytes.len()));
        }
        let mut key = [0_u8; SECRET_KEY_BYTES];
        key.copy_from_slice(bytes.as_slice());
        Ok(Self {
            key: Arc::new(Zeroizing::new(key)),
        })
    }

    /// Build a codec from exact key bytes.
    ///
    /// # Arguments
    ///
    /// * `key` - Exact 32-byte deployment key.
    ///
    /// # Returns
    ///
    /// Codec initialized with zeroizing key material.
    pub fn from_key(key: [u8; SECRET_KEY_BYTES]) -> Self {
        Self {
            key: Arc::new(Zeroizing::new(key)),
        }
    }

    /// Encrypt one plaintext value for its stable storage context.
    ///
    /// # Arguments
    ///
    /// * `plaintext` - Secret value to protect.
    /// * `context` - Stable row-and-field associated data.
    ///
    /// # Returns
    ///
    /// Versioned authenticated ciphertext envelope.
    pub fn encrypt(&self, plaintext: &str, context: &str) -> Result<String, SecretError> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.as_ref().as_ref()));
        let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(
                &nonce,
                Payload {
                    msg: plaintext.as_bytes(),
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Authentication)?;
        Ok(format!(
            "{ENVELOPE_PREFIX}{}:{}",
            URL_SAFE_NO_PAD.encode(nonce),
            URL_SAFE_NO_PAD.encode(ciphertext)
        ))
    }

    /// Decrypt one stored value for its stable storage context.
    ///
    /// # Arguments
    ///
    /// * `stored` - Versioned encrypted value or an empty cleared value.
    /// * `context` - Stable row-and-field associated data.
    ///
    /// # Returns
    ///
    /// Decrypted value, or a fail-closed error for plaintext or invalid data.
    pub fn decrypt(&self, stored: &str, context: &str) -> Result<String, SecretError> {
        if stored.is_empty() {
            return Ok(String::new());
        }
        if !stored.starts_with(ENVELOPE_PREFIX) {
            return Err(SecretError::LegacyPlaintext);
        }
        let encoded = &stored[ENVELOPE_PREFIX.len()..];
        let (nonce, ciphertext) = encoded.split_once(':').ok_or(SecretError::Authentication)?;
        let nonce = URL_SAFE_NO_PAD
            .decode(nonce)
            .map_err(|_| SecretError::Authentication)?;
        let ciphertext = URL_SAFE_NO_PAD
            .decode(ciphertext)
            .map_err(|_| SecretError::Authentication)?;
        if nonce.len() != XNonce::default().len() {
            return Err(SecretError::Authentication);
        }
        let nonce = XNonce::from_slice(&nonce);
        let cipher = XChaCha20Poly1305::new(Key::from_slice(self.key.as_ref().as_ref()));
        let plaintext = cipher
            .decrypt(
                nonce,
                Payload {
                    msg: &ciphertext,
                    aad: context.as_bytes(),
                },
            )
            .map_err(|_| SecretError::Authentication)?;
        String::from_utf8(plaintext).map_err(|_| SecretError::Authentication)
    }
}

impl fmt::Debug for SecretCodec {
    /// Format the codec without exposing key material.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretCodec([REDACTED])")
    }
}

/// Secret key, envelope, or database operation failure.
pub enum SecretError {
    /// Key file access failed.
    KeyFile(std::io::Error),
    /// Key file was not exactly 32 bytes.
    InvalidKeyLength(usize),
    /// Stored ciphertext failed authenticated decryption.
    Authentication,
    /// A normal runtime read encountered legacy plaintext.
    LegacyPlaintext,
    /// SQLite returned an error.
    Sqlite(rusqlite::Error),
}

impl fmt::Debug for SecretError {
    /// Format secret failures without exposing key, plaintext, or ciphertext content.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for SecretError {
    /// Format a non-secret diagnostic.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::KeyFile(error) => {
                write!(formatter, "Unable to read the secret key file: {error}")
            }
            Self::InvalidKeyLength(length) => write!(
                formatter,
                "Secret key file must contain exactly {SECRET_KEY_BYTES} bytes; found {length}"
            ),
            Self::Authentication => formatter.write_str("Stored secret authentication failed"),
            Self::LegacyPlaintext => formatter.write_str(
                "Legacy plaintext secret found; run `admin secrets migrate` before startup",
            ),
            Self::Sqlite(error) => write!(formatter, "{error}"),
        }
    }
}

impl Error for SecretError {
    /// Return the underlying non-secret source error.
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::KeyFile(error) => Some(error),
            Self::Sqlite(error) => Some(error),
            _ => None,
        }
    }
}

impl From<rusqlite::Error> for SecretError {
    /// Convert SQLite errors into secret errors.
    fn from(error: rusqlite::Error) -> Self {
        Self::Sqlite(error)
    }
}

/// Counts returned by explicit plaintext migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretMigrationReport {
    /// Number of plaintext values encrypted.
    pub migrated: usize,
    /// Number of existing encrypted values authenticated.
    pub verified: usize,
    /// Number of empty cleared values observed.
    pub empty: usize,
}

/// Counts returned by fail-closed secret verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SecretVerificationReport {
    /// Number of encrypted values authenticated.
    pub verified: usize,
    /// Number of empty cleared values observed.
    pub empty: usize,
}

/// Explicitly encrypt legacy plaintext values in one auth database.
///
/// # Arguments
///
/// * `auth_db_path` - Migrated auth database path.
/// * `codec` - Deployment-key codec used for new envelopes.
///
/// # Returns
///
/// Transactional migration counts.
pub fn migrate_database_secrets(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<SecretMigrationReport, SecretError> {
    let mut connection = open_sqlite_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut report = SecretMigrationReport {
        migrated: 0,
        verified: 0,
        empty: 0,
    };
    visit_secret_values(&transaction, |stored, context| {
        if stored.is_empty() {
            report.empty += 1;
            return Ok(None);
        }
        if stored.starts_with("litradarenc:") {
            codec.decrypt(stored, context)?;
            report.verified += 1;
            return Ok(None);
        }
        report.migrated += 1;
        codec.encrypt(stored, context).map(Some)
    })?;
    transaction.commit()?;
    Ok(report)
}

/// Verify that all stored secret values are encrypted and authentic.
///
/// # Arguments
///
/// * `auth_db_path` - Migrated auth database path.
/// * `codec` - Deployment-key codec used to authenticate envelopes.
///
/// # Returns
///
/// Verification counts when every value passes.
pub fn verify_database_secrets(
    auth_db_path: impl AsRef<Path>,
    codec: &SecretCodec,
) -> Result<SecretVerificationReport, SecretError> {
    let connection = open_sqlite_connection(auth_db_path)?;
    let mut report = SecretVerificationReport {
        verified: 0,
        empty: 0,
    };
    inspect_secret_values(&connection, |stored, context| {
        if stored.is_empty() {
            report.empty += 1;
        } else {
            codec.decrypt(stored, context)?;
            report.verified += 1;
        }
        Ok(())
    })?;
    Ok(report)
}

/// Re-encrypt all stored values from one deployment key to another.
///
/// # Arguments
///
/// * `auth_db_path` - Migrated auth database path.
/// * `old_codec` - Codec for current envelopes.
/// * `new_codec` - Codec for replacement envelopes.
///
/// # Returns
///
/// Number of values rotated transactionally.
pub fn rotate_database_secrets(
    auth_db_path: impl AsRef<Path>,
    old_codec: &SecretCodec,
    new_codec: &SecretCodec,
) -> Result<usize, SecretError> {
    let mut connection = open_sqlite_connection(auth_db_path)?;
    let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
    let mut rotated = 0;
    visit_secret_values(&transaction, |stored, context| {
        if stored.is_empty() {
            return Ok(None);
        }
        let plaintext = old_codec.decrypt(stored, context)?;
        rotated += 1;
        new_codec.encrypt(&plaintext, context).map(Some)
    })?;
    transaction.commit()?;
    Ok(rotated)
}

fn visit_secret_values(
    transaction: &Transaction<'_>,
    mut visitor: impl FnMut(&str, &str) -> Result<Option<String>, SecretError>,
) -> Result<(), SecretError> {
    let notification_rows = query_notification_secrets(transaction)?;
    for (user_id, field, stored) in notification_rows {
        let context = notification_context(user_id, field);
        if let Some(replacement) = visitor(&stored, &context)? {
            transaction.execute(
                &format!("UPDATE notification_settings SET {field} = ?1 WHERE user_id = ?2"),
                params![replacement, user_id],
            )?;
        }
    }
    let runtime_rows = query_runtime_secrets(transaction)?;
    for (field, stored) in runtime_rows {
        let context = runtime_context(&field);
        if let Some(replacement) = visitor(&stored, &context)? {
            transaction.execute(
                "UPDATE runtime_settings SET value = ?1 WHERE key = ?2",
                params![replacement, field],
            )?;
        }
    }
    let cnki_rows = query_cnki_secrets(transaction)?;
    for (user_id, stored) in cnki_rows {
        let context = cnki_context(user_id);
        if let Some(replacement) = visitor(&stored, &context)? {
            transaction.execute(
                "UPDATE cnki_sessions SET session_json = ?1 WHERE user_id = ?2",
                params![replacement, user_id],
            )?;
        }
    }
    Ok(())
}

fn inspect_secret_values(
    connection: &Connection,
    mut visitor: impl FnMut(&str, &str) -> Result<(), SecretError>,
) -> Result<(), SecretError> {
    for (user_id, field, stored) in query_notification_secrets(connection)? {
        visitor(&stored, &notification_context(user_id, field))?;
    }
    for (field, stored) in query_runtime_secrets(connection)? {
        visitor(&stored, &runtime_context(&field))?;
    }
    for (user_id, stored) in query_cnki_secrets(connection)? {
        visitor(&stored, &cnki_context(user_id))?;
    }
    Ok(())
}

fn query_notification_secrets(
    connection: &Connection,
) -> Result<Vec<(i64, &'static str, String)>, SecretError> {
    let mut statement = connection.prepare(
        "SELECT user_id, pushplus_token, ai_api_key, ai_backup_api_key FROM notification_settings",
    )?;
    let rows = statement.query_map([], |row| {
        let user_id = row.get::<_, i64>(0)?;
        Ok([
            (user_id, "pushplus_token", row.get::<_, String>(1)?),
            (user_id, "ai_api_key", row.get::<_, String>(2)?),
            (user_id, "ai_backup_api_key", row.get::<_, String>(3)?),
        ])
    })?;
    let mut values = Vec::new();
    for row in rows {
        values.extend(row?);
    }
    Ok(values)
}

fn query_runtime_secrets(connection: &Connection) -> Result<Vec<(String, String)>, SecretError> {
    let mut statement = connection.prepare(
        "SELECT key, value FROM runtime_settings \
         WHERE key IN ('openalex_api_key_pool', 'semantic_scholar_api_key_pool')",
    )?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(SecretError::from)
}

fn query_cnki_secrets(connection: &Connection) -> Result<Vec<(i64, String)>, SecretError> {
    let mut statement = connection.prepare("SELECT user_id, session_json FROM cnki_sessions")?;
    let rows = statement.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(SecretError::from)
}

/// Return stable associated data for one notification secret field.
///
/// # Arguments
///
/// * `user_id` - Owner row identifier.
/// * `field` - Secret column name.
///
/// # Returns
///
/// Stable associated-data string.
pub(crate) fn notification_context(user_id: i64, field: &str) -> String {
    format!("notification_settings:{user_id}:{field}")
}

/// Return stable associated data for one runtime setting.
///
/// # Arguments
///
/// * `field` - Runtime setting key.
///
/// # Returns
///
/// Stable associated-data string.
pub(crate) fn runtime_context(field: &str) -> String {
    format!("runtime_settings:{field}")
}

/// Return stable associated data for one CNKI session.
///
/// # Arguments
///
/// * `user_id` - Owner row identifier.
///
/// # Returns
///
/// Stable associated-data string.
pub(crate) fn cnki_context(user_id: i64) -> String {
    format!("cnki_sessions:{user_id}:session_json")
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;
    use tempfile::tempdir;

    use super::{SecretCodec, SecretError};

    #[test]
    fn envelope_round_trip_binds_ciphertext_to_context() {
        let codec = SecretCodec::from_key([7_u8; 32]);
        let envelope = codec
            .encrypt("fixture-secret", "table:1:field")
            .expect("secret should encrypt");

        assert!(envelope.starts_with("litradarenc:v1:"));
        assert!(!envelope.contains("fixture-secret"));
        assert_eq!(
            codec
                .decrypt(&envelope, "table:1:field")
                .expect("secret should decrypt"),
            "fixture-secret"
        );
        assert!(matches!(
            codec.decrypt(&envelope, "table:2:field"),
            Err(SecretError::Authentication)
        ));
    }

    #[test]
    fn missing_wrong_length_wrong_key_and_tampering_fail_closed() {
        let temp_dir = tempdir().expect("temporary directory should exist");
        let missing = SecretCodec::load(temp_dir.path().join("missing.key"))
            .expect_err("missing key should fail");
        assert!(matches!(missing, SecretError::KeyFile(_)));

        let short_path = temp_dir.path().join("short.key");
        std::fs::write(&short_path, [1_u8; 31]).expect("short key should write");
        assert!(matches!(
            SecretCodec::load(short_path),
            Err(SecretError::InvalidKeyLength(31))
        ));

        let codec = SecretCodec::from_key([1_u8; 32]);
        let wrong = SecretCodec::from_key([2_u8; 32]);
        let envelope = codec
            .encrypt("fixture-secret", "fixture-context")
            .expect("secret should encrypt");
        assert!(matches!(
            wrong.decrypt(&envelope, "fixture-context"),
            Err(SecretError::Authentication)
        ));
        let mut tampered = envelope.into_bytes();
        let last = tampered.last_mut().expect("envelope should not be empty");
        *last = if *last == b'A' { b'B' } else { b'A' };
        let tampered = String::from_utf8(tampered).expect("envelope should remain UTF-8");
        assert!(matches!(
            codec.decrypt(&tampered, "fixture-context"),
            Err(SecretError::Authentication)
        ));
        assert!(matches!(
            codec.decrypt("legacy-secret", "fixture-context"),
            Err(SecretError::LegacyPlaintext)
        ));
    }

    #[test]
    fn migration_is_transactional_when_an_existing_envelope_is_invalid() {
        let temp_dir = tempdir().expect("temporary directory should exist");
        let database = temp_dir.path().join("auth.sqlite");
        let connection = Connection::open(&database).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE notification_settings (
                    user_id INTEGER PRIMARY KEY,
                    pushplus_token TEXT NOT NULL,
                    ai_api_key TEXT NOT NULL,
                    ai_backup_api_key TEXT NOT NULL
                );
                CREATE TABLE runtime_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE cnki_sessions (user_id INTEGER PRIMARY KEY, session_json TEXT NOT NULL);
                INSERT INTO notification_settings VALUES (1, 'plain-one', 'litradarenc:v1:bad', '');",
            )
            .expect("fixture schema should write");
        drop(connection);

        let codec = SecretCodec::from_key([3_u8; 32]);
        let error = super::migrate_database_secrets(&database, &codec)
            .expect_err("invalid envelope should roll back migration");
        assert!(matches!(error, SecretError::Authentication));
        let connection = Connection::open(database).expect("database should reopen");
        let stored: String = connection
            .query_row(
                "SELECT pushplus_token FROM notification_settings WHERE user_id = 1",
                [],
                |row| row.get(0),
            )
            .expect("stored value should load");
        assert_eq!(stored, "plain-one");
    }

    #[test]
    fn explicit_migration_verification_and_rotation_cover_every_secret_table() {
        let temp_dir = tempdir().expect("temporary directory should exist");
        let database = temp_dir.path().join("auth.sqlite");
        let connection = Connection::open(&database).expect("database should open");
        connection
            .execute_batch(
                "CREATE TABLE notification_settings (
                    user_id INTEGER PRIMARY KEY,
                    pushplus_token TEXT NOT NULL,
                    ai_api_key TEXT NOT NULL,
                    ai_backup_api_key TEXT NOT NULL
                );
                CREATE TABLE runtime_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                CREATE TABLE cnki_sessions (user_id INTEGER PRIMARY KEY, session_json TEXT NOT NULL);
                INSERT INTO notification_settings VALUES (
                    7, 'push-plaintext', 'primary-plaintext', 'backup-plaintext'
                );
                INSERT INTO runtime_settings VALUES ('openalex_api_key_pool', 'pool-plaintext');
                INSERT INTO cnki_sessions VALUES (7, '{\"token\":\"cnki-plaintext\"}');",
            )
            .expect("fixture schema should write");
        drop(connection);
        let codec = SecretCodec::from_key([4_u8; 32]);

        assert!(matches!(
            super::verify_database_secrets(&database, &codec),
            Err(SecretError::LegacyPlaintext)
        ));
        let migrated = super::migrate_database_secrets(&database, &codec)
            .expect("plaintext migration should succeed");
        assert_eq!(migrated.migrated, 5);
        assert_eq!(migrated.verified, 0);
        let verified = super::verify_database_secrets(&database, &codec)
            .expect("migrated secrets should verify");
        assert_eq!(verified.verified, 5);
        let connection = Connection::open(&database).expect("database should reopen");
        let stored = connection
            .query_row(
                "SELECT pushplus_token || ai_api_key || ai_backup_api_key \
                 FROM notification_settings WHERE user_id = 7",
                [],
                |row| row.get::<_, String>(0),
            )
            .expect("encrypted notification values should load");
        assert!(!stored.contains("plaintext"));
        drop(connection);
        assert!(matches!(
            super::verify_database_secrets(&database, &SecretCodec::from_key([5_u8; 32])),
            Err(SecretError::Authentication)
        ));

        let replacement = SecretCodec::from_key([6_u8; 32]);
        assert_eq!(
            super::rotate_database_secrets(&database, &codec, &replacement)
                .expect("secret rotation should succeed"),
            5
        );
        super::verify_database_secrets(&database, &replacement)
            .expect("replacement key should verify");
        assert!(matches!(
            super::verify_database_secrets(&database, &codec),
            Err(SecretError::Authentication)
        ));

        let connection = Connection::open(&database).expect("database should reopen");
        connection
            .execute(
                "UPDATE runtime_settings SET value = value || 'A' \
                 WHERE key = 'openalex_api_key_pool'",
                [],
            )
            .expect("ciphertext should tamper");
        drop(connection);
        assert!(matches!(
            super::verify_database_secrets(&database, &replacement),
            Err(SecretError::Authentication)
        ));
    }
}
