//! Weekly update manifest loading and article grouping.

use std::path::PathBuf;
use std::time::SystemTime;

use chrono::{DateTime, SecondsFormat, TimeDelta, Utc};

use super::articles::fetch_articles_by_ids;
use super::shared::*;
use super::*;

/// Maximum unique manifest references admitted by the legacy aggregate endpoint.
pub const LEGACY_WEEKLY_ARTICLE_LIMIT: usize = 2_000;

/// Parameters for one fixed-window weekly article page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WeeklyArticlePageParams {
    /// Selected index database name.
    pub db_name: String,
    /// Selected journal identifier.
    pub journal_id: i64,
    /// RFC3339 end of the seven-day manifest window.
    pub window_end: String,
    /// Optional simple full-text query.
    pub q: Option<String>,
    /// Maximum records to return from 1 through 200.
    pub limit: i64,
    /// Optional descending `date|article_id` cursor.
    pub cursor: Option<String>,
}

/// Return weekly updates grouped by database and journal.
///
/// # Arguments
///
/// * `config` - Storage paths.
///
/// # Returns
///
/// Weekly updates response.
pub fn get_weekly_updates(
    config: &StorageConfig,
) -> Result<WeeklyUpdatesResponse, IndexRepositoryError> {
    get_weekly_updates_at(config, DateTime::<Utc>::from(SystemTime::now()))
}

fn get_weekly_updates_at(
    config: &StorageConfig,
    now: DateTime<Utc>,
) -> Result<WeeklyUpdatesResponse, IndexRepositoryError> {
    let window_start_at = weekly_window_start(now);
    let window_start = format_utc_datetime(window_start_at);
    let window_end = format_utc_datetime(now);
    let by_db = load_weekly_buckets(config, window_start_at, now)?;
    enforce_legacy_weekly_article_limit(&by_db)?;
    let mut databases = Vec::new();
    for (db_name, bucket) in by_db {
        let db_path = config.index_dir().join(&db_name);
        if !db_path.exists() || bucket.article_ids.is_empty() {
            continue;
        }
        let connection = open_sqlite_connection(db_path)?;
        let articles = fetch_weekly_articles(&connection, &bucket.article_ids)?;
        if articles.is_empty() {
            continue;
        }
        databases.push(WeeklyDatabaseUpdate {
            db_name,
            run_id: bucket.run_id,
            generated_at: format_utc_datetime(bucket.generated_at),
            new_article_count: articles.len(),
            journals: group_weekly_articles_by_journal(articles),
        });
    }
    databases.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.db_name.cmp(&left.db_name))
    });
    Ok(WeeklyUpdatesResponse {
        generated_at: window_end.clone(),
        window_start,
        window_end,
        databases,
    })
}

/// Return weekly database and journal counts without article bodies.
///
/// # Arguments
///
/// * `config` - Storage paths.
///
/// # Returns
///
/// Fixed seven-day summary for bounded browser navigation.
pub fn get_weekly_updates_summary(
    config: &StorageConfig,
) -> Result<WeeklyUpdatesSummaryResponse, IndexRepositoryError> {
    get_weekly_updates_summary_at(config, DateTime::<Utc>::from(SystemTime::now()))
}

fn get_weekly_updates_summary_at(
    config: &StorageConfig,
    now: DateTime<Utc>,
) -> Result<WeeklyUpdatesSummaryResponse, IndexRepositoryError> {
    let window_start_at = weekly_window_start(now);
    let window_start = format_precise_utc_datetime(window_start_at);
    let window_end = format_precise_utc_datetime(now);
    let by_db = load_weekly_buckets(config, window_start_at, now)?;
    let mut databases = Vec::new();
    for (db_name, bucket) in by_db {
        let db_path = config.index_dir().join(&db_name);
        if !db_path.exists() || bucket.article_ids.is_empty() {
            continue;
        }
        let mut connection = open_sqlite_connection(db_path)?;
        install_weekly_membership(&mut connection, &bucket.article_ids)?;
        let journals = fetch_weekly_journal_summaries(&connection)?;
        if journals.is_empty() {
            continue;
        }
        databases.push(WeeklyDatabaseSummary {
            db_name,
            run_id: bucket.run_id,
            generated_at: format_utc_datetime(bucket.generated_at),
            new_article_count: journals
                .iter()
                .map(|journal| journal.new_article_count)
                .sum(),
            journals,
        });
    }
    databases.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.db_name.cmp(&left.db_name))
    });
    Ok(WeeklyUpdatesSummaryResponse {
        generated_at: window_end.clone(),
        window_start,
        window_end,
        databases,
    })
}

