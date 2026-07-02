//! SQLite schema and writer helpers for Rust scholarly indexing.

use rusqlite::{params, params_from_iter, Connection};

use crate::stats::IndexRunStats;
use crate::transforms::{ArticleRecord, IssueRecord, JournalRecord, MetaRecord};

/// Initialize the index database schema used by Python-compatible readers.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
///
/// # Returns
///
/// SQLite result.
pub fn init_index_db(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute_batch(
        "
        PRAGMA foreign_keys = ON;
        PRAGMA journal_mode = WAL;
        PRAGMA synchronous = NORMAL;

        CREATE TABLE IF NOT EXISTS journals (
            journal_id INTEGER PRIMARY KEY,
            library_id TEXT NOT NULL,
            platform_journal_id TEXT,
            title TEXT,
            issn TEXT,
            eissn TEXT,
            scimago_rank REAL,
            cover_url TEXT,
            available INTEGER,
            toc_data_approved_and_live INTEGER,
            has_articles INTEGER
        );

        CREATE TABLE IF NOT EXISTS journal_meta (
            journal_id INTEGER PRIMARY KEY,
            source_csv TEXT NOT NULL,
            area TEXT,
            csv_title TEXT,
            csv_issn TEXT,
            csv_library TEXT,
            resolved_source TEXT,
            resolved_source_id TEXT,
            resolved_title TEXT,
            resolved_issn TEXT,
            resolved_eissn TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS issues (
            issue_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            publication_year INTEGER,
            title TEXT,
            volume TEXT,
            number TEXT,
            date TEXT,
            is_valid_issue INTEGER,
            suppressed INTEGER,
            embargoed INTEGER,
            within_subscription INTEGER,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS articles (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            title TEXT,
            date TEXT,
            authors TEXT,
            start_page TEXT,
            end_page TEXT,
            abstract TEXT,
            doi TEXT,
            pmid TEXT,
            permalink TEXT,
            suppressed INTEGER,
            in_press INTEGER,
            open_access INTEGER,
            platform_id TEXT,
            retraction_doi TEXT,
            within_library_holdings INTEGER,
            content_location TEXT,
            full_text_file TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS article_listing (
            article_id INTEGER PRIMARY KEY,
            journal_id INTEGER NOT NULL,
            issue_id INTEGER,
            publication_year INTEGER,
            date TEXT,
            open_access INTEGER,
            in_press INTEGER,
            suppressed INTEGER,
            within_library_holdings INTEGER,
            doi TEXT,
            pmid TEXT,
            area TEXT,
            FOREIGN KEY (journal_id) REFERENCES journals(journal_id)
                ON DELETE CASCADE,
            FOREIGN KEY (issue_id) REFERENCES issues(issue_id)
                ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS journal_year_state (
            journal_id INTEGER NOT NULL,
            year INTEGER NOT NULL,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL,
            PRIMARY KEY (journal_id, year)
        );

        CREATE TABLE IF NOT EXISTS journal_state (
            journal_id INTEGER PRIMARY KEY,
            status TEXT NOT NULL,
            updated_at TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS index_runs (
            run_id TEXT PRIMARY KEY,
            csv_file TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            status TEXT NOT NULL,
            total_journals INTEGER NOT NULL,
            succeeded_journals INTEGER NOT NULL,
            failed_journals INTEGER NOT NULL,
            resumed_journals INTEGER NOT NULL,
            error_summary TEXT
        );

        CREATE TABLE IF NOT EXISTS index_path_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            status TEXT NOT NULL,
            started_at TEXT NOT NULL,
            finished_at TEXT,
            works_count INTEGER NOT NULL,
            issues_count INTEGER NOT NULL,
            article_summaries_count INTEGER NOT NULL,
            article_details_count INTEGER NOT NULL,
            articles_written_count INTEGER NOT NULL,
            articles_deleted_no_authors_count INTEGER NOT NULL,
            error_type TEXT,
            error_message TEXT,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS index_api_call_stats (
            run_id TEXT NOT NULL,
            source TEXT NOT NULL,
            service TEXT NOT NULL,
            endpoint TEXT NOT NULL,
            method TEXT NOT NULL,
            url_path TEXT NOT NULL,
            journal_id INTEGER,
            journal_title TEXT,
            logical_calls INTEGER NOT NULL,
            attempts INTEGER NOT NULL,
            successes INTEGER NOT NULL,
            failures INTEGER NOT NULL,
            retry_count INTEGER NOT NULL,
            status_codes_json TEXT NOT NULL,
            transport_errors INTEGER NOT NULL,
            rate_limit_failures INTEGER NOT NULL,
            total_latency_ms INTEGER NOT NULL,
            error_samples_json TEXT NOT NULL,
            FOREIGN KEY (run_id) REFERENCES index_runs(run_id)
                ON DELETE CASCADE
        );

        CREATE VIRTUAL TABLE IF NOT EXISTS article_search
        USING fts5(
            article_id UNINDEXED,
            title,
            abstract,
            doi,
            authors,
            journal_title
        );

        CREATE INDEX IF NOT EXISTS idx_articles_journal ON articles(journal_id);
        CREATE INDEX IF NOT EXISTS idx_articles_issue ON articles(issue_id);
        CREATE INDEX IF NOT EXISTS idx_articles_doi ON articles(doi);
        CREATE INDEX IF NOT EXISTS idx_article_listing_date_id
            ON article_listing(date, article_id);
        CREATE INDEX IF NOT EXISTS idx_index_api_call_stats_run
            ON index_api_call_stats(run_id);
        ",
    )
}

/// Insert or update a journal record.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `record` - Journal record.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_journal(connection: &Connection, record: &JournalRecord) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journals (
            journal_id, library_id, platform_journal_id, title, issn, eissn,
            scimago_rank, cover_url, available, toc_data_approved_and_live,
            has_articles
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(journal_id) DO UPDATE SET
            library_id = excluded.library_id,
            platform_journal_id = excluded.platform_journal_id,
            title = excluded.title,
            issn = excluded.issn,
            eissn = excluded.eissn,
            scimago_rank = excluded.scimago_rank,
            cover_url = excluded.cover_url,
            available = excluded.available,
            toc_data_approved_and_live = excluded.toc_data_approved_and_live,
            has_articles = excluded.has_articles
        ",
        params![
            record.journal_id,
            record.library_id,
            record.platform_journal_id,
            record.title,
            record.issn,
            record.eissn,
            record.scimago_rank,
            record.cover_url,
            record.available,
            record.toc_data_approved_and_live,
            record.has_articles,
        ],
    )?;
    Ok(())
}

/// Insert or update journal metadata.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `record` - Metadata record.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_meta(connection: &Connection, record: &MetaRecord) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_meta (
            journal_id, source_csv, area, csv_title, csv_issn, csv_library,
            resolved_source, resolved_source_id, resolved_title, resolved_issn,
            resolved_eissn
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(journal_id) DO UPDATE SET
            source_csv = excluded.source_csv,
            area = excluded.area,
            csv_title = excluded.csv_title,
            csv_issn = excluded.csv_issn,
            csv_library = excluded.csv_library,
            resolved_source = excluded.resolved_source,
            resolved_source_id = excluded.resolved_source_id,
            resolved_title = excluded.resolved_title,
            resolved_issn = excluded.resolved_issn,
            resolved_eissn = excluded.resolved_eissn
        ",
        params![
            record.journal_id,
            record.source_csv,
            record.area,
            record.csv_title,
            record.csv_issn,
            record.csv_library,
            record.resolved_source,
            record.resolved_source_id,
            record.resolved_title,
            record.resolved_issn,
            record.resolved_eissn,
        ],
    )?;
    Ok(())
}

/// Insert or update issue records.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Issue records.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_issues(connection: &Connection, records: &[IssueRecord]) -> rusqlite::Result<()> {
    for record in records {
        connection.execute(
            "
            INSERT INTO issues (
                issue_id, journal_id, publication_year, title, volume, number,
                date, is_valid_issue, suppressed, embargoed, within_subscription
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
            ON CONFLICT(issue_id) DO UPDATE SET
                journal_id = excluded.journal_id,
                publication_year = excluded.publication_year,
                title = excluded.title,
                volume = excluded.volume,
                number = excluded.number,
                date = excluded.date,
                is_valid_issue = excluded.is_valid_issue,
                suppressed = excluded.suppressed,
                embargoed = excluded.embargoed,
                within_subscription = excluded.within_subscription
            ",
            params![
                record.issue_id,
                record.journal_id,
                record.publication_year,
                record.title,
                record.volume,
                record.number,
                record.date,
                record.is_valid_issue,
                record.suppressed,
                record.embargoed,
                record.within_subscription,
            ],
        )?;
    }
    Ok(())
}

/// Insert or update article records.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Article records.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_articles(connection: &Connection, records: &[ArticleRecord]) -> rusqlite::Result<()> {
    for record in records {
        connection.execute(
            "
            INSERT INTO articles (
                article_id, journal_id, issue_id, title, date, authors, start_page,
                end_page, abstract, doi, pmid, permalink, suppressed, in_press,
                open_access, platform_id, retraction_doi, within_library_holdings,
                content_location, full_text_file
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14,
                ?15, ?16, ?17, ?18, ?19, ?20
            )
            ON CONFLICT(article_id) DO UPDATE SET
                journal_id = excluded.journal_id,
                issue_id = excluded.issue_id,
                title = excluded.title,
                date = excluded.date,
                authors = excluded.authors,
                start_page = excluded.start_page,
                end_page = excluded.end_page,
                abstract = excluded.abstract,
                doi = excluded.doi,
                pmid = excluded.pmid,
                permalink = excluded.permalink,
                suppressed = excluded.suppressed,
                in_press = excluded.in_press,
                open_access = excluded.open_access,
                platform_id = excluded.platform_id,
                retraction_doi = excluded.retraction_doi,
                within_library_holdings = excluded.within_library_holdings,
                content_location = excluded.content_location,
                full_text_file = excluded.full_text_file
            ",
            params![
                record.article_id,
                record.journal_id,
                record.issue_id,
                record.title,
                record.date,
                record.authors,
                record.start_page,
                record.end_page,
                record.abstract_text,
                record.doi,
                record.pmid,
                record.permalink,
                record.suppressed,
                record.in_press,
                record.open_access,
                record.platform_id,
                record.retraction_doi,
                record.within_library_holdings,
                record.content_location,
                record.full_text_file,
            ],
        )?;
    }
    Ok(())
}

