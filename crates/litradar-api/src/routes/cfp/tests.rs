//! Authenticated CFP API scope, persistence and pagination regression tests.

use std::fs;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use litradar_domain::cfp::{CfpSeed, CfpSource};
use litradar_index::transforms::CATALOG_CSV_V3_COLUMNS;
use litradar_sources::cfp::{
    CfpAdapter, CfpDocument, CfpSourceConfig, CfpSourceError, CfpTransport, CfpUrlRule,
};
use litradar_storage::business::cfp::{begin_cfp_refresh, import_cfp_seed};
use litradar_worker::cfp::refresh_cfp_source;
use serde_json::{json, Value};

use super::*;
use crate::test_support::{json_request, TestBackend};

fn write_catalog(backend: &TestBackend, name: &str, rows: &[(&str, &str, &str)]) {
    fs::create_dir_all(backend.storage_config().meta_dir()).unwrap();
    let mut csv = CATALOG_CSV_V3_COLUMNS.join(",");
    csv.push('\n');
    for (id, aliases, title) in rows {
        let mut values = vec![""; 16];
        values[0] = id;
        values[1] = aliases;
        values[2] = title;
        values[7] = "Test subject";
        csv.push_str(&values.join(","));
        csv.push('\n');
    }
    fs::write(
        backend
            .storage_config()
            .meta_dir()
            .join(format!("{name}.csv")),
        csv,
    )
    .unwrap();
}

fn source(title: &str, dates: &str) -> CfpSource {
    serde_json::from_value(json!({"catalogIds":["cfp-fixture","cfp-alias"],"journalTitle":"Example Journal","title":title,"scope":"Original English scope.","requirements":"Original manuscripts only.","typeText":"Special Issue","dateText":dates,"sourceUrl":"https://example.org/calls","checkedOn":"2026-09-15","timeZone":"UTC"})).unwrap()
}

fn import_fixture(backend: &TestBackend) {
    let seed = CfpSeed {
        format_version: 1,
        sources: vec![
            source(
                "Open original title",
                "Submission deadline: 31 December 2099",
            ),
            source(
                "Another original title",
                "Submission deadline: 31 December 2099",
            ),
            source(
                "Closed original title",
                "Submission deadline: 1 January 2020",
            ),
        ],
        empty_journals: Vec::new(),
        expected_journals: Some(1),
        expected_notices: Some(3),
    };
    import_cfp_seed(
        backend.auth_db_path(),
        "api-fixture",
        &serde_json::to_vec(&seed).unwrap(),
    )
    .unwrap();
}

