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
    /// FTS query interpretation mode.
    pub search_mode: ArticleSearchMode,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
    /// Cursor string.
    pub cursor: Option<String>,
    /// Whether to include total count, with a mode-specific default when absent.
    pub include_total: Option<bool>,
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
            search_mode: ArticleSearchMode::default(),
            sort: Some("date:desc".to_string()),
            limit: 50,
            offset: 0,
            cursor: None,
            include_total: None,
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
    validate_article_list_input(db_name, params)?;
    let connection = open_index_connection(config, db_name)?;
    let mut base_clauses = Vec::new();
    let mut base_values = Vec::new();
    push_int_list_filter(
        &mut base_clauses,
        &mut base_values,
        "l.journal_id",
        &params.journal_id,
    );
    push_optional_int_filter(
        &mut base_clauses,
        &mut base_values,
        "l.issue_id = ?",
        params.issue_id,
    );
    push_string_list_filter(&mut base_clauses, &mut base_values, "l.area", &params.area);
    push_optional_bool_filter(
        &mut base_clauses,
        &mut base_values,
        "l.in_press = ?",
        params.in_press,
    );
    push_optional_bool_filter(
        &mut base_clauses,
        &mut base_values,
        "l.open_access = ?",
        params.open_access,
    );
    push_optional_text_filter(
        &mut base_clauses,
        &mut base_values,
        "l.date >= ?",
        &params.date_from,
    );
    push_optional_text_filter(
        &mut base_clauses,
        &mut base_values,
        "l.date <= ?",
        &params.date_to,
    );
    push_optional_text_filter(
        &mut base_clauses,
        &mut base_values,
        "l.doi = ?",
        &params.doi,
    );
    push_optional_text_filter(
        &mut base_clauses,
        &mut base_values,
        "l.pmid = ?",
        &params.pmid,
    );
    push_optional_int_filter(
        &mut base_clauses,
        &mut base_values,
        "l.publication_year = ?",
        params.year,
    );
    push_fts_filter(
        &mut base_clauses,
        &mut base_values,
        "l.article_id",
        &params.q,
        params.search_mode,
    );
    let direction = article_sort_direction(params.sort.as_deref().unwrap_or("date:desc"))?;
    let base_where_sql = where_sql(&base_clauses);
    let total = if should_include_total(params) {
        Some(article_total(
            &connection,
            &base_where_sql,
            &base_values,
            params,
        )?)
    } else {
        None
    };
    let mut page_clauses = base_clauses;
    let mut page_values = base_values;
    push_cursor_filter(
        &mut page_clauses,
        &mut page_values,
        "l",
        direction,
        params.cursor.as_deref(),
    )?;
    let page_where_sql = where_sql(&page_clauses);
    let id_rows = article_id_rows(
        &connection,
        &page_where_sql,
        direction,
        &page_values,
        params,
    )?;
    article_page_from_ids(&connection, id_rows, total, params)
}

fn validate_article_list_input(
    db_name: Option<&str>,
    params: &ArticleListParams,
) -> Result<(), IndexRepositoryError> {
    let validation_result = (|| {
        if let Some(db_name) = db_name {
            validate_characters("db", db_name, MAX_DATABASE_NAME_CHARS)?;
        }
        validate_item_count(
            "search filters",
            params.journal_id.len().saturating_add(params.area.len()),
            MAX_SEARCH_FILTER_ITEMS,
        )?;
        for area in &params.area {
            validate_characters("area", area, MAX_SEARCH_TEXT_CHARS)?;
        }
        for (label, value) in [
            ("date_from", params.date_from.as_deref()),
            ("date_to", params.date_to.as_deref()),
            ("doi", params.doi.as_deref()),
            ("pmid", params.pmid.as_deref()),
            ("q", params.q.as_deref()),
            ("sort", params.sort.as_deref()),
            ("cursor", params.cursor.as_deref()),
        ] {
            if let Some(value) = value {
                validate_characters(label, value, MAX_SEARCH_TEXT_CHARS)?;
            }
        }
        Ok::<(), litradar_domain::InputValidationError>(())
    })();
    validation_result.map_err(|error| IndexRepositoryError::InvalidInput(error.to_string()))
}

