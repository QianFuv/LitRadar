//! Article listing, lookup, count, and delivery candidate repositories.

use super::shared::*;
use super::*;

/// Article list filters.
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
    /// Suppressed filter.
    pub suppressed: Option<bool>,
    /// Library holdings filter.
    pub within_library_holdings: Option<bool>,
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
    /// Build default Python-compatible article list parameters.
    fn default() -> Self {
        Self {
            journal_id: Vec::new(),
            issue_id: None,
            year: None,
            area: Vec::new(),
            in_press: None,
            open_access: None,
            suppressed: None,
            within_library_holdings: None,
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
        let journal_id = row.get::<_, i64>(0)?;
        let issue_id = row.get::<_, i64>(1)?;
        let count = row.get::<_, i64>(2)?;
        Ok((build_issue_key(journal_id, issue_id), count))
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
        let journal_id = row.get::<_, i64>(0)?;
        let count = row.get::<_, i64>(1)?;
        Ok((journal_id.to_string(), count))
    })?;
    collect_rows(rows).map(|items| items.into_iter().collect())
}

/// Fetch visible article candidates for issue keys.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `issue_keys` - Pending issue keys.
///
/// # Returns
///
/// Candidate articles ordered like the Python notification query.
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
    fetch_candidates_for_issue_ids(index_db_path, &issue_ids)
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
/// Candidate articles ordered like the Python notification query.
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
    fetch_candidates_for_inpress_journal_ids(index_db_path, &journal_ids)
}

/// Fetch visible article candidates by explicit article identifiers.
///
/// # Arguments
///
/// * `index_db_path` - Path to the selected index database.
/// * `article_ids` - Article identifiers to load.
///
/// # Returns
///
/// Candidate articles ordered like the Python notification query.
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
    fetch_candidates_for_ids(index_db_path, &article_ids)
}

/// List articles with filters.
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
    if is_article_listing_ready(&connection) {
        list_articles_from_listing(&connection, params)
    } else {
        list_articles_from_articles(&connection, params)
    }
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
/// Article record.
pub fn get_article(
    config: &StorageConfig,
    db_name: Option<&str>,
    article_id: i64,
) -> Result<ArticleRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    get_article_from_connection(&connection, article_id)?
        .ok_or(IndexRepositoryError::NotFound("Article not found"))
}

fn list_articles_from_listing(
    connection: &Connection,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
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
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "l.within_library_holdings = ?",
        params.within_library_holdings,
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
    let total = article_total(
        connection,
        params.include_total,
        "article_listing l",
        "",
        &where_sql,
        &values,
    )?;
    let id_rows = article_id_rows(
        connection,
        ArticleIdQuery {
            table_sql: "article_listing l",
            join_sql: "",
            where_sql: &where_sql,
            alias: "l",
            direction,
            values: &values,
            params,
        },
    )?;
    article_page_from_ids(connection, id_rows, total, params)
}

fn list_articles_from_articles(
    connection: &Connection,
    params: &ArticleListParams,
) -> Result<ArticlePage, IndexRepositoryError> {
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_int_list_filter(
        &mut clauses,
        &mut values,
        "a.journal_id",
        &params.journal_id,
    );
    push_optional_int_filter(&mut clauses, &mut values, "a.issue_id = ?", params.issue_id);
    push_string_list_filter(&mut clauses, &mut values, "m.area", &params.area);
    push_optional_bool_filter(&mut clauses, &mut values, "a.in_press = ?", params.in_press);
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.open_access = ?",
        params.open_access,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "a.within_library_holdings = ?",
        params.within_library_holdings,
    );
    push_optional_text_filter(&mut clauses, &mut values, "a.date >= ?", &params.date_from);
    push_optional_text_filter(&mut clauses, &mut values, "a.date <= ?", &params.date_to);
    push_optional_text_filter(&mut clauses, &mut values, "a.doi = ?", &params.doi);
    push_optional_text_filter(&mut clauses, &mut values, "a.pmid = ?", &params.pmid);
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.publication_year = ?",
        params.year,
    );
    if let Some(query) = nonempty(params.q.as_deref()) {
        clauses.push("article_search MATCH ?".to_string());
        values.push(SqlValue::Text(query.to_string()));
    }
    let mut joins = Vec::new();
    if params.year.is_some() {
        joins.push("JOIN issues i ON i.issue_id = a.issue_id");
    }
    if !params.area.is_empty() {
        joins.push("JOIN journal_meta m ON m.journal_id = a.journal_id");
    }
    if nonempty(params.q.as_deref()).is_some() {
        joins.push("JOIN article_search ON article_search.article_id = a.article_id");
    }
    let join_sql = joins.join(" ");
    let direction = article_sort_direction(params.sort.as_deref().unwrap_or("date:desc"))?;
    push_cursor_filter(
        &mut clauses,
        &mut values,
        "a",
        direction,
        params.cursor.as_deref(),
    )?;
    let where_sql = where_sql(&clauses);
    let total = article_total(
        connection,
        params.include_total,
        "articles a",
        &join_sql,
        &where_sql,
        &values,
    )?;
    let id_rows = article_id_rows(
        connection,
        ArticleIdQuery {
            table_sql: "articles a",
            join_sql: &join_sql,
            where_sql: &where_sql,
            alias: "a",
            direction,
            values: &values,
            params,
        },
    )?;
    article_page_from_ids(connection, id_rows, total, params)
}

