//! Provider-neutral journal and issue repositories.

use super::shared::*;
use super::*;

/// Journal list filters backed only by canonical content fields.
#[derive(Debug, Clone, Default)]
pub struct JournalListParams {
    /// Area filter.
    pub area: Option<String>,
    /// Exact journal grades, intersected with other journal filters.
    pub ratings: JournalRatingFilters,
    /// Whether the journal currently has indexed articles.
    pub has_articles: Option<bool>,
    /// Publication year filter.
    pub year: Option<i64>,
    /// Sort string.
    pub sort: Option<String>,
    /// Limit.
    pub limit: i64,
    /// Offset.
    pub offset: i64,
}

/// Issue list filters backed only by canonical content fields.
#[derive(Debug, Clone, Default)]
pub struct IssueListParams {
    /// Journal identifier filter.
    pub journal_id: Option<i64>,
    /// Publication year filter.
    pub year: Option<i64>,
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

/// List canonical journal areas.
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
        "SELECT area, COUNT(*) FROM journals \
         WHERE area IS NOT NULL AND area != '' GROUP BY area ORDER BY area ASC",
    )?;
    let rows = statement.query_map([], value_count_from_row)?;
    collect_rows(rows)
}

/// List exact rating choices with journal counts, including journals without articles.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `db_name` - Optional database name.
///
/// # Returns
///
/// Four stable rating groups; unrated systems have empty lists.
pub fn list_journal_ratings(
    config: &StorageConfig,
    db_name: Option<&str>,
) -> Result<JournalRatingOptions, IndexRepositoryError> {
    let connection = open_index_connection(config, db_name)?;
    let mut options = JournalRatingOptions::default();
    for (field, target) in [
        ("utd_rating", &mut options.utd_rating),
        ("abs_rating", &mut options.abs_rating),
        ("fms_rating", &mut options.fms_rating),
        ("fmscn_rating", &mut options.fmscn_rating),
    ] {
        let mut statement = connection.prepare(&format!(
            "SELECT {field}, COUNT(*) FROM journals WHERE {field} IS NOT NULL \
             AND {field} != '' GROUP BY {field} ORDER BY {field}"
        ))?;
        *target = collect_rows(statement.query_map([], value_count_from_row)?)?;
    }
    Ok(options)
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
        "SELECT publication_year, COUNT(DISTINCT issue_id), COUNT(DISTINCT journal_id) \
         FROM issues WHERE publication_year IS NOT NULL \
         GROUP BY publication_year ORDER BY publication_year DESC",
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

/// List journals with canonical filters.
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
    validate_item_count(
        "search filters",
        rating_filter_count(&params.ratings).saturating_add(usize::from(params.area.is_some())),
        MAX_SEARCH_FILTER_ITEMS,
    )
    .map_err(|error| IndexRepositoryError::InvalidInput(error.to_string()))?;
    let connection = open_index_connection(config, db_name)?;
    let mut clauses = Vec::new();
    let mut values = Vec::new();
    push_optional_text_filter(&mut clauses, &mut values, "j.area = ?", &params.area);
    push_rating_filters(&mut clauses, &mut values, &params.ratings)?;
    if let Some(has_articles) = params.has_articles {
        clauses.push(format!(
            "{}EXISTS (SELECT 1 FROM articles a WHERE a.journal_id = j.journal_id)",
            if has_articles { "" } else { "NOT " }
        ));
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
        params.sort.as_deref().unwrap_or("title:asc"),
        &[
            ("journal_id", "j.journal_id"),
            ("title", "j.title"),
            ("issn", "j.issn"),
            ("eissn", "j.eissn"),
            ("area", "j.area"),
        ],
    )?;
    let total = connection.query_row(
        &format!("SELECT COUNT(*) FROM journals j {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values;
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT j.journal_id, j.catalog_id, j.title, j.title_aliases_json, j.issns_json, \
         j.issn, j.eissn, j.area, j.utd_rank, j.utd_rating, j.abs_rank, j.abs_rating, \
         j.fms_rank, j.fms_rating, j.fmscn_rank, j.fmscn_rating, \
         EXISTS (SELECT 1 FROM articles a WHERE a.journal_id = j.journal_id) \
         FROM journals j {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), journal_from_row)?;
    Ok(JournalPage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one canonical journal.
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
            "SELECT j.journal_id, j.catalog_id, j.title, j.title_aliases_json, j.issns_json, \
             j.issn, j.eissn, j.area, j.utd_rank, j.utd_rating, j.abs_rank, j.abs_rating, \
             j.fms_rank, j.fms_rating, j.fmscn_rank, j.fmscn_rating, \
             EXISTS (SELECT 1 FROM articles a WHERE a.journal_id = j.journal_id) \
             FROM journals j WHERE j.journal_id = ?1",
            [journal_id],
            journal_from_row,
        )
        .optional()?
        .ok_or(IndexRepositoryError::NotFound("Journal not found"))
}

/// List issues with canonical filters.
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
    let total = connection.query_row(
        &format!("SELECT COUNT(*) FROM issues i {where_sql}"),
        params_from_iter(values.iter()),
        |row| row.get(0),
    )?;
    let mut page_values = values;
    page_values.push(SqlValue::Integer(params.limit));
    page_values.push(SqlValue::Integer(params.offset));
    let mut statement = connection.prepare(&format!(
        "SELECT i.issue_id, i.journal_id, i.publication_year, i.title, i.volume, i.number, i.date \
         FROM issues i {where_sql} {order_sql} LIMIT ? OFFSET ?"
    ))?;
    let rows = statement.query_map(params_from_iter(page_values.iter()), issue_from_row)?;
    Ok(IssuePage {
        items: collect_rows(rows)?,
        page: page_meta(Some(total), params.limit, params.offset, None, None),
    })
}