/// Delete article records and derived rows.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `article_ids` - Article ids to delete.
///
/// # Returns
///
/// SQLite result.
pub fn delete_articles(connection: &Connection, article_ids: &[i64]) -> rusqlite::Result<()> {
    for article_id in article_ids {
        connection.execute("DELETE FROM article_search WHERE rowid = ?1", [article_id])?;
        connection.execute(
            "DELETE FROM article_listing WHERE article_id = ?1",
            [article_id],
        )?;
        connection.execute("DELETE FROM articles WHERE article_id = ?1", [article_id])?;
    }
    Ok(())
}

/// Insert or update article search rows.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `records` - Article records.
/// * `journal_title` - Journal title.
///
/// # Returns
///
/// SQLite result.
pub fn upsert_article_search(
    connection: &Connection,
    records: &[ArticleRecord],
    journal_title: &str,
) -> rusqlite::Result<()> {
    for record in records {
        connection.execute(
            "
            INSERT OR REPLACE INTO article_search (
                rowid, article_id, title, abstract, doi, authors, journal_title
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
            ",
            params![
                record.article_id,
                record.article_id,
                record.title.as_deref().unwrap_or(""),
                record.abstract_text.as_deref().unwrap_or(""),
                record.doi.as_deref().unwrap_or(""),
                record.authors.as_deref().unwrap_or(""),
                journal_title,
            ],
        )?;
    }
    Ok(())
}

/// Refresh article listing rows for article ids.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `article_ids` - Article ids.
///
/// # Returns
///
/// SQLite result.
pub fn refresh_article_listing_for_articles(
    connection: &Connection,
    article_ids: &[i64],
) -> rusqlite::Result<()> {
    if article_ids.is_empty() {
        return Ok(());
    }
    let placeholders = std::iter::repeat_n("?", article_ids.len())
        .collect::<Vec<_>>()
        .join(", ");
    let sql = format!(
        "
        INSERT INTO article_listing (
            article_id, journal_id, issue_id, publication_year, date, open_access,
            in_press, suppressed, within_library_holdings, doi, pmid, area
        )
        SELECT
            a.article_id,
            a.journal_id,
            a.issue_id,
            i.publication_year,
            a.date,
            a.open_access,
            a.in_press,
            a.suppressed,
            a.within_library_holdings,
            a.doi,
            a.pmid,
            m.area
        FROM articles a
        LEFT JOIN issues i ON i.issue_id = a.issue_id
        LEFT JOIN journal_meta m ON m.journal_id = a.journal_id
        WHERE a.article_id IN ({placeholders})
        ON CONFLICT(article_id) DO UPDATE SET
            journal_id = excluded.journal_id,
            issue_id = excluded.issue_id,
            publication_year = excluded.publication_year,
            date = excluded.date,
            open_access = excluded.open_access,
            in_press = excluded.in_press,
            suppressed = excluded.suppressed,
            within_library_holdings = excluded.within_library_holdings,
            doi = excluded.doi,
            pmid = excluded.pmid,
            area = excluded.area
        "
    );
    connection.execute(&sql, params_from_iter(article_ids.iter()))?;
    Ok(())
}

/// Mark one journal year as indexed.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
/// * `year` - Publication year.
/// * `updated_at` - Updated timestamp.
///
/// # Returns
///
/// SQLite result.
pub fn mark_year_done(
    connection: &Connection,
    journal_id: i64,
    year: i64,
    updated_at: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_year_state (journal_id, year, status, updated_at)
        VALUES (?1, ?2, 'done', ?3)
        ON CONFLICT(journal_id, year) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        ",
        params![journal_id, year, updated_at],
    )?;
    Ok(())
}