/// Return one bounded weekly article page for a fixed manifest window.
///
/// # Arguments
///
/// * `config` - Storage paths.
/// * `params` - Database, journal, window, search, limit, and cursor parameters.
///
/// # Returns
///
/// Descending weekly article page.
pub fn get_weekly_update_articles(
    config: &StorageConfig,
    params: &WeeklyArticlePageParams,
) -> Result<WeeklyArticlePage, IndexRepositoryError> {
    validate_weekly_article_page_params(params)?;
    let window_end = parse_iso_datetime(&params.window_end).ok_or_else(|| {
        IndexRepositoryError::InvalidInput(
            "window_end must be a valid RFC3339 timestamp".to_string(),
        )
    })?;
    let window_start = weekly_window_start(window_end);
    let db_name = normalize_db_name(&params.db_name).ok_or_else(|| {
        IndexRepositoryError::InvalidInput("db must select a database".to_string())
    })?;
    let mut connection = open_index_connection(config, Some(&db_name))?;
    let journal_exists = connection.query_row(
        "SELECT EXISTS(SELECT 1 FROM journals WHERE journal_id = ?1)",
        [params.journal_id],
        |row| row.get::<_, bool>(0),
    )?;
    if !journal_exists {
        return Err(IndexRepositoryError::NotFound("Journal not found"));
    }
    let mut by_db = load_weekly_buckets(config, window_start, window_end)?;
    let article_ids = by_db
        .remove(&db_name)
        .map(|bucket| bucket.article_ids)
        .unwrap_or_default();
    install_weekly_membership(&mut connection, &article_ids)?;
    fetch_weekly_article_page(&connection, params)
}

fn weekly_window_start(window_end: DateTime<Utc>) -> DateTime<Utc> {
    let window_delta = TimeDelta::try_days(7).expect("seven-day duration should be valid");
    window_end
        .checked_sub_signed(window_delta)
        .unwrap_or(DateTime::<Utc>::MIN_UTC)
}

fn load_weekly_buckets(
    config: &StorageConfig,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<HashMap<String, WeeklyBucket>, IndexRepositoryError> {
    let manifests = load_weekly_manifests(config, window_start, window_end)?;
    let mut by_db: HashMap<String, WeeklyBucket> = HashMap::new();
    for manifest in manifests {
        let bucket = by_db
            .entry(manifest.db_name.clone())
            .or_insert(WeeklyBucket {
                generated_at: manifest.generated_at,
                run_id: manifest.run_id.clone(),
                article_ids: Vec::new(),
                seen: HashSet::new(),
            });
        for article_id in manifest.article_ids {
            if bucket.seen.insert(article_id) {
                bucket.article_ids.push(article_id);
            }
        }
    }
    Ok(by_db)
}

fn enforce_legacy_weekly_article_limit(
    by_db: &HashMap<String, WeeklyBucket>,
) -> Result<(), IndexRepositoryError> {
    let article_count = by_db.values().fold(0_usize, |count, bucket| {
        count.saturating_add(bucket.article_ids.len())
    });
    if article_count > LEGACY_WEEKLY_ARTICLE_LIMIT {
        Err(IndexRepositoryError::LegacyWeeklyArticleLimitExceeded)
    } else {
        Ok(())
    }
}

fn install_weekly_membership(
    connection: &mut Connection,
    article_ids: &[i64],
) -> Result<(), IndexRepositoryError> {
    connection.execute_batch(
        "CREATE TEMP TABLE weekly_membership (
             article_id INTEGER PRIMARY KEY
         ) WITHOUT ROWID;",
    )?;
    let transaction = connection.transaction()?;
    {
        let mut statement = transaction
            .prepare("INSERT OR IGNORE INTO temp.weekly_membership (article_id) VALUES (?1)")?;
        for &article_id in article_ids {
            statement.execute([article_id])?;
        }
    }
    transaction.commit()?;
    Ok(())
}