fn article_total(
    connection: &Connection,
    include_total: bool,
    table_sql: &str,
    join_sql: &str,
    where_sql: &str,
    values: &[SqlValue],
) -> Result<Option<i64>, IndexRepositoryError> {
    if !include_total {
        return Ok(None);
    }
    Ok(Some(connection.query_row(
        &format!("SELECT COUNT(*) FROM {table_sql} {join_sql} {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?))
}

fn article_id_rows(
    connection: &Connection,
    query: ArticleIdQuery<'_>,
) -> Result<Vec<(i64, Option<String>)>, IndexRepositoryError> {
    let mut page_values = query.values.to_vec();
    page_values.push(SqlValue::Integer(query.params.limit));
    let pagination_sql = if query.params.cursor.is_none() {
        page_values.push(SqlValue::Integer(query.params.offset));
        "LIMIT ? OFFSET ?"
    } else {
        "LIMIT ?"
    };
    let order_direction = query.direction.sql();
    let mut statement = connection.prepare(&format!(
        "SELECT {alias}.article_id, {alias}.date FROM {table_sql} {join_sql} {where_sql} \
         ORDER BY {alias}.date {order_direction}, {alias}.article_id {order_direction} {pagination_sql}",
        alias = query.alias,
        table_sql = query.table_sql,
        join_sql = query.join_sql,
        where_sql = query.where_sql,
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
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
    let items = fetch_articles_by_ids(connection, &article_ids)?;
    Ok(ArticlePage {
        items,
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
    let placeholders = placeholders(article_ids.len());
    let values = article_ids
        .iter()
        .copied()
        .map(SqlValue::Integer)
        .collect::<Vec<_>>();
    let mut statement = connection.prepare(&format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors, \
         a.start_page, a.end_page, a.abstract, a.doi, a.pmid, a.permalink, a.suppressed, \
         a.in_press, a.open_access, a.platform_id, a.retraction_doi, \
         a.within_library_holdings, a.content_location, a.full_text_file, \
         j.title AS journal_title, i.volume, i.number \
         FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
         JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({placeholders})"
    ))?;
    let rows = statement.query_map(params_from_iter(values.iter()), article_from_row)?;
    let mut by_id = collect_rows(rows)?
        .into_iter()
        .map(|article: ArticleRecord| (article.article_id.value(), article))
        .collect::<HashMap<_, _>>();
    Ok(article_ids
        .iter()
        .filter_map(|article_id| by_id.remove(article_id))
        .collect())
}

fn get_article_from_connection(
    connection: &Connection,
    article_id: i64,
) -> Result<Option<ArticleRecord>, IndexRepositoryError> {
    let rows = fetch_articles_by_ids(connection, &[article_id])?;
    Ok(rows.into_iter().next())
}

fn article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleRecord> {
    Ok(ArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        date: row.get(4)?,
        authors: row.get(5)?,
        start_page: row.get(6)?,
        end_page: row.get(7)?,
        abstract_text: row.get(8)?,
        doi: row.get(9)?,
        pmid: row.get(10)?,
        permalink: row.get(11)?,
        suppressed: row.get(12)?,
        in_press: row.get(13)?,
        open_access: row.get(14)?,
        platform_id: row.get(15)?,
        retraction_doi: row.get(16)?,
        within_library_holdings: row.get(17)?,
        content_location: row.get(18)?,
        full_text_file: row.get(19)?,
        journal_title: row.get(20)?,
        volume: row.get(21)?,
        number: row.get(22)?,
    })
}

fn article_sort_direction(sort: &str) -> Result<SortDirection, IndexRepositoryError> {
    let specs = sort_specs(sort, &[("date", "date")])?;
    if specs.len() != 1 {
        return Err(IndexRepositoryError::UnsupportedArticleSort);
    }
    Ok(specs[0].direction)
}

fn fetch_candidates_for_issue_ids(
    index_db_path: impl AsRef<Path>,
    issue_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if issue_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(issue_ids.len());
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract, a.date, \
         a.open_access, a.in_press, a.within_library_holdings, a.doi, a.full_text_file, \
         a.permalink, j.title AS journal_title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.issue_id IN ({placeholders}) AND COALESCE(a.suppressed, 0) = 0 \
         ORDER BY a.date DESC, a.article_id DESC"
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(issue_ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn fetch_candidates_for_inpress_journal_ids(
    index_db_path: impl AsRef<Path>,
    journal_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if journal_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(journal_ids.len());
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract, a.date, \
         a.open_access, a.in_press, a.within_library_holdings, a.doi, a.full_text_file, \
         a.permalink, j.title AS journal_title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.issue_id IS NULL AND COALESCE(a.in_press, 0) = 1 \
           AND a.journal_id IN ({placeholders}) AND COALESCE(a.suppressed, 0) = 0 \
         ORDER BY a.date DESC, a.article_id DESC"
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(journal_ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn fetch_candidates_for_ids(
    index_db_path: impl AsRef<Path>,
    article_ids: &[i64],
) -> Result<Vec<ArticleCandidateInfo>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = repeat_placeholders(article_ids.len());
    let sql = format!(
        "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.abstract, a.date, \
         a.open_access, a.in_press, a.within_library_holdings, a.doi, a.full_text_file, \
         a.permalink, j.title AS journal_title \
         FROM articles a JOIN journals j ON j.journal_id = a.journal_id \
         WHERE a.article_id IN ({placeholders}) AND COALESCE(a.suppressed, 0) = 0 \
         ORDER BY a.date DESC, a.article_id DESC"
    );
    let connection = Connection::open(index_db_path)?;
    let mut statement = connection.prepare(&sql)?;
    let rows = statement.query_map(params_from_iter(article_ids.iter()), candidate_from_row)?;
    collect_rows(rows)
}

fn candidate_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ArticleCandidateInfo> {
    Ok(ArticleCandidateInfo {
        article_id: row.get(0)?,
        journal_id: row.get(1)?,
        issue_id: row.get(2)?,
        title: nonempty_owned(row.get::<_, Option<String>>(3)?)
            .unwrap_or_else(|| "Untitled article".to_string()),
        abstract_text: nonempty_owned(row.get::<_, Option<String>>(4)?).unwrap_or_default(),
        date: nonempty_owned(row.get(5)?),
        open_access: row.get::<_, Option<i64>>(6)?.unwrap_or(0) != 0,
        in_press: row.get::<_, Option<i64>>(7)?.unwrap_or(0) != 0,
        within_library_holdings: row.get::<_, Option<i64>>(8)?.unwrap_or(0) != 0,
        doi: nonempty_owned(row.get(9)?),
        full_text_file: nonempty_owned(row.get(10)?),
        permalink: nonempty_owned(row.get(11)?),
        journal_title: nonempty_owned(row.get::<_, Option<String>>(12)?)
            .unwrap_or_else(|| "Unknown journal".to_string()),
    })
}

fn repeat_placeholders(count: usize) -> String {
    (0..count).map(|_| "?").collect::<Vec<_>>().join(", ")
}

fn build_issue_key(journal_id: i64, issue_id: i64) -> String {
    format!("{journal_id}:{issue_id}")
}

fn parse_issue_key(key: &str) -> Result<(i64, i64), IndexRepositoryError> {
    let (journal_id, issue_id) = key
        .split_once(':')
        .ok_or(IndexRepositoryError::InvalidCursor)?;
    let journal_id = journal_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    let issue_id = issue_id
        .parse::<i64>()
        .map_err(|_| IndexRepositoryError::InvalidCursor)?;
    Ok((journal_id, issue_id))
}

#[derive(Debug, Clone, Copy)]
struct ArticleIdQuery<'a> {
    table_sql: &'a str,
    join_sql: &'a str,
    where_sql: &'a str,
    alias: &'a str,
    direction: SortDirection,
    values: &'a [SqlValue],
    params: &'a ArticleListParams,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_support::{
        article_filter_params, article_ids, candidate_ids, fixture_db_path, IndexFixture,
    };

    #[test]
    fn article_listing_filters_cover_fts5_and_supported_expressions() {
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
                vec![1003, 1001, 1002, 1005, 1008],
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
                "library holdings",
                ArticleListParams {
                    within_library_holdings: Some(false),
                    ..article_filter_params()
                },
                vec![1003, 1005, 1008],
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
                    pmid: Some("PMID-1002".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "fts5 title and abstract",
                ArticleListParams {
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1004, 1001],
            ),
            (
                "fts5 indexed-only token",
                ArticleListParams {
                    q: Some("indexedonly".to_string()),
                    ..article_filter_params()
                },
                vec![1002],
            ),
            (
                "combined fts5 and structured filters",
                ArticleListParams {
                    area: vec!["Medicine".to_string()],
                    open_access: Some(true),
                    within_library_holdings: Some(true),
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1001],
            ),
            (
                "suppressed",
                ArticleListParams {
                    suppressed: Some(true),
                    q: Some("genome".to_string()),
                    ..article_filter_params()
                },
                vec![1006],
            ),
        ];

        for (name, params, expected_ids) in cases {
            let page = list_articles(&fixture.config, Some(&fixture.db_name), &params)
                .unwrap_or_else(|error| panic!("{name} should query successfully: {error}"));
            assert_eq!(article_ids(&page), expected_ids, "{name}");
            assert_eq!(page.page.total, Some(expected_ids.len() as i64), "{name}");
        }
    }

    #[test]
    fn article_listing_cursor_and_sort_expression_errors_are_checked() {
        let fixture = IndexFixture::new(true);
        let first_page_params = ArticleListParams {
            limit: 2,
            ..article_filter_params()
        };

        let first_page = list_articles(&fixture.config, Some(&fixture.db_name), &first_page_params)
            .expect("first page should query");

        assert_eq!(article_ids(&first_page), [1003, 1004]);
        assert_eq!(first_page.page.total, Some(6));
        assert_eq!(first_page.page.has_more, Some(true));
        assert_eq!(
            first_page.page.next_cursor.as_deref(),
            Some("2026-01-06|1004")
        );

        let second_page_params = ArticleListParams {
            cursor: first_page.page.next_cursor,
            limit: 2,
            ..article_filter_params()
        };
        let second_page =
            list_articles(&fixture.config, Some(&fixture.db_name), &second_page_params)
                .expect("second page should query");
        assert_eq!(article_ids(&second_page), [1001, 1002]);

        let invalid_cursor = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                cursor: Some("not-a-cursor".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("invalid cursor should fail");
        assert!(matches!(
            invalid_cursor,
            IndexRepositoryError::InvalidCursor
        ));

        let unsupported_field = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                sort: Some("title:asc".to_string()),
                ..article_filter_params()
            },
        )
        .expect_err("unsupported article sort field should fail");
        assert!(matches!(
            unsupported_field,
            IndexRepositoryError::UnsupportedSortField(field) if field == "title"
        ));

        let empty_sort = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                sort: Some(String::new()),
                ..article_filter_params()
            },
        )
        .expect_err("empty article sort should fail");
        assert!(matches!(
            empty_sort,
            IndexRepositoryError::UnsupportedArticleSort
        ));
    }

    #[test]
    fn article_fallback_query_uses_fts5_with_joined_filters_when_listing_is_not_ready() {
        let fixture = IndexFixture::new(false);
        let indexed_only = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string()],
                q: Some("indexedonly".to_string()),
                ..article_filter_params()
            },
        )
        .expect("fallback search should use FTS5");
        assert_eq!(article_ids(&indexed_only), [1002]);
        assert_eq!(indexed_only.page.total, Some(1));

        let joined = list_articles(
            &fixture.config,
            Some(&fixture.db_name),
            &ArticleListParams {
                area: vec!["Medicine".to_string()],
                year: Some(2026),
                q: Some("genome".to_string()),
                ..article_filter_params()
            },
        )
        .expect("fallback search should join FTS5, issues, and metadata");
        assert_eq!(article_ids(&joined), [1001]);
    }
    #[test]
    fn index_counts_and_candidate_helpers_cover_visible_rows_and_errors() {
        let fixture = IndexFixture::new(true);
        let db_path = fixture_db_path(&fixture);

        let issue_counts =
            collect_issue_article_counts(&db_path).expect("issue counts should be collected");
        assert_eq!(issue_counts.len(), 3);
        assert_eq!(issue_counts.get("1:10"), Some(&4));
        assert_eq!(issue_counts.get("1:11"), Some(&1));
        assert_eq!(issue_counts.get("2:20"), Some(&1));

        let inpress_counts =
            collect_inpress_article_counts(&db_path).expect("in-press counts should be collected");
        assert_eq!(inpress_counts.len(), 1);
        assert_eq!(inpress_counts.get("1"), Some(&1));

        let issue_candidates = fetch_candidates_for_issue_keys(
            &db_path,
            &[
                build_issue_key(1, 10),
                build_issue_key(2, 20),
                build_issue_key(1, 10),
            ],
        )
        .expect("issue candidates should be fetched");
        assert_eq!(
            candidate_ids(&issue_candidates),
            vec![1003, 1001, 1002, 1005, 1008]
        );
        assert_eq!(issue_candidates[0].journal_title, "Beta CNKI");
        assert_eq!(issue_candidates[1].doi.as_deref(), Some("10.1000/genome"));
        assert!(issue_candidates[1].open_access);
        assert!(issue_candidates[1].within_library_holdings);

        let suppressed_issue_candidates =
            fetch_candidates_for_issue_keys(&db_path, &[build_issue_key(1, 11)])
                .expect("suppressed issue query should resolve");
        assert!(suppressed_issue_candidates.is_empty());

        let inpress_candidates =
            fetch_candidates_for_inpress_keys(&db_path, &["1".to_string(), "1".to_string()])
                .expect("in-press candidates should be fetched");
        assert_eq!(candidate_ids(&inpress_candidates), vec![1004]);
        assert!(inpress_candidates[0].in_press);
        assert_eq!(inpress_candidates[0].issue_id, None);

        let article_id_candidates = fetch_candidates_for_article_ids(&db_path, &[1002, 1001, 1002])
            .expect("article id candidates should be fetched");
        assert_eq!(candidate_ids(&article_id_candidates), vec![1001, 1002]);

        assert!(fetch_candidates_for_issue_keys(&db_path, &[])
            .expect("empty issue keys should resolve")
            .is_empty());
        assert!(fetch_candidates_for_inpress_keys(&db_path, &[])
            .expect("empty in-press keys should resolve")
            .is_empty());
        assert!(fetch_candidates_for_article_ids(&db_path, &[])
            .expect("empty article ids should resolve")
            .is_empty());

        let invalid_issue_key = fetch_candidates_for_issue_keys(&db_path, &["bad".to_string()])
            .expect_err("invalid issue key should fail");
        assert!(matches!(
            invalid_issue_key,
            IndexRepositoryError::InvalidCursor
        ));

        let invalid_inpress_key = fetch_candidates_for_inpress_keys(&db_path, &["bad".to_string()])
            .expect_err("invalid in-press key should fail");
        assert!(matches!(
            invalid_inpress_key,
            IndexRepositoryError::InvalidCursor
        ));
    }
    #[test]
    fn article_key_helpers_cover_valid_and_invalid_keys() {
        assert_eq!(build_issue_key(1, 10), "1:10");
        assert_eq!(
            parse_issue_key("1:10").expect("valid issue key should parse"),
            (1, 10)
        );
        for key in ["1", "one:10", "1:two", "1:10:20"] {
            let error = parse_issue_key(key).expect_err("invalid key should fail");
            assert!(matches!(error, IndexRepositoryError::InvalidCursor));
        }
    }
}
