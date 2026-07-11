//! Index metadata, journal, and issue repositories.

use super::shared::*;
use super::*;

/// Journal list filters.
#[derive(Debug, Clone, Default)]
pub struct JournalListParams {
    /// Area filter.
    pub area: Option<String>,
    /// Library identifier filter.
    pub library_id: Option<String>,
    /// Available filter.
    pub available: Option<bool>,
    /// Has-articles filter.
    pub has_articles: Option<bool>,
    /// Publication year filter.
    pub year: Option<i64>,
    /// Minimum Scimago rank.
    pub scimago_min: Option<f64>,
    /// Maximum Scimago rank.
    pub scimago_max: Option<f64>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
}

/// Issue list filters.
#[derive(Debug, Clone, Default)]
pub struct IssueListParams {
    /// Journal identifier filter.
    pub journal_id: Option<i64>,
    /// Publication year filter.
    pub year: Option<i64>,
    /// Valid issue filter.
    pub is_valid_issue: Option<bool>,
    /// Suppressed filter.
    pub suppressed: Option<bool>,
    /// Embargoed filter.
    pub embargoed: Option<bool>,
    /// Subscription filter.
    pub within_subscription: Option<bool>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
}

/// List available index database filenames.
///
/// # Arguments
///
/// * `config` - Storage paths.
///
/// # Returns
///
/// Sorted database filenames.
pub fn list_index_database_names(
    config: &StorageConfig,
) -> Result<Vec<String>, IndexRepositoryError> {
    Ok(config
        .list_index_databases()?
        .into_iter()
        .filter_map(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .map(str::to_string)
        })
        .collect())
}

/// List journal areas.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Area counts.
pub fn list_areas(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<ValueCount>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT area AS value, COUNT(*) AS count FROM journal_meta \
         WHERE area IS NOT NULL AND area != '' GROUP BY area ORDER BY value ASC",
    )?;
    let rows = statement.query_map([], value_count_from_row)?;
    collect_rows(rows)
}

/// List journal options.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Journal options.
pub fn list_journal_options(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<JournalOption>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement =
        connection.prepare("SELECT journal_id, title FROM journals ORDER BY title ASC")?;
    let rows = statement.query_map([], |row| {
        Ok(JournalOption {
            journal_id: JournalId(row.get(0)?),
            title: row.get(1)?,
        })
    })?;
    collect_rows(rows)
}

/// List metadata sources.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Source counts.
pub fn list_sources(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<ValueCount>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT csv_library AS value, COUNT(*) AS count FROM journal_meta \
         WHERE csv_library IS NOT NULL AND csv_library != '' \
         GROUP BY csv_library ORDER BY count DESC, value ASC",
    )?;
    let rows = statement.query_map([], value_count_from_row)?;
    collect_rows(rows)
}

/// List publication year summaries.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Year summaries.
pub fn list_years(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<Vec<YearSummary>, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut statement = connection.prepare(
        "SELECT CAST(strftime('%Y', date) AS INTEGER) AS year, \
         COUNT(DISTINCT issue_id) AS issue_count, COUNT(DISTINCT journal_id) AS journal_count \
         FROM issues WHERE date IS NOT NULL GROUP BY year ORDER BY year DESC",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(YearSummary {
            year: row.get(0)?,
            issue_count: row.get(1)?,
            journal_count: row.get(2)?,
        })
    })?;
    collect_rows(rows)
}

