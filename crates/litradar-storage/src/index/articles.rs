//! Provider-neutral article listing, lookup, count, and delivery repositories.

use super::shared::*;
use super::*;

/// Article list filters backed by canonical content projections.
#[derive(Debug, Clone)]
pub struct ArticleListParams {
    /// Journal identifiers.
    pub journal_id: Vec<i64>,
    /// Issue identifier.
    pub issue_id: Option<i64>,
    /// Publication year.
    pub year: Option<i64>,
    /// Journal areas.
    pub area: Vec<String>,
    /// In-press filter.
    pub in_press: Option<bool>,
    /// Open-access filter.
    pub open_access: Option<bool>,
    /// Minimum date.
    pub date_from: Option<String>,
    /// Maximum date.
    pub date_to: Option<String>,
    /// DOI filter.
    pub doi: Option<String>,
    /// PMID filter.
    pub pmid: Option<String>,
    /// FTS query.
    pub q: Option<String>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
    /// Cursor string.
    pub cursor: Option<String>,
    /// Whether to include total count.
    pub include_total: bool,
}

impl Default for ArticleListParams {
    /// Build default article list parameters.
    fn default() -> Self {
        Self {
            journal_id: Vec::new(),
            issue_id: None,
            year: None,
            area: Vec::new(),
            in_press: None,
            open_access: None,
            date_from: None,
            date_to: None,
            doi: None,
            pmid: None,
            q: None,
            sort: Some("date:desc".to_string()),
            limit: 50,
            offset: 0,
            cursor: None,
            include_total: true,
        }
    }
}

/// Collect article counts grouped by journal and issue.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
///
/// # Returns
///
/// Snapshot map keyed by `journal_id:issue_id`.
pub fn collect_issue_article_counts(
    index_db_path: impl AsRef<Path>,
) -> Result<BTreeMap<String, i64>, IndexRepositoryError> {
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(
        "SELECT journal_id, issue_id, COUNT(*) FROM articles \
         WHERE issue_id IS NOT NULL GROUP BY journal_id, issue_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            build_issue_key(row.get(0)?, row.get(1)?),
            row.get::<_, i64>(2)?,
        ))
    })?;
    collect_rows(rows).map(|items| items.into_iter().collect())
}

/// Collect in-press article counts grouped by journal.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
///
/// # Returns
///
/// Snapshot map keyed by journal id.
pub fn collect_inpress_article_counts(
    index_db_path: impl AsRef<Path>,
) -> Result<BTreeMap<String, i64>, IndexRepositoryError> {
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(
        "SELECT journal_id, COUNT(*) FROM articles \
         WHERE issue_id IS NULL AND COALESCE(in_press, 0) = 1 GROUP BY journal_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, i64>(0)?.to_string(), row.get::<_, i64>(1)?))
    })?;
    collect_rows(rows).map(|items| items.into_iter().collect())
}

/// Fetch article candidates for issue keys.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `issue_keys` - Pending issue keys.
///
/// # Returns
///
/// Canonical candidates ordered by date and identifier.
pub fn fetch_candidates_for_issue_keys(
    index_db_path: impl AsRef<Path>,
    issue_keys: &[String],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if issue_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut issue_ids = issue_keys
        .iter()
        .map(|key| parse_issue_key(key).map(|(_, issue_id)| issue_id))
        .collect::<Result<Vec<_>, _>>()?;
    issue_ids.sort_unstable();
    issue_ids.dedup();
    fetch_candidates_by_column(index_db_path, "a.issue_id", &issue_ids, "")
}

/// Fetch visible in-press candidates for journal keys.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `inpress_keys` - Pending in-press journal keys.
///
/// # Returns
///
/// Canonical candidates ordered by date and identifier.
pub fn fetch_candidates_for_inpress_keys(
    index_db_path: impl AsRef<Path>,
    inpress_keys: &[String],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if inpress_keys.is_empty() {
        return Ok(Vec::new());
    }
    let mut journal_ids = inpress_keys
        .iter()
        .map(|key| {
            key.parse::<i64>()
                .map_err(|_| IndexRepositoryError::InvalidCursor)
        })
        .collect::<Result<Vec<_>, _>>()?;
    journal_ids.sort_unstable();
    journal_ids.dedup();
    fetch_candidates_by_column(
        index_db_path,
        "a.journal_id",
        &journal_ids,
        "a.issue_id IS NULL AND COALESCE(a.in_press, 0) = 1 AND ",
    )
}

