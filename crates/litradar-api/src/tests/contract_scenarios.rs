use axum::http::{Method, StatusCode};
use serde_json::json;

use crate::test_support::{
    assert_api_scenario, json_request, replace_json_pointer, TestBackend, TEST_PASSWORD,
};

#[tokio::test]
async fn login_response_matches_shared_scenario() {
    let backend = TestBackend::new();
    backend.authenticated_user("scenario_admin", true);
    let response = json_request(
        &backend.router(),
        Method::POST,
        "/api/auth/login",
        None,
        None,
        Some(json!({
            "username": "scenario_admin",
            "password": TEST_PASSWORD
        })),
    )
    .await;
    let mut payload = response.payload;
    replace_json_pointer(&mut payload, "/expires_at", json!(0));

    assert_eq!(response.status, StatusCode::OK);
    assert!(response.headers.contains_key("set-cookie"));
    assert!(payload.get("token").is_none());
    assert_api_scenario("login.json", &payload);
}

#[tokio::test]
async fn article_page_matches_shared_scenario() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("scenario_reader", false);
    backend.create_index_database("scenario.sqlite");
    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/articles?db=scenario.sqlite&limit=10&offset=0&include_total=true",
        Some(&user.authorization_header()),
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::OK);
    assert_api_scenario("article-page.json", &response.payload);
}

#[tokio::test]
async fn masked_notification_settings_match_shared_scenario() {
    let backend = TestBackend::new();
    let user = backend.authenticated_user("scenario_reader", false);
    let response = json_request(
        &backend.router(),
        Method::PUT,
        "/api/tracking/notification-settings",
        Some(&user.authorization_header()),
        None,
        Some(json!({
            "keywords": ["oncology", "genomics"],
            "directions": ["precision medicine"],
            "selected_databases": [],
            "delivery_method": "pushplus",
            "pushplus_token": "fixture-pushplus-secret",
            "pushplus_template": "markdown",
            "pushplus_topic": "LitRadar",
            "pushplus_channel": "wechat",
            "sync_to_tracking_folder": false,
            "ai_base_url": "https://ai.invalid/v1",
            "ai_api_key": "fixture-primary-secret",
            "ai_model": "fixture-primary",
            "ai_system_prompt": "Summarize safely.",
            "ai_backup_base_url": "https://backup.invalid/v1",
            "ai_backup_api_key": "fixture-backup-secret",
            "ai_backup_model": "fixture-backup",
            "ai_backup_system_prompt": "Provide a fallback summary.",
            "ai_retry_attempts": 3,
            "enabled": true
        })),
    )
    .await;
    let mut payload = response.payload;
    replace_json_pointer(&mut payload, "/created_at", json!(0));
    replace_json_pointer(&mut payload, "/updated_at", json!(0));

    assert_eq!(response.status, StatusCode::OK);
    assert_api_scenario("masked-notification-settings.json", &payload);
}

#[tokio::test]
async fn authentication_error_matches_shared_scenario() {
    let backend = TestBackend::new();
    let response = json_request(
        &backend.router(),
        Method::GET,
        "/api/auth/me",
        None,
        None,
        None,
    )
    .await;

    assert_eq!(response.status, StatusCode::UNAUTHORIZED);
    assert_api_scenario("error.json", &response.payload);
}
