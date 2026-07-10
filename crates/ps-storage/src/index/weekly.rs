//! Weekly update manifest loading and article grouping.

use super::shared::*;
use super::*;

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
    let now = current_utc_iso_text();
    let manifests = load_weekly_manifests(config)?;
    if manifests.is_empty() {
        let window_start = iso_minus_days(&now, 7).unwrap_or_else(|| now.clone());
        return Ok(WeeklyUpdatesResponse {
            generated_at: now.clone(),
            window_start,
            window_end: now,
            databases: Vec::new(),
        });
    }
    let window_end = manifests
        .iter()
        .map(|manifest| manifest.generated_at.clone())
        .max()
        .unwrap_or_else(|| now.clone());
    let window_start = iso_minus_days(&window_end, 7).unwrap_or_else(|| window_end.clone());
    let mut by_db: HashMap<String, WeeklyBucket> = HashMap::new();
    for manifest in manifests {
        let bucket = by_db
            .entry(manifest.db_name.clone())
            .or_insert(WeeklyBucket {
                generated_at: manifest.generated_at.clone(),
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
            generated_at: bucket.generated_at,
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
        generated_at: now,
        window_start,
        window_end,
        databases,
    })
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
            "SELECT a.article_id, a.journal_id, a.issue_id, a.title, a.date, a.authors, \
             a.abstract, a.doi, a.platform_id, a.permalink, a.full_text_file, a.open_access, \
             a.in_press, j.title AS journal_title, i.volume, i.number \
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
                .and_then(|article| article.journal_title.clone());
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
) -> Result<Vec<WeeklyManifest>, IndexRepositoryError> {
    let push_state_dir = config.project_root().join("data").join("push_state");
    if !push_state_dir.exists() {
        return Ok(Vec::new());
    }
    let mut manifests = Vec::new();
    for entry in fs::read_dir(push_state_dir)? {
        let path = entry?.path();
        if !path
            .file_name()
            .and_then(|value| value.to_str())
            .is_some_and(|name| name.ends_with(".changes.json"))
        {
            continue;
        }
        let payload = read_weekly_manifest_payload(&path)?;
        if let Some(manifest) = parse_weekly_manifest(payload) {
            manifests.push(manifest);
        }
    }
    manifests.sort_by(|left, right| {
        right
            .generated_at
            .cmp(&left.generated_at)
            .then_with(|| right.db_name.cmp(&left.db_name))
    });
    Ok(manifests)
}

#[derive(Debug, Deserialize)]
struct WeeklyManifestPayload {
    db_name: Option<String>,
    db_path: Option<String>,
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
    let db_name = payload
        .db_name
        .as_deref()
        .or(payload.db_path.as_deref())
        .and_then(normalize_db_name)?;
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
        .and_then(normalize_iso_datetime)
        .unwrap_or_else(current_utc_iso_text);
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
    Ok(WeeklyArticleRecord {
        article_id: ArticleId(row.get(0)?),
        journal_id: JournalId(row.get(1)?),
        issue_id: row.get(2)?,
        title: row.get(3)?,
        date: row.get(4)?,
        authors: row.get(5)?,
        abstract_text: row.get(6)?,
        doi: row.get(7)?,
        platform_id: row.get(8)?,
        permalink: row.get(9)?,
        full_text_file: row.get(10)?,
        open_access: row.get(11)?,
        in_press: row.get(12)?,
        journal_title: row.get(13)?,
        volume: row.get(14)?,
        number: row.get(15)?,
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

fn current_utc_iso_text() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time should be after Unix epoch")
        .as_secs() as i64;
    format_unix_seconds(seconds)
}

fn normalize_iso_datetime(value: &str) -> Option<String> {
    parse_iso_utc_seconds(value).map(format_unix_seconds)
}

fn iso_minus_days(value: &str, days: i64) -> Option<String> {
    parse_iso_utc_seconds(value).map(|seconds| format_unix_seconds(seconds - days * 86_400))
}

fn parse_iso_utc_seconds(value: &str) -> Option<i64> {
    let text = value
        .trim()
        .strip_suffix('Z')
        .unwrap_or_else(|| value.trim())
        .strip_suffix("+00:00")
        .unwrap_or_else(|| {
            value
                .trim()
                .strip_suffix('Z')
                .unwrap_or_else(|| value.trim())
        });
    let (date, time) = text.split_once('T')?;
    let mut date_parts = date.split('-');
    let year = date_parts.next()?.parse::<i64>().ok()?;
    let month = date_parts.next()?.parse::<i64>().ok()?;
    let day = date_parts.next()?.parse::<i64>().ok()?;
    if date_parts.next().is_some() {
        return None;
    }
    let mut time_parts = time.split(':');
    let hour = time_parts.next()?.parse::<i64>().ok()?;
    let minute = time_parts.next()?.parse::<i64>().ok()?;
    let second_text = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let second = second_text
        .split_once('.')
        .map_or(second_text, |(seconds, _)| seconds)
        .parse::<i64>()
        .ok()?;
    if !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return None;
    }
    Some(days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second)
}

fn format_unix_seconds(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let day_seconds = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = day_seconds / 3_600;
    let minute = (day_seconds % 3_600) / 60;
    let second = day_seconds % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month_prime = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month_prime + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let days = days + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    let year = year + i64::from(month <= 2);
    (year, month, day)
}

#[derive(Debug, Clone)]
struct WeeklyManifest {
    db_name: String,
    run_id: Option<String>,
    generated_at: String,
    article_ids: Vec<i64>,
}

#[derive(Debug, Clone)]
struct WeeklyBucket {
    generated_at: String,
    run_id: Option<String>,
    article_ids: Vec<i64>,
    seen: HashSet<i64>,
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::{json, Value as JsonValue};

