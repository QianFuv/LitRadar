//! Direct HTTP contract tests for authenticated weekly updates.

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{Method, StatusCode};
use rusqlite::Connection;
use serde_json::{json, Value};

use crate::test_support::{assert_api_scenario, json_request, replace_json_pointer, TestBackend};
use crate::AUTHENTICATED_CACHE_CONTROL;

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_match_shared_scenario() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("scenario_reader", false);
    let database = backend.create_index_database("scenario.sqlite");
    backend.create_weekly_manifest(&database);
    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;
    let mut payload = response.payload;
    replace_json_pointer(&mut payload, "/generated_at", json!("2024-01-22T00:00:00Z"));
    replace_json_pointer(&mut payload, "/window_start", json!("2024-01-15T00:00:00Z"));
    replace_json_pointer(&mut payload, "/window_end", json!("2024-01-22T00:00:00Z"));
    replace_json_pointer(
        &mut payload,
        "/databases/0/generated_at",
        json!("2024-01-22T00:00:00Z"),
    );

    assert_eq!(response.status, StatusCode::OK);
    assert_api_scenario("weekly-updates.json", &payload);
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_group_and_order_route_payload() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_order_reader", false);
    let older_database = backend.create_index_database("beta.sqlite");
    insert_additional_weekly_articles(&older_database.path);
    let newer_database = backend.create_index_database("alpha.sqlite");
    write_weekly_manifest(
        &backend,
        "alpha.changes.json",
        &json!({
            "db_name": newer_database.db_name,
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "alpha-run",
            "notifiable_article_ids": [newer_database.article_id]
        }),
    );
    write_weekly_manifest(
        &backend,
        "beta.changes.json",
        &json!({
            "db_name": older_database.db_name,
            "generated_at": weekly_timestamp_days_ago(1),
            "run_id": "beta-run",
            "notifiable_article_ids": [9003, 9002, older_database.article_id]
        }),
    );

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response
            .headers
            .get(CACHE_CONTROL)
            .expect("cache-control should exist"),
        AUTHENTICATED_CACHE_CONTROL
    );
    assert_eq!(
        response
            .headers
            .get(CONTENT_TYPE)
            .expect("content-type should exist"),
        "application/json"
    );
    assert_eq!(
        response.payload["window_end"],
        response.payload["generated_at"]
    );
    assert_ne!(
        response.payload["window_start"],
        response.payload["window_end"]
    );
    assert_eq!(
        response.payload["databases"]
            .as_array()
            .expect("databases should be an array")
            .iter()
            .map(|database| database["db_name"]
                .as_str()
                .expect("db name should be text"))
            .collect::<Vec<_>>(),
        vec!["alpha.sqlite", "beta.sqlite"]
    );
    assert_eq!(response.payload["databases"][1]["new_article_count"], 3);
    assert_eq!(
        response.payload["databases"][1]["journals"]
            .as_array()
            .expect("journals should be an array")
            .iter()
            .map(|journal| journal["journal_id"]
                .as_str()
                .expect("journal id should be text"))
            .collect::<Vec<_>>(),
        vec!["101", "102"]
    );
    assert_eq!(
        response.payload["databases"][1]["journals"][0]["articles"]
            .as_array()
            .expect("articles should be an array")
            .iter()
            .map(|article| article["article_id"]
                .as_str()
                .expect("article id should be text"))
            .collect::<Vec<_>>(),
        vec!["9002", "9001"]
    );
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_merge_current_history_and_reject_out_of_window_manifests() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_history_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    insert_additional_weekly_articles(&database.path);
    let current_timestamp = current_epoch_seconds();
    let current_payload = json!({
        "db_name": database.db_name,
        "generated_at": current_timestamp.to_string(),
        "run_id": "current-run",
        "notifiable_article_ids": [database.article_id]
    });
    write_weekly_manifest(&backend, "fixture.changes.json", &current_payload);
    write_weekly_history_manifest(
        &backend,
        &"11".repeat(32),
        &json!({
            "db_name": database.db_name,
            "generated_at": current_timestamp.saturating_sub(86_400).to_string(),
            "run_id": "history-run",
            "notifiable_article_ids": [9002, database.article_id]
        }),
    );
    write_weekly_history_manifest(&backend, &"22".repeat(32), &current_payload);
    write_weekly_history_manifest(
        &backend,
        &"33".repeat(32),
        &json!({
            "db_name": database.db_name,
            "generated_at": current_timestamp.saturating_sub(8 * 86_400).to_string(),
            "run_id": "expired-run",
            "notifiable_article_ids": [9003]
        }),
    );
    write_weekly_history_manifest(
        &backend,
        &"44".repeat(32),
        &json!({
            "db_name": database.db_name,
            "generated_at": current_timestamp.saturating_add(86_400).to_string(),
            "run_id": "future-run",
            "notifiable_article_ids": [9003]
        }),
    );

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.payload["databases"].as_array().map(Vec::len),
        Some(1)
    );
    assert_eq!(response.payload["databases"][0]["run_id"], "current-run");
    assert_eq!(response.payload["databases"][0]["new_article_count"], 2);
    let article_ids = response.payload["databases"][0]["journals"]
        .as_array()
        .expect("journals should be an array")
        .iter()
        .flat_map(|journal| {
            journal["articles"]
                .as_array()
                .expect("articles should be an array")
        })
        .map(|article| {
            article["article_id"]
                .as_str()
                .expect("article id should be text")
        })
        .collect::<Vec<_>>();
    assert_eq!(article_ids, vec!["9001", "9002"]);
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn legacy_weekly_updates_reject_more_than_two_thousand_manifest_references() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_legacy_limit_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    write_weekly_manifest(
        &backend,
        "legacy-limit.changes.json",
        &json!({
            "db_name": database.db_name,
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "legacy-limit",
            "notifiable_article_ids": (1..=2_001_i64).collect::<Vec<_>>()
        }),
    );

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        response.payload["detail"],
        "Weekly updates exceed 2000 articles; use /api/weekly-updates/summary and /api/weekly-updates/articles"
    );
    assert!(response.payload.get("databases").is_none());
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_summary_returns_counts_without_article_bodies() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_summary_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    insert_additional_weekly_articles(&database.path);
    write_weekly_manifest(
        &backend,
        "summary.changes.json",
        &json!({
            "db_name": database.db_name,
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "summary-run",
            "notifiable_article_ids": [9001, 9002, 9003, 9999]
        }),
    );

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates/summary",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.payload["databases"][0]["new_article_count"], 3);
    assert_eq!(
        response.payload["databases"][0]["journals"]
            .as_array()
            .expect("journals should be an array")
            .iter()
            .map(|journal| (
                journal["journal_id"]
                    .as_str()
                    .expect("journal id should be text"),
                journal["new_article_count"]
                    .as_u64()
                    .expect("journal count should be unsigned")
            ))
            .collect::<Vec<_>>(),
        vec![("101", 2), ("102", 1)]
    );
    let serialized =
        serde_json::to_string(&response.payload).expect("weekly summary response should serialize");
    assert!(!serialized.contains("\"articles\""));
    assert!(!serialized.contains("\"abstract\""));
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_article_route_pages_searches_and_freezes_manifest_membership() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_page_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    insert_additional_weekly_articles(&database.path);
    write_weekly_manifest(
        &backend,
        "current.changes.json",
        &json!({
            "db_name": database.db_name,
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "current-run",
            "notifiable_article_ids": [9001, 9002]
        }),
    );
    let app = backend.router();
    let authorization = user.authorization_header();
    let summary = json_request(
        &app,
        Method::GET,
        "/api/weekly-updates/summary",
        Some(&authorization),
        None,
        None,
    )
    .await;
    let window_end = summary.payload["window_end"]
        .as_str()
        .expect("summary window end should be text")
        .replace(':', "%3A");
    write_weekly_manifest(
        &backend,
        "future.changes.json",
        &json!({
            "db_name": database.db_name,
            "generated_at": current_epoch_seconds().saturating_add(86_400).to_string(),
            "run_id": "future-run",
            "notifiable_article_ids": [9003]
        }),
    );

    let first = json_request(
        &app,
        Method::GET,
        &format!(
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end={window_end}&limit=1"
        ),
        Some(&authorization),
        None,
        None,
    )
    .await;

    assert_eq!(first.status, StatusCode::OK);
    assert_eq!(first.payload["items"][0]["article_id"], "9002");
    assert_eq!(
        first.payload["items"][0]["abstract"],
        "Second fixture abstract."
    );
    assert_eq!(first.payload["page"]["has_more"], true);
    let cursor = first.payload["page"]["next_cursor"]
        .as_str()
        .expect("first page should provide a cursor")
        .replace('|', "%7C");
    let second = json_request(
        &app,
        Method::GET,
        &format!(
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end={window_end}&limit=1&cursor={cursor}"
        ),
        Some(&authorization),
        None,
        None,
    )
    .await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.payload["items"][0]["article_id"], "9001");
    assert_eq!(second.payload["page"]["has_more"], false);
    assert!(second.payload["page"]["next_cursor"].is_null());

    let searched = json_request(
        &app,
        Method::GET,
        &format!(
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end={window_end}&q=Second+Fixture"
        ),
        Some(&authorization),
        None,
        None,
    )
    .await;
    assert_eq!(searched.status, StatusCode::OK);
    assert_eq!(searched.payload["items"][0]["article_id"], "9002");
    let future = json_request(
        &app,
        Method::GET,
        &format!(
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=102&window_end={window_end}&q=Alpha"
        ),
        Some(&authorization),
        None,
        None,
    )
    .await;
    assert_eq!(future.status, StatusCode::OK);
    assert_eq!(future.payload["items"], json!([]));
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_article_route_rejects_invalid_queries_and_missing_resources() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_invalid_page_reader", false);
    backend.create_index_database("fixture.sqlite");
    let app = backend.router();
    let authorization = user.authorization_header();
    let cases = [
        ("/api/weekly-updates/articles", StatusCode::BAD_REQUEST, "db is required"),
        (
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=0&window_end=2026-08-24T00%3A00%3A00Z",
            StatusCode::BAD_REQUEST,
            "journal_id must be greater than 0",
        ),
        (
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end=not-a-date",
            StatusCode::BAD_REQUEST,
            "window_end must be a valid RFC3339 timestamp",
        ),
        (
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end=2026-08-24T00%3A00%3A00Z&limit=201",
            StatusCode::BAD_REQUEST,
            "limit must be between 1 and 200",
        ),
        (
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=101&window_end=2026-08-24T00%3A00%3A00Z&cursor=invalid",
            StatusCode::BAD_REQUEST,
            "Invalid cursor",
        ),
        (
            "/api/weekly-updates/articles?db=missing.sqlite&journal_id=101&window_end=2026-08-24T00%3A00%3A00Z",
            StatusCode::NOT_FOUND,
            "Database not found",
        ),
        (
            "/api/weekly-updates/articles?db=fixture.sqlite&journal_id=999&window_end=2026-08-24T00%3A00%3A00Z",
            StatusCode::NOT_FOUND,
            "Journal not found",
        ),
    ];

    for (uri, expected_status, expected_detail) in cases {
        let response = json_request(&app, Method::GET, uri, Some(&authorization), None, None).await;
        assert_eq!(response.status, expected_status, "{uri}");
        assert_eq!(response.payload["detail"], expected_detail, "{uri}");
    }
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_return_an_empty_seven_day_window() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_empty_reader", false);

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(response.payload["databases"], json!([]));
    assert_eq!(
        response.payload["window_end"],
        response.payload["generated_at"]
    );
    assert_ne!(
        response.payload["window_start"],
        response.payload["window_end"]
    );
    assert_eq!(
        response
            .payload
            .as_object()
            .expect("response should be an object")
            .len(),
        4
    );
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_ignore_article_discovery_filters() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_filter_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    backend.create_weekly_manifest(&database);
    let app = backend.router();
    let authorization = user.authorization_header();
    let unfiltered = json_request(
        &app,
        Method::GET,
        "/api/weekly-updates",
        Some(&authorization),
        None,
        None,
    )
    .await;
    let filtered = json_request(
        &app,
        Method::GET,
        "/api/weekly-updates?db=missing.sqlite&area=Missing&q=missing&year=1900",
        Some(&authorization),
        None,
        None,
    )
    .await;
    let mut unfiltered_payload = unfiltered.payload;
    let mut filtered_payload = filtered.payload;
    replace_json_pointer(&mut unfiltered_payload, "/generated_at", json!("stable"));
    replace_json_pointer(&mut filtered_payload, "/generated_at", json!("stable"));

    assert_eq!(unfiltered.status, StatusCode::OK);
    assert_eq!(filtered.status, StatusCode::OK);
    assert_eq!(filtered_payload, unfiltered_payload);
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_require_authentication() {
    let backend = TestBackend::new();

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        None,
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_api_scenario("error.json", &response.payload);
    assert_eq!(
        response
            .headers
            .get(CACHE_CONTROL)
            .expect("cache-control should exist"),
        AUTHENTICATED_CACHE_CONTROL
    );
    assert_eq!(
        response
            .headers
            .get(CONTENT_TYPE)
            .expect("content-type should exist"),
        "application/json"
    );
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_skip_unavailable_databases() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_missing_reader", false);
    write_weekly_manifest(
        &backend,
        "missing.changes.json",
        &json!({
            "db_name": "missing.sqlite",
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "missing-run",
            "notifiable_article_ids": [9001]
        }),
    );

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_eq!(
        response.payload["window_end"],
        response.payload["generated_at"]
    );
    assert_eq!(response.payload["databases"], json!([]));
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_reject_malformed_databases() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_database_error_reader", false);
    let app = backend.router();
    fs::write(
        backend.storage_config().index_dir().join("broken.sqlite"),
        b"not a SQLite database",
    )
    .expect("malformed database should write");
    write_weekly_manifest(
        &backend,
        "broken.changes.json",
        &json!({
            "db_name": "broken.sqlite",
            "generated_at": weekly_timestamp_days_ago(0),
            "run_id": "broken-run",
            "notifiable_article_ids": [9001]
        }),
    );

    let response = json_request(
        &app,
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.payload, json!({"detail": "Internal Server Error"}));
}