/// Get one canonical issue.
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
            "SELECT issue_id, journal_id, publication_year, title, volume, number, date \
             FROM issues WHERE issue_id = ?1",
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
        catalog_id: row.get(1)?,
        title: row.get(2)?,
        title_aliases: json_string_vec_from_row(row, 3)?,
        issns: json_string_vec_from_row(row, 4)?,
        issn: row.get(5)?,
        eissn: row.get(6)?,
        area: row.get(7)?,
        utd_rank: row.get(8)?,
        utd_rating: row.get(9)?,
        abs_rank: row.get(10)?,
        abs_rating: row.get(11)?,
        fms_rank: row.get(12)?,
        fms_rating: row.get(13)?,
        fmscn_rank: row.get(14)?,
        fmscn_rating: row.get(15)?,
        has_articles: row.get::<_, i64>(16)? != 0,
    })
}

fn issue_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<IssueRecord> {
    let date = row.get::<_, Option<String>>(6)?;
    Ok(IssueRecord {
        issue_id: row.get(0)?,
        journal_id: JournalId(row.get(1)?),
        publication_year: row.get(2)?,
        title: row.get(3)?,
        volume: row.get(4)?,
        number: row.get(5)?,
        date_precision: date.as_deref().and_then(litradar_domain::date_precision),
        date,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::test_support::{
        fixture_db_path, set_rating_fixture, value_counts, IndexFixture,
    };

    #[test]
    fn rating_metadata_counts_journals_and_filters_combine_with_existing_fields() {
        let fixture = IndexFixture::new(true);
        set_rating_fixture(&fixture);
        let options = list_journal_ratings(&fixture.config, Some(&fixture.db_name)).unwrap();
        assert_eq!(value_counts(options.utd_rating), [("UTD24".into(), 1)]);
        assert_eq!(
            value_counts(options.abs_rating),
            [("3".into(), 1), ("4".into(), 1), ("4*".into(), 1)]
        );
        assert_eq!(
            value_counts(options.fms_rating),
            [("A".into(), 1), ("B".into(), 1)]
        );
        assert_eq!(
            value_counts(options.fmscn_rating),
            [("T1".into(), 2), ("T2".into(), 1)]
        );
        let params = JournalListParams {
            ratings: JournalRatingFilters {
                abs_rating: vec!["4*".into(), "4".into()],
                ..Default::default()
            },
            area: Some("Engineering".into()),
            has_articles: Some(true),
            year: Some(2026),
            limit: 10,
            ..Default::default()
        };
        let page = list_journals(&fixture.config, Some(&fixture.db_name), &params).unwrap();
        assert_eq!(page.page.total, Some(1));
        assert_eq!(page.items[0].journal_id.value(), 2);
        let mut params = JournalListParams {
            area: None,
            has_articles: None,
            year: None,
            sort: Some("journal_id:desc".into()),
            limit: 1,
            offset: 1,
            ..params
        };
        let page = list_journals(&fixture.config, Some(&fixture.db_name), &params).unwrap();
        assert_eq!(page.page.total, Some(2));
        assert_eq!(page.items[0].journal_id.value(), 1);
        params.ratings.fms_rating = vec!["unknown".into()];
        assert_eq!(
            list_journals(&fixture.config, Some(&fixture.db_name), &params)
                .unwrap()
                .page
                .total,
            Some(0)
        );
        params.ratings.fms_rating = vec![" ".into()];
        assert!(matches!(
            list_journals(&fixture.config, Some(&fixture.db_name), &params),
            Err(IndexRepositoryError::InvalidInput(_))
        ));
        let connection = Connection::open(fixture_db_path(&fixture)).unwrap();
        connection
            .execute(
                "UPDATE journals SET utd_rating=NULL,abs_rating='',fms_rating=NULL,fmscn_rating=''",
                [],
            )
            .unwrap();
        assert_eq!(
            list_journal_ratings(&fixture.config, Some(&fixture.db_name)).unwrap(),
            JournalRatingOptions::default()
        );
    }

    #[test]
    fn journal_metadata_filters_use_only_canonical_content() {
        let fixture = IndexFixture::new(true);

        assert_eq!(
            list_index_database_names(&fixture.config).expect("databases should list"),
            ["fixture.sqlite"]
        );
        assert_eq!(
            value_counts(list_areas(&fixture.config, Some(&fixture.db_name)).expect("areas")),
            [("Engineering".to_string(), 1), ("Medicine".to_string(), 2)]
        );
        let years = list_years(&fixture.config, Some(&fixture.db_name)).expect("years");
        assert_eq!((years[0].year, years[0].issue_count), (2026, 2));

        let options = list_journal_options(&fixture.config, Some(&fixture.db_name))
            .expect("options should list");
        assert_eq!(
            options
                .iter()
                .map(|option| option.title.as_str())
                .collect::<Vec<_>>(),
            ["Alpha Journal", "Beta Journal", "Gamma Journal"]
        );

        let page = list_journals(
            &fixture.config,
            Some(&fixture.db_name),
            &JournalListParams {
                area: Some("Medicine".to_string()),
                ratings: JournalRatingFilters::default(),
                has_articles: Some(true),
                year: Some(2026),
                sort: Some("title:asc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("journal filters should apply");
        assert_eq!(page.page.total, Some(1));
        assert_eq!(page.items[0].catalog_id, "alpha-journal");
        assert_eq!(page.items[0].issns, ["1234-5679"]);
        assert!(page.items[0].has_articles);

        let issue_page = list_issues(
            &fixture.config,
            Some(&fixture.db_name),
            &IssueListParams {
                journal_id: Some(1),
                year: Some(2026),
                sort: Some("date:desc".to_string()),
                limit: 10,
                offset: 0,
            },
        )
        .expect("issue filters should apply");
        assert_eq!(issue_page.page.total, Some(1));
        assert_eq!(issue_page.items[0].issue_id, 10);
        assert_eq!(
            issue_page.items[0].date_precision,
            Some(litradar_domain::DatePrecision::Day)
        );
    }
}