/// Fetch article candidates by explicit identifiers.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `article_ids` - Article identifiers to load.
///
/// # Returns
///
/// Canonical candidates ordered by date and identifier.
pub fn fetch_candidates_for_article_ids(
    index_db_path: impl AsRef<Path>,
    article_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut article_ids = article_ids.to_vec();
    article_ids.sort_unstable();
    article_ids.dedup();
    fetch_candidates_by_column(index_db_path, "a.article_id", &article_ids, "")
}

/// List articles with canonical filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Article filters.
///
/// # Returns
///
/// Paginated article response.
pub fn list_articles(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_int_list_filter(
        &mut clauses,
        &mut values,
        "l.journal_id",
        &params.journal_id,
    );
    push_optional_int_filter(&mut clauses, &mut values, "l.issue_id = ?", params.issue_id);
    push_string_list_filter(&mut clauses, &mut values, "l.area", &params.area);
    push_optional_bool_filter(&mut clauses, &mut values, "l.in_press = ?", params.in_press);
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.open_access = ?",
        params.open_access,
    );
    push_optional_text_filter(&mut clauses, &mut values, "l.date >= ?", &params.date_from);
    push_optional_text_filter(&mut clauses, &mut values, "l.date <= ?", &params.date_to);
    push_optional_text_filter(&mut clauses, &mut values, "l.doi = ?", &params.doi);
    push_optional_text_filter(&mut clauses, &mut values, "l.pmid = ?", &params.pmid);
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "l.publication_year = ?",
        params.year,
    );
    push_fts_filter(&mut clauses, &mut values, "l.article_id", &params.q);
    let direction = article_sort_direction(params.sort.as_deref().unwrap_or("date:desc"))?;
    push_cursor_filter(
        &mut clauses,
        &mut values,
        "l",
        direction,
        params.cursor.as_deref(),
    )?;
    let where_sql = where_sql(&clauses);
    let total = if params.include_total {
        Some(connection.query_row(
            &format!("SELECT COUNT(*) FROM article_listing l {where_sql}"),
            params_from_iter(values.iter()),
            |row| row.get(0),
        )?)
    } else {
        None
    };
    let id_rows = article_id_rows(&connection, &where_sql, direction, &values, params)?;
    article_page_from_ids(&connection, id_rows, total, params)
}

/// Get one article.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `article_id` - Article identifier.
///
/// # Returns
///
/// Canonical article record.
pub fn get_article(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
) -> Result<ArticleRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    fetch_articles_by_ids(&connection, &[article_id])?
        .into_iter()
        .next()
        .ok_or(IndexRepositoryError::NotFound("Article not found"))
}

fn article_id_rows(
    connection: &Connection,
    where_sql: &str,
    direction: SortDirection,
    values: &[SqlValue],
    params: &ArticleListParams,
) -> Result<Vec<(i64, Option<String>)>, IndexRepositoryError> {
    let mut page_values = values.to_vec();
    page_values.push(SqlValue::Integer(params.limit));
    let pagination_sql = if params.cursor.is_none() {
        page_values.push(SqlValue::Integer(params.offset));
        "LIMIT ? OFFSET ?"
    } else {
        "LIMIT ?"
    };
    let order_direction = direction.sql();
    let mut statement = connection.prepare(&format!(
        "SELECT l.article_id, l.date FROM article_listing l {where_sql} \
         ORDER BY l.date {order_direction}, l.article_id {order_direction} {pagination_sql}"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), |row| {
        Ok((row.get(0)?, row.get(1)?))
    })?;
    collect_rows(rows)
}

fn article_page_from_ids(
    connection: &Connection,
    id_rows: Vec<(i64, Option<String>)>,
    total: Option<i64>,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    let has_more = id_rows.len() as i64 == params.limit;
    let next_cursor = if has_more {
        id_rows
            .last()
            .and_then(|(article_id, date)| date.as_ref().map(|date| format!("{date}|{article_id}")))
    } else {
        None
    };
    let article_ids = id_rows
        .iter()
        .map(|(article_id, _)| *article_id)
        .collect::<Vec<_>>();
    Ok(ArticlePage {
        items: fetch_articles_by_ids(connection, &article_ids)?,
        page: page_meta(
            total,
            params.limit,
            params.offset,
            next_cursor.clone(),
            Some(has_more && next_cursor.is_some()),
        ),
    })
}