#[tokio::test]
#[cfg_attr(
    miri,
    ignore = "Miri does not support Tokio's Windows IOCP runtime initialization"
)]
async fn weekly_updates_do_not_return_partial_payloads_for_malformed_manifests() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("weekly_manifest_error_reader", false);
    let database = backend.create_index_database("fixture.sqlite");
    backend.create_weekly_manifest(&database);
    write_raw_weekly_manifest(&backend, "malformed.changes.json", br#"{"db_name":"#);

    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/weekly-updates",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::INTERNAL_SERVER_ERROR);
    assert_eq!(response.payload, json!({"detail": "Internal Server Error"}));
    assert!(response.payload.get("databases").is_none());
}

fn write_weekly_manifest(backend: &TestBackend, file_name: &str, payload: &Value) {
    let bytes = serde_json::to_vec_pretty(payload).expect("weekly manifest should serialize");
    write_raw_weekly_manifest(backend, file_name, &bytes);
}

fn write_raw_weekly_manifest(backend: &TestBackend, file_name: &str, bytes: &[u8]) {
    let push_state_dir = backend.project_root().join("data").join("push_state");
    fs::create_dir_all(&push_state_dir).expect("push state dir should be created");
    fs::write(push_state_dir.join(file_name), bytes).expect("weekly manifest should write");
}