fn should_include_total(params: &ArticleListParams) -> bool {
    params
        .include_total
        .unwrap_or_else(|| params.cursor.is_none())
}

fn article_total(
    connection: &Connection,
    where_sql: &str,
    values: &[SqlValue],
    params: &ArticleListParams,
) -> Result<i64, IndexRepositoryError> {
    #[cfg(test)]
    ARTICLE_TOTAL_QUERY_COUNT.with(|count| count.set(count.get() + 1));
    connection
        .query_row(
            &format!("SELECT COUNT(*) FROM article_listing l {where_sql}"),
            params_from_iter(values.iter()),
            |row| row.get(0),
        )
        .map_err(IndexRepositoryError::from)
        .map_err(|error| classify_article_query_error(error, params))
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
    page_values.push(SqlValue::Integer(params.limit + 1));
    let pagination_sql = if params.cursor.is_none() {
        page_values.push(SqlValue::Integer(params.offset));
        "LIMIT ? OFFSET ?"
    } else {
        "LIMIT ?"
    };
    let order_direction = direction.sql();
    let mut statement = connection
        .prepare(&article_id_query_sql(
            where_sql,
            order_direction,
            pagination_sql,
        ))
        .map_err(IndexRepositoryError::from)
        .map_err(|error| classify_article_query_error(error, params))?;
    let rows = statement
        .query_map(params_from_iter(page_values.iter()), |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(IndexRepositoryError::from)
        .map_err(|error| classify_article_query_error(error, params))?;
    collect_rows(rows).map_err(|error| classify_article_query_error(error, params))
}

fn article_id_query_sql(where_sql: &str, order_direction: &str, pagination_sql: &str) -> String {
    format!(
        "SELECT l.article_id, l.date FROM article_listing l {where_sql} \
         ORDER BY l.date {order_direction}, l.article_id {order_direction} {pagination_sql}"
    )
}

fn article_page_from_ids(
    connection: &Connection,
    mut id_rows: Vec<(i64, Option<String>)>,
    total: Option<i64>,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    let has_more = id_rows.len() as i64 > params.limit;
    id_rows.truncate(params.limit as usize);
    let next_cursor = if has_more {
        id_rows.last().map(|(article_id, date)| {
            format!("{}|{article_id}", date.as_deref().unwrap_or_default())
        })
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
            Some(has_more),
        ),
    })
}

fn classify_article_query_error(
    error: IndexRepositoryError,
    params: &ArticleListParams,
) -> IndexRepositoryError {
    if params.search_mode == ArticleSearchMode::Advanced
        && params
            .q
            .as_deref()
            .is_some_and(|query| !query.trim().is_empty())
        && matches!(&error, IndexRepositoryError::Sqlite(error) if is_fts_expression_error(error))
    {
        IndexRepositoryError::InvalidSearchExpression
    } else {
        error
    }
}

fn is_fts_expression_error(error: &rusqlite::Error) -> bool {
    let rusqlite::Error::SqliteFailure(_, Some(message)) = error else {
        return false;
    };
    let message = message.to_ascii_lowercase();
    message.contains("fts5: syntax error")
        || message.contains("malformed match expression")
        || message.contains("unterminated string")
        || message.starts_with("no such column:")
}

