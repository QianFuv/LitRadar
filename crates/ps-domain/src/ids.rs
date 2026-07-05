//! Identifier types and stable SQLite identifier generation.

use std::fmt;

use serde::de::{self, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest, Sha256};

/// Maximum positive SQLite integer used by Python-compatible stable hashes.
pub const SQLITE_INT_MAX: u64 = i64::MAX as u64;

/// Article identifier stored internally as a SQLite integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ArticleId(pub i64);

/// Journal identifier stored internally as a SQLite integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct JournalId(pub i64);

/// Auth user identifier stored as an ordinary SQLite integer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct UserId(pub i64);

impl ArticleId {
    /// Return the raw SQLite integer.
    ///
    /// # Returns
    ///
    /// Raw article identifier.
    pub fn value(self) -> i64 {
        self.0
    }
}

impl JournalId {
    /// Return the raw SQLite integer.
    ///
    /// # Returns
    ///
    /// Raw journal identifier.
    pub fn value(self) -> i64 {
        self.0
    }
}

impl UserId {
    /// Return the raw SQLite integer.
    ///
    /// # Returns
    ///
    /// Raw user identifier.
    pub fn value(self) -> i64 {
        self.0
    }
}

impl Serialize for ArticleId {
    /// Serialize the article identifier as a decimal JSON string.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for ArticleId {
    /// Deserialize an article identifier from a decimal string or integer.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_sqlite_id(deserializer).map(Self)
    }
}

impl Serialize for JournalId {
    /// Serialize the journal identifier as a decimal JSON string.
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.0.to_string())
    }
}

impl<'de> Deserialize<'de> for JournalId {
    /// Deserialize a journal identifier from a decimal string or integer.
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_sqlite_id(deserializer).map(Self)
    }
}

/// Convert a Python source value into the same stable SQLite integer.
///
/// # Arguments
///
/// * `value` - Source identifier value.
/// * `prefix` - Domain prefix used to reduce cross-domain collisions.
///
/// # Returns
///
/// Existing integer values when valid, otherwise a SHA-256-derived positive
/// SQLite integer.
pub fn stable_sqlite_id(value: impl AsRef<str>, prefix: impl AsRef<str>) -> i64 {
    let value = value.as_ref();
    if let Ok(parsed) = value.parse::<i64>() {
        return parsed;
    }

    let text = format!("{}:{value}", prefix.as_ref());
    let digest = Sha256::digest(text.as_bytes());
    let mut bytes = [0_u8; 8];
    bytes.copy_from_slice(&digest[..8]);
    let raw_value = u64::from_be_bytes(bytes);
    let safe_value = raw_value & SQLITE_INT_MAX;
    if safe_value == 0 {
        1
    } else {
        safe_value as i64
    }
}

fn deserialize_sqlite_id<'de, D>(deserializer: D) -> Result<i64, D::Error>
where
    D: Deserializer<'de>,
{
    deserializer.deserialize_any(SqliteIdVisitor)
}

struct SqliteIdVisitor;

impl<'de> Visitor<'de> for SqliteIdVisitor {
    type Value = i64;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a SQLite integer or decimal string")
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(value)
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        i64::try_from(value).map_err(E::custom)
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        value.parse::<i64>().map_err(E::custom)
    }
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::Value;

    use super::{stable_sqlite_id, ArticleId, JournalId};

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct ArticlePreview {
        article_id: ArticleId,
        journal_id: JournalId,
    }

    #[test]
    fn serializes_ids_as_decimal_strings() {
        let preview = ArticlePreview {
            article_id: ArticleId(9_007_199_254_740_995),
            journal_id: JournalId(9_007_199_254_740_993),
        };

        let payload = serde_json::to_value(preview).expect("preview should serialize");

        assert_eq!(
            payload["article_id"],
            Value::String("9007199254740995".into())
        );
        assert_eq!(
            payload["journal_id"],
            Value::String("9007199254740993".into())
        );
    }

    #[test]
    fn deserializes_ids_from_decimal_strings() {
        let preview: ArticlePreview =
            serde_json::from_str(r#"{"article_id":"42","journal_id":"99"}"#)
                .expect("preview should deserialize");

        assert_eq!(
            preview,
            ArticlePreview {
                article_id: ArticleId(42),
                journal_id: JournalId(99),
            }
        );
    }

    #[test]
    fn stable_ids_match_python_golden_values() {
        assert_eq!(
            stable_sqlite_id("10.1000/golden", "article"),
            1_916_262_609_001_879_182
        );
        assert_eq!(
            stable_sqlite_id("Golden Systems Journal", "journal"),
            1_786_608_276_134_106_497
        );
    }
}