#[tokio::test]
async fn cfp_catalog_reads_meta_without_articles_and_distinguishes_unadapted_and_verified_empty() {
    let backend = TestBackend::new();
    write_catalog(
        &backend,
        "fixture",
        &[
            ("cfp-fixture", "cfp-alias", "Example Journal"),
            ("unadapted", "", "Unadapted Journal"),
            (
                "issn-1549-8328",
                "",
                "IEEE Transactions on Circuits and Systems I: Regular Papers",
            ),
        ],
    );
    import_fixture(&backend);
    let user = backend.authenticated_user("cfp_reader", false);
    let auth = user.authorization_header();
    let app = backend.router();
    let unauthorized = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals?db=fixture.sqlite",
        None,
        None,
        None,
    )
    .await;
    assert_eq!(unauthorized.status, StatusCode::UNAUTHORIZED);
    let response = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals?db=fixture.sqlite",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(response.status, StatusCode::OK, "{}", response.payload);
    assert_eq!(response.payload["summary"]["journals"], 3);
    assert_eq!(response.payload["summary"]["adaptedJournals"], 2);
    assert_eq!(response.payload["summary"]["notices"], 3);
    assert!(response.payload["items"]
        .as_array()
        .unwrap()
        .iter()
        .all(|journal| journal.get("notices").is_none() && journal.get("scope").is_none()));
    let unadapted = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals/unadapted/notices?db=fixture.sqlite",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(unadapted.status, StatusCode::OK);
    assert_eq!(unadapted.payload["journal"]["coverage"], "unadapted");
    assert_eq!(unadapted.payload["items"], json!([]));
    let empty = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals/issn-1549-8328/notices?db=fixture.sqlite",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(empty.status, StatusCode::OK);
    assert_eq!(empty.payload["journal"]["coverage"], "adapted");
    assert_eq!(
        empty.payload["journal"]["sourceStatement"],
        "No current call documents available."
    );
    for uri in [
        "/api/cfp/journals?db=missing.sqlite",
        "/api/cfp/journals?db=../auth.sqlite",
        "/api/cfp/journals/foreign/notices?db=fixture.sqlite",
    ] {
        assert_eq!(
            json_request(&app, Method::GET, uri, Some(&auth), None, None)
                .await
                .status,
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn cfp_cursor_is_scoped_authenticated_fixed_time_and_rejects_content_changes() {
    let backend = TestBackend::new();
    write_catalog(
        &backend,
        "fixture",
        &[
            ("cfp-fixture", "cfp-alias", "Example Journal"),
            ("unadapted", "", "Other Journal"),
        ],
    );
    write_catalog(
        &backend,
        "other",
        &[("cfp-fixture", "cfp-alias", "Example Journal")],
    );
    import_fixture(&backend);
    let user = backend.authenticated_user("cursor_reader", false);
    let auth = user.authorization_header();
    let app = backend.router();
    let first = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals/cfp-alias/notices?db=fixture.sqlite&limit=1",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(first.status, StatusCode::OK, "{}", first.payload);
    assert_eq!(first.payload["journal"]["catalogId"], "cfp-fixture");
    assert_eq!(first.payload["page"]["total"], 2);
    assert_eq!(first.payload["items"][0]["state"], "open");
    assert_eq!(
        first.payload["items"][0]["scope"],
        "Original English scope."
    );
    let cursor = first.payload["page"]["next_cursor"].as_str().unwrap();
    let second = json_request(
        &app,
        Method::GET,
        &format!("/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&limit=1&cursor={cursor}"),
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(second.status, StatusCode::OK);
    assert_eq!(second.payload["evaluatedAt"], first.payload["evaluatedAt"]);
    assert_ne!(
        second.payload["items"][0]["id"],
        first.payload["items"][0]["id"]
    );
    assert_eq!(second.payload["page"]["has_more"], false);
    for suffix in [
        format!("cfp-fixture/notices?db=other.sqlite&cursor={cursor}"),
        format!("unadapted/notices?db=fixture.sqlite&cursor={cursor}"),
        format!("cfp-fixture/notices?db=fixture.sqlite&include_closed=true&cursor={cursor}"),
        "cfp-fixture/notices?db=fixture.sqlite&cursor=forged".into(),
    ] {
        assert_eq!(
            json_request(
                &app,
                Method::GET,
                &format!("/api/cfp/journals/{suffix}"),
                Some(&auth),
                None,
                None
            )
            .await
            .status,
            StatusCode::CONFLICT
        );
    }
    let closed = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&include_closed=true",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(closed.payload["page"]["total"], 3);
    assert_eq!(closed.payload["items"][2]["state"], "closed");
    assert_eq!(
        json_request(
            &app,
            Method::GET,
            "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&limit=201",
            Some(&auth),
            None,
            None
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    let mut encrypted: Value = serde_json::from_str(
        &backend
            .secret_codec()
            .decrypt(cursor, CURSOR_CONTEXT)
            .unwrap(),
    )
    .unwrap();
    encrypted["evaluated_at"] = json!(unix_now() - 901);
    let expired = backend
        .secret_codec()
        .encrypt(&encrypted.to_string(), CURSOR_CONTEXT)
        .unwrap();
    assert_eq!(
        json_request(
            &app,
            Method::GET,
            &format!("/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&cursor={expired}"),
            Some(&auth),
            None,
            None
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    write_catalog(
        &backend,
        "fixture",
        &[("cfp-fixture", "cfp-alias", "Renamed catalog title")],
    );
    assert_eq!(
        json_request(
            &app,
            Method::GET,
            &format!("/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&cursor={cursor}"),
            Some(&auth),
            None,
            None
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
}

#[tokio::test]
async fn cfp_backend_refresh_changes_api_without_rebuilding_and_survives_router_restart() {
    let backend = TestBackend::new();
    write_catalog(
        &backend,
        "fixture",
        &[("cfp-fixture", "cfp-alias", "Example Journal")],
    );
    import_fixture(&backend);
    let user = backend.authenticated_user("refresh_reader", false);
    let auth = user.authorization_header();
    let app = backend.router();
    let first = json_request(
        &app,
        Method::GET,
        "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&limit=1",
        Some(&auth),
        None,
        None,
    )
    .await;
    let cursor = first.payload["page"]["next_cursor"].as_str().unwrap();
    struct Transport;
    impl CfpTransport for Transport {
        fn fetch(
            &self,
            config: &CfpSourceConfig,
            _url: &str,
            _deadline: Instant,
        ) -> Result<CfpDocument, CfpSourceError> {
            Ok(CfpDocument {final_url:config.discovery_url.clone(),text:"<h1>Example Journal</h1><h2>Call for papers</h2><h3>New original source title</h3><p>New original topic.</p><p>Submission deadline: 31 December 2099</p>".into(),format:"html".into()})
        }
    }
    let config = CfpSourceConfig {
        source_key: "journal:cfp-fixture".into(),
        catalog_ids: vec!["cfp-fixture".into(), "cfp-alias".into()],
        journal_title: "Example Journal".into(),
        discovery_url: "https://example.org/calls".into(),
        adapter: CfpAdapter::ElsevierCalls,
        config_version: 1,
        allowed_urls: vec![CfpUrlRule {
            host: "example.org".into(),
            path_prefix: "/calls".into(),
        }],
        identity_texts: vec!["Example Journal".into()],
        empty_statements: Vec::new(),
        capability_note: None,
        retains_previous_notices: false,
    };
    assert_eq!(
        refresh_cfp_source(
            backend.auth_db_path(),
            &config,
            &Transport,
            Instant::now() + Duration::from_secs(10)
        )
        .unwrap()
        .status,
        "success"
    );
    assert_eq!(
        json_request(
            &app,
            Method::GET,
            &format!("/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite&cursor={cursor}"),
            Some(&auth),
            None,
            None
        )
        .await
        .status,
        StatusCode::CONFLICT
    );
    let restarted = backend.router();
    let fresh = json_request(
        &restarted,
        Method::GET,
        "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(
        fresh.payload["items"][0]["title"],
        "New original source title"
    );
    assert_eq!(fresh.payload["page"]["total"], 1);
    assert_eq!(fresh.payload["journal"]["refreshStatus"], "success");
    begin_cfp_refresh(
        backend.auth_db_path(),
        "journal:cfp-fixture",
        unix_now() - 100,
        1,
    )
    .unwrap();
    let expired = json_request(
        &restarted,
        Method::GET,
        "/api/cfp/journals/cfp-fixture/notices?db=fixture.sqlite",
        Some(&auth),
        None,
        None,
    )
    .await;
    assert_eq!(expired.payload["journal"]["refreshStatus"], "failed");
    assert_eq!(
        expired.payload["items"][0]["title"],
        "New original source title"
    );
}