fn fetch_weekly_journal_summaries(
    connection: &Connection,
) -> Result<Vec<WeeklyJournalSummary>, IndexRepositoryError> {
    let mut statement = connection.prepare(
        "SELECT l.journal_id, j.title, COUNT(*)
         FROM temp.weekly_membership membership
         JOIN article_listing l ON l.article_id = membership.article_id
         JOIN journals j ON j.journal_id = l.journal_id
         GROUP BY l.journal_id, j.title",
    )?;
    let rows = statement.query_map([], |row| {
        Ok(WeeklyJournalSummary {
            journal_id: JournalId(row.get(0)?),
            journal_title: Some(row.get(1)?),
            new_article_count: row.get::<_, i64>(2)? as usize,
        })
    })?;
    let mut journals = collect_rows(rows)?;
    journals.sort_by(|left, right| {
        right
            .new_article_count
            .cmp(&left.new_article_count)
            .then_with(|| {
                left.journal_title
                    .clone()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .journal_title
                            .clone()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
            .then_with(|| left.journal_id.value().cmp(&right.journal_id.value()))
    });
    Ok(journals)
}

fn validate_weekly_article_page_params(
    params: &WeeklyArticlePageParams,
) -> Result<(), IndexRepositoryError> {
    let validation_result = (|| {
        validate_characters("db", &params.db_name, MAX_DATABASE_NAME_CHARS)?;
        validate_characters("window_end", &params.window_end, MAX_SEARCH_TEXT_CHARS)?;
        if let Some(q) = &params.q {
            validate_characters("q", q, MAX_SEARCH_TEXT_CHARS)?;
        }
        if let Some(cursor) = &params.cursor {
            validate_characters("cursor", cursor, MAX_SEARCH_TEXT_CHARS)?;
        }
        Ok::<(), litradar_domain::InputValidationError>(())
    })();
    validation_result.map_err(|error| IndexRepositoryError::InvalidInput(error.to_string()))?;
    if params.journal_id <= 0 {
        return Err(IndexRepositoryError::InvalidInput(
            "journal_id must be greater than 0".to_string(),
        ));
    }
    validate_limit_offset(params.limit, 0)
}

fn fetch_weekly_article_page(
    connection: &Connection,
    params: &WeeklyArticlePageParams,
) -> Result<WeeklyArticlePage, IndexRepositoryError> {
    let mut clauses = vec!["l.journal_id = ?".to_string()];
    let mut values = vec![SqlValue::Integer(params.journal_id)];
    push_fts_filter(
        &mut clauses,
        &mut values,
        "l.article_id",
        &params.q,
        ArticleSearchMode::Simple,
    );
    push_cursor_filter(
        &mut clauses,
        &mut values,
        "l",
        SortDirection::Desc,
        params.cursor.as_deref(),
    )?;
    values.push(SqlValue::Integer(params.limit + 1));
    let mut statement = connection.prepare(&format!(
        "SELECT l.article_id, l.date
         FROM temp.weekly_membership membership
         JOIN article_listing l ON l.article_id = membership.article_id
         {}
         ORDER BY l.date DESC, l.article_id DESC
         LIMIT ?",
        where_sql(&clauses)
    ))?;
    let rows = statement.query_map(params_from_iter(values.iter()), |row| {
        Ok((row.get::<_, i64>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    let mut id_rows = collect_rows(rows)?;
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
    let items = fetch_articles_by_ids(connection, &article_ids)?
        .into_iter()
        .map(weekly_article_from_record)
        .collect();
    Ok(WeeklyArticlePage {
        items,
        page: page_meta(None, params.limit, 0, next_cursor.clone(), Some(has_more)),
    })
}

fn weekly_article_from_record(article: ArticleRecord) -> WeeklyArticleRecord {
    WeeklyArticleRecord {
        article_id: article.article_id,
        journal_id: article.journal_id,
        issue_id: article.issue_id,
        title: article.title,
        publication_year: article.publication_year,
        date: article.date,
        date_precision: article.date_precision,
        authors: article.authors,
        abstract_text: article.abstract_text,
        doi: article.doi,
        journal_title: article.journal_title,
        open_access: article.open_access,
        in_press: article.in_press,
        volume: article.volume,
        number: article.number,
    }
}

fn fetch_weekly_articles(
    connection: &Connection,
    article_ids: &[i64],
) -> Result<Vec<WeeklyArticleRecord>, IndexRepositoryError> {
    if article_ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_id = HashMap::new();
    for chunk in article_ids.chunks(500) {
        let placeholders = placeholders(chunk.len());
        let values = chunk
            .iter()
            .copied()
            .map(SqlValue::Integer)
            .collect::<Vec<_>>();
        let mut statement = connection.prepare(&format!(
            "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.publication_year, \
             a.date, a.authors_json, a.abstract_text, a.doi, a.open_access, a.in_press, \
             j.title AS journal_title, i.volume, i.number \
             FROM articles a LEFT JOIN issues i ON i.issue_id = a.issue_id \
             JOIN journals j ON j.journal_id = a.journal_id \
             WHERE a.article_id IN ({placeholders})"
        ))?;
        let rows = statement.query_map(params_from_iter(values.iter()), weekly_article_from_row)?;
        by_id.extend(
            collect_rows(rows)?
                .into_iter()
                .map(|article: WeeklyArticleRecord| (article.article_id.value(), article)),
        );
    }
    Ok(article_ids
        .iter()
        .filter_map(|article_id| by_id.remove(article_id))
        .collect())
}

fn group_weekly_articles_by_journal(
    articles: Vec<WeeklyArticleRecord>,
) -> Vec<WeeklyJournalUpdate> {
    let mut by_journal: HashMap<i64, Vec<WeeklyArticleRecord>> = HashMap::new();
    for article in articles {
        by_journal
            .entry(article.journal_id.value())
            .or_default()
            .push(article);
    }
    let mut journals = by_journal
        .into_iter()
        .map(|(journal_id, articles)| {
            let journal_title = articles
                .first()
                .map(|article| article.journal_title.clone());
            WeeklyJournalUpdate {
                journal_id: JournalId(journal_id),
                journal_title,
                new_article_count: articles.len(),
                articles,
            }
        })
        .collect::<Vec<_>>();
    journals.sort_by(|left, right| {
        right
            .new_article_count
            .cmp(&left.new_article_count)
            .then_with(|| {
                left.journal_title
                    .clone()
                    .unwrap_or_default()
                    .to_ascii_lowercase()
                    .cmp(
                        &right
                            .journal_title
                            .clone()
                            .unwrap_or_default()
                            .to_ascii_lowercase(),
                    )
            })
            .then_with(|| left.journal_id.value().cmp(&right.journal_id.value()))
    });
    journals
}

fn load_weekly_manifests(
    config: &StorageConfig,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    let mut seen = HashSet::new();
    for path in weekly_manifest_paths(&push_state_dir)? {
        let payload = read_weekly_manifest_payload(&path)?;
        let Some(manifest) = parse_weekly_manifest(payload) else {
            continue;
        };
        if manifest.generated_at >= window_start
            && manifest.generated_at <= window_end
            && seen.insert((
                manifest.db_name.clone(),
                manifest.run_id.clone(),
                manifest.generated_at,
                manifest.article_ids.clone(),
            ))
        {
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| left.db_name.cmp(&right.db_name))
            .then_with(|| left.run_id.cmp(&right.run_id))
            .then_with(|| left.article_ids.cmp(&right.article_ids))
    });
    Ok(manifests)
}

fn weekly_manifest_paths(push_state_dir: &Path) -> Result<Vec<PathBuf>, IndexRepositoryError> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(push_state_dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() && is_change_manifest_path(&entry.path()) {
            paths.push(entry.path());
        }
    }
    let history_directory = push_state_dir.join("history");
    if history_directory.exists() {
        for catalog_entry in fs::read_dir(history_directory)? {
            let catalog_entry = catalog_entry?;
            if !catalog_entry.file_type()?.is_dir() {
                continue;
            }
            for entry in fs::read_dir(catalog_entry.path())? {
                let entry = entry?;
                if entry.file_type()?.is_file() && is_managed_history_manifest_path(&entry.path()) {
                    paths.push(entry.path());
                }
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn is_change_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .is_some_and(|name| name.ends_with(".changes.json"))
}

fn is_managed_history_manifest_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|value| value.to_str())
        .and_then(|value| value.strip_suffix(".changes.json"))
        .is_some_and(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
}

#[derive(Debug, Deserialize)]
struct WeeklyManifestPayload {
    db_name: Option<String>,
    generated_at: Option<String>,
    run_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_json_i64_list")]
    notifiable_article_ids: Vec<i64>,
}

fn read_weekly_manifest_payload(
    path: &Path,
) -> Result<WeeklyManifestPayload, IndexRepositoryError> {
    let reader = std::io::BufReader::new(fs::File::open(path)?);
    Ok(serde_json::from_reader(reader)?)
}

fn parse_weekly_manifest(payload: WeeklyManifestPayload) -> Option<WeeklyManifest> {
    let db_name = payload.db_name.as_deref().and_then(normalize_db_name)?;
    let mut seen = HashSet::new();
    let mut article_ids = Vec::new();
    for item in payload.notifiable_article_ids {
        if seen.insert(item) {
            article_ids.push(item);
        }
    }
    if article_ids.is_empty() {
        return None;
    }
    let generated_at = payload
        .generated_at
        .as_deref()
        .or(payload.run_id.as_deref())
        .and_then(parse_manifest_datetime)?;
    Some(WeeklyManifest {
        db_name,
        run_id: payload.run_id,
        generated_at,
        article_ids,
    })
}

fn deserialize_json_i64_list<'de, D>(deserializer: D) -> Result<Vec<i64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = JsonValue::deserialize(deserializer)?;
    let Some(items) = value.as_array() else {
        return Ok(Vec::new());
    };
    Ok(items.iter().filter_map(JsonValue::as_i64).collect())
}

fn weekly_article_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<WeeklyArticleRecord> {
    let date = row.get::<_, Option<String>>(5)?;
    Ok(WeeklyArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        publication_year: row.get(4)?,
        date_precision: date.as_deref().and_then(litradar_domain::date_precision),
        date,
        authors: json_string_vec_from_row(row, 6)?,
        abstract_text: row.get(7)?,
        doi: row.get(8)?,
        open_access: row.get::<_, Option<i64>>(9)?.map(|value| value != 0),
        in_press: row.get::<_, Option<i64>>(10)?.map(|value| value != 0),
        journal_title: row.get(11)?,
        volume: row.get(12)?,
        number: row.get(13)?,
    })
}

fn normalize_db_name(value: &str) -> Option<String> {
    let filename = Path::new(value.trim()).file_name()?.to_str()?;
    if filename.is_empty() {
        None
    } else if filename.ends_with(".sqlite") {
        Some(filename.to_string())
    } else {
        Some(format!("{filename}.sqlite"))
    }
}

#[cfg(test)]
fn normalize_iso_datetime(value: &str) -> Option<String> {
    parse_iso_datetime(value).map(format_utc_datetime)
}

#[cfg(test)]
fn iso_minus_days(value: &str, days: i64) -> Option<String> {
    let days = TimeDelta::try_days(days)?;
    parse_iso_datetime(value)
        .and_then(|date| date.checked_sub_signed(days))
        .map(format_utc_datetime)
}

fn parse_iso_datetime(value: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value.trim())
        .ok()
        .map(|date| date.with_timezone(&Utc))
}

fn parse_manifest_datetime(value: &str) -> Option<DateTime<Utc>> {
    parse_iso_datetime(value).or_else(|| {
        let value = value.trim();
        let timestamp = value.parse::<i64>().ok()?;
        if timestamp.to_string() != value {
            return None;
        }
        DateTime::<Utc>::from_timestamp(timestamp, 0)
    })
}

fn format_utc_datetime(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::Secs, true)
}

fn format_precise_utc_datetime(value: DateTime<Utc>) -> String {
    value.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

#[derive(Debug, Clone)]
struct WeeklyManifest {
    db_name: String,
    run_id: Option<String>,
    generated_at: DateTime<Utc>,
    article_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct WeeklyBucket {
    generated_at: DateTime<Utc>,
    run_id: Option<String>,
    article_ids: Vec<i64>,
    seen: HashSet<i64>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{json, Value as JsonValue};

    use super::*;
    use crate::index::test_support::{
        fixture_db_path, weekly_article_ids, write_weekly_manifest, IndexFixture,
    };

    #[test]
    fn weekly_updates_cover_manifest_merging_grouping_and_missing_databases() {
        let fixture = IndexFixture::new(true);
        let now = parse_iso_datetime("2026-07-07T10:00:00Z").expect("now should parse");
        let older_payload = json!({
            "db_name": fixture.db_name,
            "generated_at": "2026-07-05T10:00:00Z",
            "run_id": "run-a",
            "notifiable_article_ids": [1001, 1003, 1001, 9999],
            "summary": {
                "added_article_ids": [1001, 1003, 9999],
                "issues": [{"added_article_ids": [1001, 1003, 9999]}]
            }
        });
        write_weekly_history_manifest(&fixture.config, &"11".repeat(32), &older_payload);
        let newer_payload = json!({
            "db_name": fixture.db_name,
            "generated_at": "2026-07-06T10:00:00Z",
            "run_id": "run-b",
            "notifiable_article_ids": [1002, 1001]
        });
        write_weekly_manifest(
            &fixture.config,
            "fixture.changes.json",
            newer_payload.clone(),
        );
        write_weekly_history_manifest(&fixture.config, &"22".repeat(32), &newer_payload);
        write_weekly_history_manifest(
            &fixture.config,
            &"33".repeat(32),
            &json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-06-30T10:00:00Z",
                "run_id": "boundary-run",
                "notifiable_article_ids": [1004]
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "missing.changes.json",
            json!({
                "db_name": "missing.sqlite",
                "generated_at": "2026-07-04T10:00:00Z",
                "run_id": "run-missing",
                "notifiable_article_ids": [1001]
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "empty.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-07T10:00:00Z",
                "notifiable_article_ids": []
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "old.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-06-30T09:59:59Z",
                "run_id": "old-run",
                "notifiable_article_ids": [1004]
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "future.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-07T10:00:01Z",
                "run_id": "future-run",
                "notifiable_article_ids": [1004]
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "untimestamped.changes.json",
            json!({
                "db_name": fixture.db_name,
                "run_id": "untimestamped-run",
                "notifiable_article_ids": [1004]
            }),
        );

        let updates =
            get_weekly_updates_at(&fixture.config, now).expect("weekly updates should resolve");

        assert_eq!(updates.generated_at, "2026-07-07T10:00:00Z");
        assert_eq!(updates.window_start, "2026-06-30T10:00:00Z");
        assert_eq!(updates.window_end, "2026-07-07T10:00:00Z");
        assert_eq!(updates.databases.len(), 1);

        let database = &updates.databases[0];
        assert_eq!(database.db_name, "fixture.sqlite");
        assert_eq!(database.run_id.as_deref(), Some("run-b"));
        assert_eq!(database.generated_at, "2026-07-06T10:00:00Z");
        assert_eq!(database.new_article_count, 4);
        assert_eq!(database.journals.len(), 2);

        assert_eq!(database.journals[0].journal_id.value(), 1);
        assert_eq!(
            database.journals[0].journal_title.as_deref(),
            Some("Alpha Journal")
        );
        assert_eq!(database.journals[0].new_article_count, 3);
        assert_eq!(
            weekly_article_ids(&database.journals[0].articles),
            vec![1002, 1001, 1004]
        );
        assert!(database.journals[0]
            .articles
            .iter()
            .all(|article| article.date_precision == Some(litradar_domain::DatePrecision::Day)));

        assert_eq!(database.journals[1].journal_id.value(), 2);
        assert_eq!(
            database.journals[1].journal_title.as_deref(),
            Some("Beta Journal")
        );
        assert_eq!(database.journals[1].new_article_count, 1);
        assert_eq!(
            weekly_article_ids(&database.journals[1].articles),
            vec![1003]
        );
    }

    #[test]
    fn legacy_weekly_updates_accept_exact_limit_and_reject_next_reference() {
        let fixture = IndexFixture::new(true);
        let now = parse_iso_datetime("2026-07-07T10:00:00Z").expect("now should parse");
        let manifest_path = "legacy-limit.changes.json";
        write_weekly_manifest(
            &fixture.config,
            manifest_path,
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-06T10:00:00Z",
                "run_id": "legacy-limit",
                "notifiable_article_ids": (1..=LEGACY_WEEKLY_ARTICLE_LIMIT).collect::<Vec<_>>()
            }),
        );

        get_weekly_updates_at(&fixture.config, now)
            .expect("the exact legacy article limit should succeed");

        write_weekly_manifest(
            &fixture.config,
            manifest_path,
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-06T10:00:00Z",
                "run_id": "legacy-limit",
                "notifiable_article_ids": (1..=LEGACY_WEEKLY_ARTICLE_LIMIT + 1).collect::<Vec<_>>()
            }),
        );
        assert!(matches!(
            get_weekly_updates_at(&fixture.config, now),
            Err(IndexRepositoryError::LegacyWeeklyArticleLimitExceeded)
        ));
    }

    #[test]
    fn weekly_summary_returns_only_existing_database_and_journal_counts() {
        let fixture = IndexFixture::new(true);
        let now = parse_iso_datetime("2026-07-07T10:00:00Z").expect("now should parse");
        write_weekly_manifest(
            &fixture.config,
            "summary.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-06T10:00:00Z",
                "run_id": "summary-run",
                "notifiable_article_ids": [1001, 1002, 1003, 1004, 9999]
            }),
        );

        let summary = get_weekly_updates_summary_at(&fixture.config, now)
            .expect("weekly summary should load");

        assert_eq!(summary.generated_at, "2026-07-07T10:00:00Z");
        assert_eq!(summary.window_start, "2026-06-30T10:00:00Z");
        assert_eq!(summary.window_end, summary.generated_at);
        assert_eq!(summary.databases.len(), 1);
        let database = &summary.databases[0];
        assert_eq!(database.db_name, "fixture.sqlite");
        assert_eq!(database.run_id.as_deref(), Some("summary-run"));
        assert_eq!(database.new_article_count, 4);
        assert_eq!(database.journals.len(), 2);
        assert_eq!(database.journals[0].journal_id, JournalId(1));
        assert_eq!(database.journals[0].new_article_count, 3);
        assert_eq!(database.journals[1].journal_id, JournalId(2));
        assert_eq!(database.journals[1].new_article_count, 1);
        let payload = serde_json::to_string(&summary).expect("summary should serialize");
        assert!(!payload.contains("\"articles\""));
        assert!(!payload.contains("\"abstract\""));
        let connection = Connection::open(fixture_db_path(&fixture))
            .expect("index database should remain readable");
        let persistent_membership_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'weekly_membership'",
                [],
                |row| row.get(0),
            )
            .expect("persistent schema should be inspectable");
        assert_eq!(persistent_membership_count, 0);
    }

    #[test]
    fn weekly_summary_preserves_fractional_window_boundaries_for_article_pages() {
        let fixture = IndexFixture::new(true);
        let now = parse_iso_datetime("2026-07-07T10:00:00.900Z").expect("now should parse");
        write_weekly_manifest(
            &fixture.config,
            "fractional-window.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-07T10:00:00.500Z",
                "run_id": "fractional-window",
                "notifiable_article_ids": [1001]
            }),
        );

        let summary = get_weekly_updates_summary_at(&fixture.config, now)
            .expect("fractional weekly summary should load");
        assert_eq!(summary.window_end, "2026-07-07T10:00:00.900Z");
        assert_eq!(summary.databases[0].new_article_count, 1);

        let page = get_weekly_update_articles(
            &fixture.config,
            &WeeklyArticlePageParams {
                db_name: fixture.db_name,
                journal_id: 1,
                window_end: summary.window_end,
                q: None,
                limit: 50,
                cursor: None,
            },
        )
        .expect("summary window should reproduce its article membership");
        assert_eq!(weekly_article_ids(&page.items), [1001]);
    }

    #[test]
    fn weekly_article_pages_apply_fixed_membership_search_and_descending_cursors() {
        let fixture = IndexFixture::new(true);
        write_weekly_manifest(
            &fixture.config,
            "current.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-06T10:00:00Z",
                "run_id": "current-run",
                "notifiable_article_ids": [1001, 1002, 1003, 1004]
            }),
        );
        write_weekly_history_manifest(
            &fixture.config,
            &"44".repeat(32),
            &json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-06-30T10:00:00Z",
                "run_id": "boundary-run",
                "notifiable_article_ids": [1008]
            }),
        );
        write_weekly_history_manifest(
            &fixture.config,
            &"55".repeat(32),
            &json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-06-30T09:59:59Z",
                "run_id": "expired-run",
                "notifiable_article_ids": [1005]
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "future.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-07T10:00:01Z",
                "run_id": "future-run",
                "notifiable_article_ids": [1005]
            }),
        );
        let mut params = WeeklyArticlePageParams {
            db_name: fixture.db_name.clone(),
            journal_id: 1,
            window_end: "2026-07-07T10:00:00Z".to_string(),
            q: None,
            limit: 2,
            cursor: None,
        };

        let first = get_weekly_update_articles(&fixture.config, &params)
            .expect("first weekly page should load");
        assert_eq!(weekly_article_ids(&first.items), [1004, 1001]);
        assert_eq!(first.page.limit, 2);
        assert_eq!(first.page.offset, 0);
        assert_eq!(first.page.total, None);
        assert_eq!(first.page.has_more, Some(true));
        assert_eq!(first.page.next_cursor.as_deref(), Some("2026-01-05|1001"));

        params.cursor = first.page.next_cursor;
        let second = get_weekly_update_articles(&fixture.config, &params)
            .expect("second weekly page should load");
        assert_eq!(weekly_article_ids(&second.items), [1002, 1008]);
        assert_eq!(second.page.has_more, Some(false));
        assert_eq!(second.page.next_cursor, None);

        params.cursor = None;
        params.limit = 200;
        params.q = Some("indexedonly".to_string());
        let searched = get_weekly_update_articles(&fixture.config, &params)
            .expect("weekly simple search should load");
        assert_eq!(weekly_article_ids(&searched.items), [1002]);
        params.q = Some("DOI fallback".to_string());
        let excluded = get_weekly_update_articles(&fixture.config, &params)
            .expect("non-member search should remain bounded");
        assert!(excluded.items.is_empty());

        params.q = None;
        params.cursor = Some("invalid-cursor".to_string());
        assert!(matches!(
            get_weekly_update_articles(&fixture.config, &params),
            Err(IndexRepositoryError::InvalidCursor)
        ));
        params.cursor = None;
        params.limit = 201;
        assert!(matches!(
            get_weekly_update_articles(&fixture.config, &params),
            Err(IndexRepositoryError::InvalidPagination(_))
        ));
        params.limit = 50;
        params.journal_id = 999;
        assert!(matches!(
            get_weekly_update_articles(&fixture.config, &params),
            Err(IndexRepositoryError::NotFound("Journal not found"))
        ));
        params.journal_id = 1;
        params.db_name = "missing.sqlite".to_string();
        assert!(matches!(
            get_weekly_update_articles(&fixture.config, &params),
            Err(IndexRepositoryError::DatabaseResolution(
                DatabaseResolutionError::DatabaseNotFound
            ))
        ));

        let connection = Connection::open(fixture_db_path(&fixture))
            .expect("index database should remain readable");
        let article_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM articles", [], |row| row.get(0))
            .expect("article count should remain readable");
        let persistent_membership_count: i64 = connection
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'weekly_membership'",
                [],
                |row| row.get(0),
            )
            .expect("persistent schema should be inspectable");
        assert_eq!(article_count, 6);
        assert_eq!(persistent_membership_count, 0);
    }

    #[test]
    fn weekly_updates_without_manifests_return_empty_window_with_iso_bounds() {
        let fixture = IndexFixture::new(true);
        let now = parse_iso_datetime("2026-07-07T10:00:00Z").expect("now should parse");

        let updates =
            get_weekly_updates_at(&fixture.config, now).expect("weekly updates should resolve");

        assert!(updates.databases.is_empty());
        assert_eq!(updates.generated_at, "2026-07-07T10:00:00Z");
        assert_eq!(updates.window_start, "2026-06-30T10:00:00Z");
        assert_eq!(updates.window_end, updates.generated_at);
    }

    #[test]
    fn weekly_manifest_parsing_covers_normalization_empty_and_malformed_payloads() {
        let manifest = parse_weekly_manifest_payload(json!({
            "db_name": "fixture",
            "generated_at": "2026-07-05T10:00:00.250+00:00",
            "run_id": "run-1",
            "notifiable_article_ids": [1001, 1001, "bad", 1002],
            "summary": {
                "added_article_ids": [1001, 1002],
                "issues": [{"added_article_ids": [1001, 1002]}]
            }
        }))
        .expect("valid manifest should parse");

        assert_eq!(manifest.db_name, "fixture.sqlite");
        assert_eq!(
            format_utc_datetime(manifest.generated_at),
            "2026-07-05T10:00:00Z"
        );
        assert_eq!(manifest.run_id.as_deref(), Some("run-1"));
        assert_eq!(manifest.article_ids, vec![1001, 1002]);

        let expected =
            parse_iso_datetime("2026-07-05T10:00:00Z").expect("expected timestamp should parse");
        let epoch_manifest = parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "generated_at": expected.timestamp().to_string(),
            "run_id": "run-epoch",
            "notifiable_article_ids": [1001]
        }))
        .expect("canonical epoch timestamp should parse");
        assert_eq!(epoch_manifest.generated_at, expected);

        let run_timestamp_manifest = parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "run_id": "2026-07-05T10:00:00Z",
            "notifiable_article_ids": [1001]
        }))
        .expect("timestamp-shaped legacy run id should parse");
        assert_eq!(run_timestamp_manifest.generated_at, expected);

        assert!(parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "notifiable_article_ids": []
        }))
        .is_none());
        assert!(parse_weekly_manifest_payload(json!({
            "db_path": "data/index/fixture.sqlite",
            "notifiable_article_ids": [1001]
        }))
        .is_none());
        assert!(parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "notifiable_article_ids": ["bad"]
        }))
        .is_none());
        assert!(parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "run_id": "untimestamped-run",
            "notifiable_article_ids": [1001]
        }))
        .is_none());
    }

    #[test]
    fn weekly_manifest_loading_fails_loud_on_invalid_json_files() {
        let fixture = IndexFixture::new(true);
        let push_state_dir = fixture
            .config
            .project_root()
            .join("data")
            .join("push_state");
        fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
        fs::write(push_state_dir.join("broken.changes.json"), "{")
            .expect("broken manifest should be written");

        let window_start =
            parse_iso_datetime("2026-06-30T10:00:00Z").expect("window start should parse");
        let window_end =
            parse_iso_datetime("2026-07-07T10:00:00Z").expect("window end should parse");
        let error = load_weekly_manifests(&fixture.config, window_start, window_end)
            .expect_err("invalid JSON should fail");

        assert!(matches!(error, IndexRepositoryError::Json(_)));
    }
    #[test]
    fn weekly_helpers_cover_dates_and_database_names() {
        assert_eq!(
            normalize_db_name("data/index/fixture"),
            Some("fixture.sqlite".to_string())
        );
        assert_eq!(
            normalize_db_name("fixture.sqlite"),
            Some("fixture.sqlite".to_string())
        );
        assert_eq!(normalize_db_name("   "), None);

        assert_eq!(
            normalize_iso_datetime("2026-07-05T10:11:12.900Z"),
            Some("2026-07-05T10:11:12Z".to_string())
        );
        assert_eq!(
            normalize_iso_datetime("2026-07-05T10:11:12+00:00"),
            Some("2026-07-05T10:11:12Z".to_string())
        );
        assert_eq!(
            normalize_iso_datetime("2026-07-05T18:11:12+08:00"),
            Some("2026-07-05T10:11:12Z".to_string())
        );
        assert_eq!(normalize_iso_datetime("2026-99-05T10:11:12Z"), None);
        assert_eq!(normalize_iso_datetime("2026-02-29T10:11:12Z"), None);
        assert_eq!(
            normalize_iso_datetime("2024-02-29T10:11:12Z"),
            Some("2024-02-29T10:11:12Z".to_string())
        );
        assert_eq!(
            iso_minus_days("2026-07-06T10:00:00Z", 7),
            Some("2026-06-29T10:00:00Z".to_string())
        );
        assert_eq!(
            iso_minus_days("2024-03-01T10:00:00Z", 1),
            Some("2024-02-29T10:00:00Z".to_string())
        );
    }

    fn parse_weekly_manifest_payload(payload: JsonValue) -> Option<WeeklyManifest> {
        parse_weekly_manifest(
            serde_json::from_value(payload).expect("weekly manifest payload should deserialize"),
        )
    }

    fn write_weekly_history_manifest(config: &StorageConfig, digest: &str, payload: &JsonValue) {
        let history_directory = config
            .project_root()
            .join("data")
            .join("push_state")
            .join("history")
            .join("fixture");
        fs::create_dir_all(&history_directory).expect("history directory should create");
        fs::write(
            history_directory.join(format!("{digest}.changes.json")),
            serde_json::to_vec(payload).expect("history manifest should serialize"),
        )
        .expect("history manifest should write");
    }
}