#[cfg(test)]
thread_local! {
    static ARTICLE_TOTAL_QUERY_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
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
    let date = row.get::<_, Option<String>>(5)?;
    Ok(ArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        publication_year: row.get(4)?,
        date_precision: date.as_deref().and_then(litradar_domain::date_precision),
        date,
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

    #[cfg(any(windows, target_os = "linux"))]
    #[test]
    fn article_queries_ignore_obsolete_simple_extension_assets() {
        let fixture = IndexFixture::new(true);
        let extension_path = if cfg!(windows) {
            fixture
                .config
                .project_root()
                .join("libs")
                .join("simple-windows")
                .join("libsimple-windows-x64")
                .join("simple.dll")
        } else {
            fixture
                .config
                .project_root()
                .join("libs")
                .join("simple-linux")
                .join("libsimple-linux-ubuntu-latest")
                .join("libsimple.so")
        };
        std::fs::create_dir_all(
            extension_path
                .parent()
                .expect("extension path should have a parent"),
        )
        .expect("historical extension directory should be created");
        std::fs::write(&extension_path, b"invalid historical extension")
            .expect("invalid historical extension should be written");

        let page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                q: Some("genome".to_string()),
                ..article_filter_params()
            },
        )
        .expect("current unicode61 index should ignore unrelated native assets");

        assert_eq!(article_ids(&page), [1004, 1001]);
    }

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
    fn article_listing_uses_a_sentinel_and_full_filter_total() {
        let fixture = IndexFixture::new(true);
        let exact_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                limit: 6,
                ..article_filter_params()
            },
        )
        .expect("an exact-limit page should query");
        assert_eq!(exact_page.items.len(), 6);
        assert_eq!(exact_page.page.has_more, Some(false));
        assert_eq!(exact_page.page.next_cursor, None);

        ARTICLE_TOTAL_QUERY_COUNT.with(|count| count.set(0));
        let first_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                limit: 2,
                ..article_filter_params()
            },
        )
        .expect("the first cursor page should query");
        assert_eq!(first_page.page.total, Some(6));
        assert_eq!(first_page.page.has_more, Some(true));
        assert_eq!(ARTICLE_TOTAL_QUERY_COUNT.with(std::cell::Cell::get), 1);

        let cursor = first_page
            .page
            .next_cursor
            .clone()
            .expect("the first page should provide a cursor");
        ARTICLE_TOTAL_QUERY_COUNT.with(|count| count.set(0));
        let default_cursor_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: Some(cursor.clone()),
                limit: 2,
                ..article_filter_params()
            },
        )
        .expect("the default cursor page should query");
        assert_eq!(default_cursor_page.page.total, None);
        assert_eq!(ARTICLE_TOTAL_QUERY_COUNT.with(std::cell::Cell::get), 0);

        ARTICLE_TOTAL_QUERY_COUNT.with(|count| count.set(0));
        let explicit_total_page = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: Some(cursor),
                include_total: Some(true),
                limit: 2,
                ..article_filter_params()
            },
        )
        .expect("an explicit cursor total should query");
        assert_eq!(explicit_total_page.page.total, Some(6));
        assert_eq!(ARTICLE_TOTAL_QUERY_COUNT.with(std::cell::Cell::get), 1);
    }

    #[test]
    fn article_cursor_pages_cover_null_dates_in_both_directions() {
        let fixture = IndexFixture::new(true);
        let connection = open_sqlite_connection(fixture_db_path(&fixture))
            .expect("fixture database should open");
        connection
            .execute_batch(
                "UPDATE articles SET date = NULL WHERE article_id IN (1005, 1008);
                 UPDATE article_listing SET date = NULL WHERE article_id IN (1005, 1008);",
            )
            .expect("NULL date fixtures should update");
        drop(connection);

        for (sort, expected_ids) in [
            ("date:desc", vec![1003, 1004, 1001, 1002, 1008, 1005]),
            ("date:asc", vec![1005, 1008, 1002, 1001, 1004, 1003]),
        ] {
            let mut cursor = None;
            let mut collected_ids = Vec::new();
            for _ in 0..10 {
                let page = list_articles(
                    &fixture.config,
                    Some(&fixture.db_name),
                    &ArticleListParams {
                        cursor,
                        include_total: Some(false),
                        limit: 1,
                        sort: Some(sort.to_string()),
                        ..article_filter_params()
                    },
                )
                .unwrap_or_else(|error| panic!("{sort} cursor page should query: {error}"));
                collected_ids.extend(article_ids(&page));
                if page.page.has_more != Some(true) {
                    break;
                }
                cursor = page.page.next_cursor;
            }
            assert_eq!(collected_ids, expected_ids, "{sort}");
            let unique_ids = collected_ids.iter().copied().collect::<HashSet<_>>();
            assert_eq!(unique_ids.len(), collected_ids.len(), "{sort}");
        }
    }

    #[test]
    fn article_cursor_first_page_uses_sorting_indexes() {
        let fixture = IndexFixture::new(true);
        let connection = open_sqlite_connection(fixture_db_path(&fixture))
            .expect("fixture database should open");

        for (where_sql, expected_index) in [
            ("", "idx_article_listing_date_id"),
            (
                "WHERE l.journal_id = 1",
                "idx_article_listing_journal_date_id",
            ),
        ] {
            for direction in [SortDirection::Asc, SortDirection::Desc] {
                let query = article_id_query_sql(where_sql, direction.sql(), "LIMIT 21 OFFSET 0");
                let mut statement = connection
                    .prepare(&format!("EXPLAIN QUERY PLAN {query}"))
                    .expect("article query plan should prepare");
                let details = statement
                    .query_map([], |row| row.get::<_, String>(3))
                    .expect("article query plan should run")
                    .collect::<rusqlite::Result<Vec<_>>>()
                    .expect("article query plan should collect");

                assert!(
                    details.iter().any(|detail| detail.contains(expected_index)),
                    "expected {expected_index} in {details:?}"
                );
                assert!(
                    details
                        .iter()
                        .all(|detail| !detail.contains("USE TEMP B-TREE FOR ORDER BY")),
                    "unexpected temporary sort in {details:?}"
                );
            }
        }
    }

    #[test]
    fn article_search_modes_separate_literal_text_from_fts_syntax() {
        let fixture = IndexFixture::new(true);
        assert_eq!(
            quote_fts_phrase("genome \"methods\""),
            "\"genome \"\"methods\"\"\""
        );

        let simple = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                q: Some("genome OR clinical".to_string()),
                ..article_filter_params()
            },
        )
        .expect("simple search punctuation should be escaped");
        assert!(simple.items.is_empty());

        let advanced = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                q: Some("genome OR clinical".to_string()),
                search_mode: ArticleSearchMode::Advanced,
                ..article_filter_params()
            },
        )
        .expect("valid advanced syntax should query");
        assert_eq!(article_ids(&advanced), [1004, 1001, 1002]);

        let malformed = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                q: Some("\"".to_string()),
                search_mode: ArticleSearchMode::Advanced,
                ..article_filter_params()
            },
        )
        .expect_err("malformed advanced syntax should fail safely");
        assert!(matches!(
            malformed,
            IndexRepositoryError::InvalidSearchExpression
        ));
    }

    #[test]
    fn article_search_input_bounds_apply_before_sql() {
        let fixture = IndexFixture::new(true);
        let accepted = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string(); MAX_SEARCH_FILTER_ITEMS],
                ..article_filter_params()
            },
        )
        .expect("the repeated-filter boundary should query");
        assert_eq!(accepted.items.len(), 5);

        let too_many = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string(); MAX_SEARCH_FILTER_ITEMS + 1],
                ..article_filter_params()
            },
        )
        .expect_err("one repeated filter over the boundary should fail");
        assert!(matches!(too_many, IndexRepositoryError::InvalidInput(_)));

        let too_long = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                q: Some("文".repeat(MAX_SEARCH_TEXT_CHARS + 1)),
                ..article_filter_params()
            },
        )
        .expect_err("one search character over the boundary should fail");
        assert!(matches!(too_long, IndexRepositoryError::InvalidInput(_)));
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
        assert_eq!(article.date.as_deref(), Some("2026-01-05"));
        assert_eq!(
            article.date_precision,
            Some(litradar_domain::DatePrecision::Day)
        );
    }

    #[test]
    fn article_date_precision_is_derived_without_a_content_migration() {
        let fixture = IndexFixture::new(true);
        let connection = open_sqlite_connection(fixture_db_path(&fixture))
            .expect("fixture database should open");
        connection
            .execute(
                "UPDATE articles SET date = '2026' WHERE article_id = 1001",
                [],
            )
            .expect("partial date should update");

        let year_only = get_article(&fixture.config, Some(&fixture.db_name), 1001)
            .expect("year-only article should load");
        assert_eq!(year_only.date.as_deref(), Some("2026"));
        assert_eq!(
            year_only.date_precision,
            Some(litradar_domain::DatePrecision::Year)
        );

        connection
            .execute(
                "UPDATE articles SET date = '2026-02-31' WHERE article_id = 1001",
                [],
            )
            .expect("legacy invalid date should update");
        let legacy_invalid = get_article(&fixture.config, Some(&fixture.db_name), 1001)
            .expect("legacy article should remain readable");
        assert_eq!(legacy_invalid.date_precision, None);
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