    use super::*;
    use crate::index::test_support::{weekly_article_ids, write_weekly_manifest, IndexFixture};

    #[test]
    fn weekly_updates_cover_manifest_merging_grouping_and_missing_databases() {
        let fixture = IndexFixture::new(true);

        write_weekly_manifest(
            &fixture.config,
            "older.changes.json",
            json!({
                "db_name": fixture.db_name,
                "generated_at": "2026-07-05T10:00:00Z",
                "run_id": "run-a",
                "notifiable_article_ids": [1001, 1003, 1001, 9999],
                "summary": {
                    "added_article_ids": [1001, 1003, 9999],
                    "issues": [{"added_article_ids": [1001, 1003, 9999]}]
                }
            }),
        );
        write_weekly_manifest(
            &fixture.config,
            "newer.changes.json",
            json!({
                "db_path": format!("data/index/{}", fixture.db_name),
                "generated_at": "2026-07-06T10:00:00Z",
                "run_id": "run-b",
                "notifiable_article_ids": [1002, 1001]
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

        let updates = get_weekly_updates(&fixture.config).expect("weekly updates should resolve");

        assert!(normalize_iso_datetime(&updates.generated_at).is_some());
        assert_eq!(updates.window_start, "2026-06-29T10:00:00Z");
        assert_eq!(updates.window_end, "2026-07-06T10:00:00Z");
        assert_eq!(updates.databases.len(), 1);

        let database = &updates.databases[0];
        assert_eq!(database.db_name, "fixture.sqlite");
        assert_eq!(database.run_id.as_deref(), Some("run-b"));
        assert_eq!(database.generated_at, "2026-07-06T10:00:00Z");
        assert_eq!(database.new_article_count, 3);
        assert_eq!(database.journals.len(), 2);

        assert_eq!(database.journals[0].journal_id.value(), 1);
        assert_eq!(
            database.journals[0].journal_title.as_deref(),
            Some("Alpha Journal")
        );
        assert_eq!(database.journals[0].new_article_count, 2);
        assert_eq!(
            weekly_article_ids(&database.journals[0].articles),
            vec![1002, 1001]
        );

        assert_eq!(database.journals[1].journal_id.value(), 2);
        assert_eq!(
            database.journals[1].journal_title.as_deref(),
            Some("Beta CNKI")
        );
        assert_eq!(database.journals[1].new_article_count, 1);
        assert_eq!(
            weekly_article_ids(&database.journals[1].articles),
            vec![1003]
        );
    }

    #[test]
    fn weekly_updates_without_manifests_return_empty_window_with_iso_bounds() {
        let fixture = IndexFixture::new(true);

        let updates = get_weekly_updates(&fixture.config).expect("weekly updates should resolve");

        assert!(updates.databases.is_empty());
        assert!(normalize_iso_datetime(&updates.generated_at).is_some());
        assert_eq!(updates.window_end, updates.generated_at);
        assert_eq!(
            updates.window_start,
            iso_minus_days(&updates.window_end, 7).expect("window end should be parseable")
        );
    }

    #[test]
    fn weekly_manifest_parsing_covers_normalization_empty_and_malformed_payloads() {
        let manifest = parse_weekly_manifest_payload(json!({
            "db_path": "data/index/fixture",
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
        assert_eq!(manifest.generated_at, "2026-07-05T10:00:00Z");
        assert_eq!(manifest.run_id.as_deref(), Some("run-1"));
        assert_eq!(manifest.article_ids, vec![1001, 1002]);

        assert!(parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "notifiable_article_ids": []
        }))
        .is_none());
        assert!(parse_weekly_manifest_payload(json!({
            "notifiable_article_ids": [1001]
        }))
        .is_none());
        assert!(parse_weekly_manifest_payload(json!({
            "db_name": "fixture.sqlite",
            "notifiable_article_ids": ["bad"]
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

        let error = load_weekly_manifests(&fixture.config).expect_err("invalid JSON should fail");

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
        assert_eq!(normalize_iso_datetime("2026-99-05T10:11:12Z"), None);
        assert_eq!(
            iso_minus_days("2026-07-06T10:00:00Z", 7),
            Some("2026-06-29T10:00:00Z".to_string())
        );
    }

    fn parse_weekly_manifest_payload(payload: JsonValue) -> Option<WeeklyManifest> {
        parse_weekly_manifest(
            serde_json::from_value(payload).expect("weekly manifest payload should deserialize"),
        )
    }
}