/// Mark one journal as indexed.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `journal_id` - Journal id.
/// * `updated_at` - Updated timestamp.
///
/// # Returns
///
/// SQLite result.
pub fn mark_journal_done(
    connection: &Connection,
    journal_id: i64,
    updated_at: &str,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO journal_state (journal_id, status, updated_at)
        VALUES (?1, 'done', ?2)
        ON CONFLICT(journal_id) DO UPDATE SET
            status = excluded.status,
            updated_at = excluded.updated_at
        ",
        params![journal_id, updated_at],
    )?;
    Ok(())
}

/// Persist index run statistics.
///
/// # Arguments
///
/// * `connection` - Open SQLite connection.
/// * `stats` - Index run statistics.
///
/// # Returns
///
/// SQLite result.
pub fn persist_index_run_stats(
    connection: &Connection,
    stats: &IndexRunStats,
) -> rusqlite::Result<()> {
    connection.execute(
        "
        INSERT INTO index_runs (
            run_id, csv_file, started_at, finished_at, status, total_journals,
            succeeded_journals, failed_journals, resumed_journals, error_summary
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
        ON CONFLICT(run_id) DO UPDATE SET
            csv_file = excluded.csv_file,
            started_at = excluded.started_at,
            finished_at = excluded.finished_at,
            status = excluded.status,
            total_journals = excluded.total_journals,
            succeeded_journals = excluded.succeeded_journals,
            failed_journals = excluded.failed_journals,
            resumed_journals = excluded.resumed_journals,
            error_summary = excluded.error_summary
        ",
        params![
            stats.run_id,
            stats.csv_file,
            stats.started_at,
            stats.finished_at,
            stats.status,
            stats.total_journals,
            stats.succeeded_journals,
            stats.failed_journals,
            stats.resumed_journals,
            stats.error_summary,
        ],
    )?;
    connection.execute(
        "DELETE FROM index_path_stats WHERE run_id = ?1",
        [&stats.run_id],
    )?;
    connection.execute(
        "DELETE FROM index_api_call_stats WHERE run_id = ?1",
        [&stats.run_id],
    )?;
    for path_stats in stats.path_stats.values() {
        connection.execute(
            "
            INSERT INTO index_path_stats (
                run_id, source, path, journal_id, journal_title, status,
                started_at, finished_at, works_count, issues_count,
                article_summaries_count, article_details_count,
                articles_written_count, articles_deleted_no_authors_count,
                error_type, error_message
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)
            ",
            params![
                stats.run_id,
                path_stats.key.source,
                path_stats.key.path,
                path_stats.key.journal_id,
                path_stats.key.journal_title,
                path_stats.status,
                path_stats.started_at,
                path_stats.finished_at,
                path_stats.works_count,
                path_stats.issues_count,
                path_stats.article_summaries_count,
                path_stats.article_details_count,
                path_stats.articles_written_count,
                path_stats.articles_deleted_no_authors_count,
                path_stats.error_type,
                path_stats.error_message,
            ],
        )?;
    }
    for api_stats in stats.api_stats.values() {
        let status_codes_json = python_status_codes_json(&api_stats.status_codes);
        let error_samples_json = serde_json::to_string(&api_stats.error_samples)
            .expect("error samples should serialize");
        connection.execute(
            "
            INSERT INTO index_api_call_stats (
                run_id, source, service, endpoint, method, url_path, journal_id,
                journal_title, logical_calls, attempts, successes, failures,
                retry_count, status_codes_json, transport_errors,
                rate_limit_failures, total_latency_ms, error_samples_json
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
            ",
            params![
                stats.run_id,
                api_stats.key.source,
                api_stats.key.service,
                api_stats.key.endpoint,
                api_stats.key.method,
                api_stats.key.url_path,
                api_stats.key.journal_id,
                api_stats.key.journal_title,
                api_stats.logical_calls,
                api_stats.attempts,
                api_stats.successes,
                api_stats.failures,
                api_stats.retry_count,
                status_codes_json,
                api_stats.transport_errors,
                api_stats.rate_limit_failures,
                api_stats.total_latency_ms,
                error_samples_json,
            ],
        )?;
    }
    Ok(())
}

fn python_status_codes_json(status_codes: &std::collections::BTreeMap<u16, i64>) -> String {
    let fields = status_codes
        .iter()
        .map(|(key, value)| format!("\"{key}\": {value}"))
        .collect::<Vec<_>>();
    format!("{{{}}}", fields.join(", "))
}

#[cfg(test)]
mod tests {
    use rusqlite::Connection;

    use crate::transforms::{ArticleRecord, JournalRecord, MetaRecord};

    use super::{
        init_index_db, refresh_article_listing_for_articles, upsert_article_search,
        upsert_articles, upsert_journal, upsert_meta,
    };

    #[test]
    fn initializes_schema_and_writes_listing_rows() {
        let connection = Connection::open_in_memory().expect("in-memory db should open");
        init_index_db(&connection).expect("schema should initialize");
        upsert_journal(
            &connection,
            &JournalRecord {
                journal_id: 1,
                library_id: "scholarly".into(),
                platform_journal_id: Some("1234-5678".into()),
                title: Some("Test Journal".into()),
                issn: Some("1234-5678".into()),
                eissn: None,
                scimago_rank: None,
                cover_url: None,
                available: Some(1),
                toc_data_approved_and_live: None,
                has_articles: Some(1),
            },
        )
        .expect("journal should insert");
        upsert_meta(
            &connection,
            &MetaRecord {
                journal_id: 1,
                source_csv: "journals.csv".into(),
                area: Some("testing".into()),
                csv_title: Some("Test Journal".into()),
                csv_issn: Some("1234-5678".into()),
                csv_library: Some("scholarly".into()),
                resolved_source: None,
                resolved_source_id: None,
                resolved_title: None,
                resolved_issn: None,
                resolved_eissn: None,
            },
        )
        .expect("meta should insert");
        let article = ArticleRecord {
            article_id: 2,
            journal_id: 1,
            issue_id: None,
            title: Some("Article".into()),
            date: Some("2025-01-01".into()),
            authors: Some("Ada Lovelace".into()),
            start_page: None,
            end_page: None,
            abstract_text: Some("Abstract".into()),
            doi: Some("10.1/a".into()),
            pmid: None,
            permalink: None,
            suppressed: None,
            in_press: Some(1),
            open_access: Some(1),
            platform_id: Some("10.1/a".into()),
            retraction_doi: None,
            within_library_holdings: None,
            content_location: None,
            full_text_file: None,
        };
        upsert_articles(&connection, std::slice::from_ref(&article))
            .expect("article should insert");
        upsert_article_search(&connection, &[article], "Test Journal")
            .expect("search should insert");
        refresh_article_listing_for_articles(&connection, &[2]).expect("listing should refresh");
        let area: String = connection
            .query_row(
                "SELECT area FROM article_listing WHERE article_id = 2",
                [],
                |row| row.get(0),
            )
            .expect("listing should exist");

        assert_eq!(area, "testing");
    }
}