/// List journals with filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Journal filters.
///
/// # Returns
///
/// Paginated journal response.
pub fn list_journals(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &JournalListParams,
) -> Result<JournalPage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_optional_text_filter(&mut clauses, &mut values, "m.area = ?", &params.area);
    push_optional_text_filter(
        &mut clauses,
        &mut values,
        "j.library_id = ?",
        &params.library_id,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "j.available = ?",
        params.available,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "j.has_articles = ?",
        params.has_articles,
    );
    if let Some(value) = params.scimago_min {
        clauses.push("j.scimago_rank >= ?".to_string());
        values.push(SqlValue::Real(value));
    }
    if let Some(value) = params.scimago_max {
        clauses.push("j.scimago_rank <= ?".to_string());
        values.push(SqlValue::Real(value));
    }
    if let Some(year) = params.year {
        clauses.push(
            "EXISTS (SELECT 1 FROM issues i WHERE i.journal_id = j.journal_id AND i.publication_year = ?)"
                .to_string(),
        );
        values.push(SqlValue::Integer(year));
    }
    let where_sql = where_sql(&clauses);
    let order_sql = sort_sql(
        params.sort.as_deref().unwrap_or("scimago_rank:desc"),
        &[
            ("journal_id", "j.journal_id"),
            ("title", "j.title"),
            ("issn", "j.issn"),
            ("eissn", "j.eissn"),
            ("scimago_rank", "j.scimago_rank"),
            ("available", "j.available"),
            ("has_articles", "j.has_articles"),
        ],
    )?;
    let total: i64 = connection.query_row(
        &format!(
            "SELECT COUNT(*) FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id {where_sql}"
        ),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values.clone();
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT j.journal_id, j.library_id, j.platform_journal_id, j.title, j.issn, j.eissn, \
         j.scimago_rank, j.cover_url, j.available, j.toc_data_approved_and_live, j.has_articles, \
         m.source_csv, m.area, m.csv_title, m.csv_issn, m.csv_library \
         FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id \
         {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), journal_from_row)?;
    Ok(JournalPage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one journal.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `journal_id` - Journal identifier.
///
/// # Returns
///
/// Journal record.
pub fn get_journal(
    config: &StorageConfig,
    db_name: Option<&str>,
    journal_id: i64,
) -> Result<JournalRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    connection
        .query_row(
            "SELECT j.journal_id, j.library_id, j.platform_journal_id, j.title, j.issn, j.eissn, \
             j.scimago_rank, j.cover_url, j.available, j.toc_data_approved_and_live, j.has_articles, \
             m.source_csv, m.area, m.csv_title, m.csv_issn, m.csv_library \
             FROM journals j LEFT JOIN journal_meta m ON j.journal_id = m.journal_id \
             WHERE j.journal_id = ?",
            [journal_id],
            journal_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Journal not found"))
}

/// List issues with filters.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `params` - Issue filters.
///
/// # Returns
///
/// Paginated issue response.
pub fn list_issues(
    config: &StorageConfig,
    db_name: Option<&str>,
    params: &IssueListParams,
) -> Result<IssuePage, IndexRepositoryError> {
    validate_limit_offset(params.limit, params.offset)?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.journal_id = ?",
        params.journal_id,
    );
    push_optional_int_filter(
        &mut clauses,
        &mut values,
        "i.publication_year = ?",
        params.year,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.is_valid_issue = ?",
        params.is_valid_issue,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.suppressed = ?",
        params.suppressed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.embargoed = ?",
        params.embargoed,
    );
    push_optional_bool_filter(
        &mut clauses,
        &mut values,
        "i.within_subscription = ?",
        params.within_subscription,
    );
    let where_sql = where_sql(&clauses);
    let order_sql = sort_sql(
        params.sort.as_deref().unwrap_or("publication_year:desc"),
        &[
            ("issue_id", "i.issue_id"),
            ("publication_year", "i.publication_year"),
            ("title", "i.title"),
            ("date", "i.date"),
            ("volume", "i.volume"),
            ("number", "i.number"),
        ],
    )?;
    let total: i64 = connection.query_row(
        &format!("SELECT COUNT(*) FROM issues i {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values.clone();
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT i.issue_id, i.journal_id, i.publication_year, i.title, i.volume, i.number, \
         i.date, i.is_valid_issue, i.suppressed, i.embargoed, i.within_subscription \
         FROM issues i {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), issue_from_row)?;
    Ok(IssuePage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one issue.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
/// * `issue_id` - Issue identifier.
///
/// # Returns
///
/// Issue record.
pub fn get_issue(
    config: &StorageConfig,
    db_name: Option<&str>,
    issue_id: i64,
) -> Result<IssueRecord, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    connection
        .query_row(
            "SELECT issue_id, journal_id, publication_year, title, volume, number, date, \
             is_valid_issue, suppressed, embargoed, within_subscription \
             FROM issues WHERE issue_id = ?",
            [issue_id],
            issue_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Issue not found"))
}

fn value_count_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ValueCount> {
    Ok(ValueCount {
        value: row.get(0)?,
        count: row.get(1)?,
    })
}

fn journal_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<JournalRecord> {
    Ok(JournalRecord {
        journal_id: JournalId(row.get(0)?),
        library_id: row.get(1)?,
        platform_journal_id: row.get(2)?,
        title: row.get(3)?,
        issn: row.get(4)?,
        eissn: row.get(5)?,
        scimago_rank: row.get(6)?,
        cover_url: row.get(7)?,
        available: row.get(8)?,
        toc_data_approved_and_live: row.get(9)?,
        has_articles: row.get(10)?,
        source_csv: row.get(11)?,
        area: row.get(12)?,
        csv_title: row.get(13)?,
        csv_issn: row.get(14)?,
        csv_library: row.get(15)?,
    })
}

fn issue_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueRecord> {
    Ok(IssueRecord {
        issue_id: row.get(0)?,
        journal_id: JournalId(row.get(1)?),
        publication_year: row.get(2)?,
        title: row.get(3)?,
        volume: row.get(4)?,
        number: row.get(5)?,
        date: row.get(6)?,
        is_valid_issue: row.get(7)?,
        suppressed: row.get(8)?,
        embargoed: row.get(9)?,
        within_subscription: row.get(10)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_support::{value_counts, IndexFixture};

    #[test]
    fn journal_metadata_filters_cover_available_query_options() {
        let fixture = IndexFixture::new(true);

        assert_eq!(
            list_index_database_names(&fixture.config).expect("databases should list"),
            ["fixture.sqlite"]
        );
        assert_eq!(
            value_counts(list_areas(&fixture.config, Some(&fixture.db_name)).expect("areas")),
            [("Engineering".to_string(), 1), ("Medicine".to_string(), 2)]
        );
        assert_eq!(
            value_counts(list_sources(&fixture.config, Some(&fixture.db_name)).expect("sources")),
            [("scholarly".to_string(), 2), ("cnki".to_string(), 1)]
        );

        let years = list_years(&fixture.config, Some(&fixture.db_name)).expect("years");
        assert_eq!(years[0].year, 2026);
        assert_eq!(years[0].issue_count, 2);
        assert_eq!(years[0].journal_count, 2);
        assert_eq!(years[1].year, 2025);

        let options =
            list_journal_options(&fixture.config, Some(&fixture.db_name)).expect("options");
        assert_eq!(
            options
                .iter()
                .map(|option| option.title.as_deref())
                .collect::<Vec<_>>(),
            vec![
                Some("Alpha Journal"),
                Some("Beta CNKI"),
                Some("Gamma Hidden")
            ]
        );

        let page = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                area: Some("Medicine".to_string()),
                library_id: Some("scholarly".to_string()),
                available: Some(true),
                has_articles: Some(true),
                year: Some(2026),
                scimago_min: Some(5.0),
                scimago_max: Some(11.0),
                sort: Some("title:asc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("journal filters should apply");
        assert_eq!(page.page.total, Some(1));
        assert_eq!(page.items[0].journal_id.value(), 1);
        assert_eq!(page.items[0].platform_journal_id, None);

        let unavailable = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                area: Some("Medicine".to_string()),
                available: Some(false),
                sort: Some("title:asc".to_string()),
                limit: 10,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect("availability filter should apply");
        assert_eq!(unavailable.items[0].journal_id.value(), 3);

        let issue_page = list_issues(
            &fixture.config,
            Some(&fixture.db_name),
            &IssueListParams {
                journal_id: Some(1),
                year: Some(2026),
                is_valid_issue: Some(true),
                suppressed: Some(false),
                embargoed: Some(false),
                within_subscription: Some(true),
                sort: Some("date:desc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("issue filters should apply");
        assert_eq!(issue_page.page.total, Some(1));
        assert_eq!(issue_page.items[0].issue_id, 10);

        let sort_error = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                sort: Some("unknown:asc".to_string()),
                limit: 10,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect_err("unsupported journal sort should fail");
        assert!(matches!(
            sort_error,
            IndexRepositoryError::UnsupportedSortField(field) if field == "unknown"
        ));

        let limit_error = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                limit: 0,
                offset: 0,
                ..JournalListParams::default()
            },
        )
        .expect_err("invalid limit should fail");
        assert!(matches!(
            limit_error,
            IndexRepositoryError::InvalidPagination("limit must be between 1 and 200")
        ));
    }
}