fn write_weekly_history_manifest(backend: &TestBackend, digest: &str, payload: &Value) {
    let history_directory = backend
        .project_root()
        .join("data")
        .join("push_state")
        .join("history")
        .join("fixture");
    fs::create_dir_all(&history_directory).expect("history directory should create");
    fs::write(
        history_directory.join(format!("{digest}.changes.json")),
        serde_json::to_vec_pretty(payload).expect("history manifest should serialize"),
    )
    .expect("history manifest should write");
}

fn current_epoch_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("test time should follow the Unix epoch")
        .as_secs()
}

fn weekly_timestamp_days_ago(days: u64) -> String {
    current_epoch_seconds()
        .saturating_sub(days.saturating_mul(86_400))
        .to_string()
}

fn insert_additional_weekly_articles(path: &Path) {
    let connection = Connection::open(path).expect("index database should open");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json,
                issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                fms_rank, fms_rating, fmscn_rank, fmscn_rating
            ) VALUES (
                102, 'alpha-journal', 'Alpha Journal', '[]',
                '["1111-1111","2222-2222"]', '1111-1111', '2222-2222',
                'Economics', NULL, NULL, NULL, NULL, NULL, NULL, NULL, NULL
            );

            INSERT INTO issues (
                issue_id, journal_id, publication_year, title, volume, number, date
            ) VALUES (
                202402, 102, 2024, 'Volume 2 Issue 1', '2', '1', '2024-01-17'
            );

            INSERT INTO articles (
                article_id, journal_id, issue_id, title, publication_year, date,
                authors_json, start_page, end_page, abstract_text, doi, pmid,
                open_access, in_press
            ) VALUES
            (
                9002, 101, 202401, 'Second Fixture Article', 2024, '2024-01-17',
                '["Katherine Johnson"]', '10', '18', 'Second fixture abstract.',
                '10.1234/fixture-2', '123457', 0, 0
            ),
            (
                9003, 102, 202402, 'Alpha Journal Article', 2024, '2024-01-18',
                '["Dorothy Vaughan"]', '1', '8', 'Alpha journal fixture abstract.',
                '10.1234/fixture-3', '123458', 1, 0
            );

            INSERT INTO article_listing (
                article_id, journal_id, issue_id, publication_year, date,
                open_access, in_press, doi, pmid, area
            )
            SELECT
                a.article_id, a.journal_id, a.issue_id, a.publication_year, a.date,
                a.open_access, a.in_press, a.doi, a.pmid, j.area
            FROM articles a JOIN journals j ON j.journal_id = a.journal_id
            WHERE a.article_id IN (9002, 9003);

            INSERT INTO article_search(
                rowid, article_id, title, abstract_text, doi, pmid, authors, journal_title
            ) VALUES
            (
                9002, 9002, 'Second Fixture Article', 'Second fixture abstract.',
                '10.1234/fixture-2', '123457', 'Katherine Johnson', 'Fixture Journal'
            ),
            (
                9003, 9003, 'Alpha Journal Article', 'Alpha journal fixture abstract.',
                '10.1234/fixture-3', '123458', 'Dorothy Vaughan', 'Alpha Journal'
            );
            "#,
        )
        .expect("additional weekly articles should insert");
}
