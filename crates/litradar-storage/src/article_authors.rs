//! Stored article author JSON compatibility decoding.

use litradar_domain::ArticleAuthorDraft;
use serde::Deserialize;

#[derive(Deserialize)]
#[serde(untagged)]
enum StoredArticleAuthors {
    Canonical(Vec<ArticleAuthorDraft>),
    Legacy(Vec<String>),
}

/// Decode canonical author objects and legacy author-name arrays.
pub(crate) fn decode_article_author_names(payload: &str) -> serde_json::Result<Vec<String>> {
    serde_json::from_str::<StoredArticleAuthors>(payload).map(|authors| match authors {
        StoredArticleAuthors::Canonical(authors) => authors
            .into_iter()
            .map(|author| author.display_name)
            .collect(),
        StoredArticleAuthors::Legacy(authors) => authors,
    })
}

#[cfg(test)]
mod tests {
    use super::decode_article_author_names;

    #[test]
    fn author_decoder_accepts_canonical_and_legacy_storage_shapes() {
        assert_eq!(
            decode_article_author_names(
                r#"[{"display_name":"Ada Lovelace"},{"display_name":"Grace Hopper"}]"#,
            )
            .expect("canonical authors should decode"),
            ["Ada Lovelace", "Grace Hopper"]
        );
        assert_eq!(
            decode_article_author_names(r#"["Ada Lovelace","Grace Hopper"]"#)
                .expect("legacy authors should decode"),
            ["Ada Lovelace", "Grace Hopper"]
        );
        assert!(
            decode_article_author_names(r#"[{"display_name":"Ada Lovelace"},"Grace Hopper"]"#)
                .is_err()
        );
    }
}