fn fetch_articles_by_ids(
    connection: &Connection,
    article_ids: &[i64],
) -> Result<Vec<ArticleRecord>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let values = article_ids
        .iter()
        .copied()
        .map(SqlValue::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.publication_year, \
         a.date, a.authors_json, a.start_page, a.end_page, a.abstract_text, a.doi, \
         a.pmid, a.in_press, a.open_access, j.title, i.volume, i.number \
         FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
         JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({})",
        placeholders(article_ids.len())
    ))?;
    let rows = statement.query_map(params_from_iter(values.iter()), article_from_row)?;
    let mut by_id = collect_rows(rows)?
        .into_iter()
        .map(|article: ArticleRecord| (article.article_id.value(), article))
        .collect::<HashMap<_, _>>();
    let mut retraction_statement = connection.prepare(&format!(
        "SELECT article_id, retraction_doi FROM article_retraction_dois
         WHERE article_id IN ({}) ORDER BY article_id, retraction_doi",
        placeholders(article_ids.len())
    ))?;
    let retraction_rows = retraction_statement
        .query_map(params_from_iter(values.iter()), |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
    for row in retraction_rows {
        let (article_id, retraction_doi) = row?;
        if let Some(article) = by_id.get_mut(&article_id) {
            article.retraction_dois.push(retraction_doi);
        }
    }
    Ok(article_ids
        .iter()
        .filter_map(|article_id| by_id.remove(article_id))
        .collect())
}

fn article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleRecord> {
    Ok(ArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        publication_year: row.get(4)?,
        date: row.get(5)?,
        authors: json_string_vec_from_row(row, 6)?,
        start_page: row.get(7)?,
        end_page: row.get(8)?,
        abstract_text: row.get(9)?,
        doi: row.get(10)?,
        pmid: row.get(11)?,
        in_press: row.get::<_, Option<i64>>(12)?.map(|value| value != 0),
        open_access: row.get::<_, Option<i64>>(13)?.map(|value| value != 0),
        retraction_dois: Vec::new(),
        journal_title: row.get(14)?,
        volume: row.get(15)?,
        number: row.get(16)?,
    })
}

fn article_sort_direction(sort: &str) -> Result<SortDirection, IndexRepositoryError> {
    let specs = sort_specs(sort, &[("date", "date")])?;
    if specs.len() != 1 {
        return Err(IndexRepositoryError::UnsupportedArticleSort);
    }
    Ok(specs[0].direction)
}

