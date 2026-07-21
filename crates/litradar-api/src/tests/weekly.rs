//! Direct HTTP contract tests for authenticated weekly updates.

use std::fs;
use std::path::Path;

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
            "generated_at": "2024-01-23T00:00:00Z",
            "run_id": "alpha-run",
            "notifiable_article_ids": [newer_database.article_id]
        }),
    );
    write_weekly_manifest(
        &backend,
        "beta.changes.json",
        &json!({
            "db_name": older_database.db_name,
            "generated_at": "2024-01-22T00:00:00Z",
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
    assert_eq!(response.payload["window_start"], "2024-01-16T00:00:00Z");
    assert_eq!(response.payload["window_end"], "2024-01-23T00:00:00Z");
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
            "generated_at": "2024-01-22T00:00:00Z",
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
    assert_eq!(response.payload["window_start"], "2024-01-15T00:00:00Z");
    assert_eq!(response.payload["window_end"], "2024-01-22T00:00:00Z");
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
            "generated_at": "2024-01-22T00:00:00Z",
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
            "#,
        )
        .expect("additional weekly articles should insert");
}
