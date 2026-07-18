//! Provider-neutral article locator repository for request-time access resolution.

use super::shared::*;
use super::*;

/// Load canonical article metadata for an online provider action.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional content database name.
/// * `article_id` - Immutable article identifier.
///
/// # Returns
///
/// Provider-neutral locator or a not-found error.
pub fn get_article_locator(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
) -> Result<ArticleLocator, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    connection
        .query_row(
            "SELECT a.article_id, j.catalog_id, j.title, j.issns_json, a.title, \
             a.publication_year, a.date, a.authors_json, i.volume, i.number, \
             a.start_page, a.end_page, a.doi, a.pmid \
             FROM articles a \
             JOIN journals j ON j.journal_id = a.journal_id \
             LEFT JOIN issues i ON i.issue_id = a.issue_id \
             WHERE a.article_id = ?1",
            [article_id],
            article_locator_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))
}

fn article_locator_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleLocator> {
    Ok(ArticleLocator {
        article_id: ArticleId(row.get(0)?),
        catalog_id: row.get(1)?,
        journal_title: row.get(2)?,
        journal_issns: json_string_vec_from_row(row, 3)?,
        title: row.get(4)?,
        publication_year: row.get(5)?,
        date: row.get(6)?,
        authors: json_string_vec_from_row(row, 7)?,
        volume: row.get(8)?,
        issue_number: row.get(9)?,
        start_page: row.get(10)?,
        end_page: row.get(11)?,
        doi: row.get(12)?,
        pmid: row.get(13)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_support::IndexFixture;

    #[test]
    fn article_locator_contains_only_canonical_content() {
        let fixture = IndexFixture::new(true);
        let locator = get_article_locator(&fixture.config, Some(&fixture.db_name), 1001)
            .expect("locator should load");

        assert_eq!(locator.catalog_id, "alpha-journal");
        assert_eq!(locator.journal_issns, ["1234-5679"]);
        assert_eq!(locator.authors, ["Alice", "Bob"]);
        assert_eq!(locator.doi.as_deref(), Some("10.1000/genome"));
        let serialized = format!("{locator:?}");
        for forbidden in ["provider", "platform", "permalink", "https://"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