fn fetch_candidates_by_column(
    index_db_path: impl AsRef<Path>,
    column: &str,
    ids: &[i64],
    prefix_clause: &str,
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract_text, a.date, \
         a.open_access, a.in_press, a.doi, j.title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE {prefix_clause}{column} IN ({}) \
         ORDER BY a.date DESC, a.article_id DESC",
        placeholders(ids.len())
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleCandidateInfo> {
    Ok(ArticleCandidateInfo {
        article_id: row.get(0)?,
        journal_id: row.get(1)?,
        issue_id: row.get(2)?,
        title: row.get(3)?,
        abstract_text: row.get::<_, Option<String>>(4)?.unwrap_or_default(),
        date: row.get(5)?,
        open_access: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
        in_press: row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0,
        doi: row.get(8)?,
        journal_title: row.get(9)?,
    })
}

fn build_issue_key(journal_id: i64, issue_id: i64) -> String {
    format!("{journal_id}:{issue_id}")
}

fn parse_issue_key(key: &str) -> Result<(i64, i64), IndexRepositoryError> {
    let (journal_id, issue_id) = key
        .split_once(':')
        .ok_or(IndexRepositoryError::InvalidCursor)?;
    Ok((
        journal_id
            .parse()
            .map_err(|_| IndexRepositoryError::InvalidCursor)?,
        issue_id
            .parse()
            .map_err(|_| IndexRepositoryError::InvalidCursor)?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_support::{
        article_filter_params, article_ids, candidate_ids, fixture_db_path, IndexFixture,
    };

    #[test]
    fn article_listing_filters_cover_fts5_and_canonical_expressions() {
        let fixture = IndexFixture::new(true);
        let cases = vec![
            (
                "journal ids",
                ArticleListParams {
                    journal_id: vec![1],
                    ..article_filter_params()
                },
                vec![1004, 1001, 1002, 1005, 1008],
            ),
            (
                "issue id",
                ArticleListParams {
                    issue_id: Some(10),
                    ..article_filter_params()
                },
                vec![1001, 1002, 1005, 1008],
            ),
            (
                "publication year",
                ArticleListParams {
                    year: Some(2026),
                    ..article_filter_params()
                },
                vec![1003, 1004, 1001, 1002, 1005, 1008],
            ),
            (
                "area",
                ArticleListParams {
                    area: vec!["Engineering".to_string()],
                    ..article_filter_params()
                },
                vec![1003],
            ),
            (
                "in press",
                ArticleListParams {
                    in_press: Some(true),
                    ..article_filter_params()
                },
                vec![1004],
            ),
            (
                "open access",
                ArticleListParams {
                    open_access: Some(true),
                    ..article_filter_params()
                },
                vec![1001],
            ),
            (
                "date range",
                ArticleListParams {
                    date_from: Some("2026-01-03".to_string()),
                    date_to: Some("2026-01-05".to_string()),
                    ..article_filter_params()
                },
                vec![1001, 1002, 1005],
            ),
            (
                "doi",
                ArticleListParams {
                    doi: Some("10.1000/doi-only".to_string()),
                    ..article_filter_params()
                },
                vec![1005],
            ),
            (
                "pmid",
                ArticleListParams {
                    pmid: Some("1002".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "fts",
                ArticleListParams {
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1004, 1001],
            ),
            (
                "indexed-only",
                ArticleListParams {
                    q: Some("indexedonly".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "combined",
                ArticleListParams {
                    area: vec!["Medicine".to_string()],
                    open_access: Some(true),
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1001],
            ),
        ];

        for (name, params, expected_ids) in cases {
            let page = list_articles(&fixture.config, Some(&fixture.db_name), &params)
                .unwrap_or_else(|error| panic!("{name} should query successfully: {error}"));
            assert_eq!(article_ids(&page), expected_ids, "{name}");
            assert_eq!(page.page.total, Some(expected_ids.len() as i64), "{name}");
            assert!(page.items.iter().all(|article| !article.authors.is_empty()));
        }
    }

    #[test]
    fn article_listing_cursor_and_candidate_helpers_are_stable() {
        let fixture = IndexFixture::new(true);
        let first_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                limit: 2,
                ..article_filter_params()
            },
        )
        .expect("first page should query");
        assert_eq!(article_ids(&first_page), [1003, 1004]);
        assert_eq!(
            first_page.page.next_cursor.as_deref(),
            Some("2026-01-06|1004")
        );

        let second_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: first_page.page.next_cursor,
                limit: 2,
                ..article_filter_params()
            },
        )
        .expect("second page should query");
        assert_eq!(article_ids(&second_page), [1001, 1002]);

        let db_path = fixture_db_path(&fixture);
        let issue_candidates = fetch_candidates_for_issue_keys(
            &db_path,
            &[build_issue_key(1, 10), build_issue_key(2, 20)],
        )
        .expect("issue candidates should load");
        assert_eq!(
            candidate_ids(&issue_candidates),
            [1003, 1001, 1002, 1005, 1008]
        );
        assert_eq!(
            collect_issue_article_counts(&db_path)
                .expect("counts should load")
                .get("1:10"),
            Some(&4)
        );
        assert_eq!(
            collect_inpress_article_counts(&db_path)
                .expect("in-press counts should load")
                .get("1"),
            Some(&1)
        );
    }

    #[test]
    fn article_reads_plural_retraction_dois_in_lexical_order() {
        let fixture = IndexFixture::new(true);
        let article = get_article(&fixture.config, Some(&fixture.db_name), 1001)
            .expect("article should load");

        assert_eq!(
            article.retraction_dois,
            ["10.1000/retraction-a", "10.1000/retraction-b"]
        );
    }

    #[test]
    fn article_query_errors_are_checked() {
        let fixture = IndexFixture::new(true);
        let invalid_cursor = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: Some("invalid".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("invalid cursor should fail");
        assert!(matches!(
            invalid_cursor,
            IndexRepositoryError::InvalidCursor
        ));

        let unsupported = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                sort: Some("title:asc".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("unsupported sort should fail");
        assert!(
            matches!(unsupported, IndexRepositoryError::UnsupportedSortField(field) if field == "title")
        );

        for key in ["1", "one:10", "1:two", "1:10:20"] {
            assert!(matches!(
                parse_issue_key(key),
                Err(IndexRepositoryError::InvalidCursor)
            ));
        }
    }
}
