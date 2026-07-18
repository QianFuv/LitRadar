//! Test fixtures shared by API route tests.

use std::fs;
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use litradar_auth::{AuthService, ACCESS_TOKEN_DEFAULT_TTL, SESSION_COOKIE_NAME};
use litradar_domain::{UserId, UserResponse};
use litradar_storage::{
    admin_create_invite_code, count_users, migrate_storage, SecretCodec, StorageConfig,
};
use rusqlite::Connection;
use serde_json::Value;
use tempfile::{tempdir, TempDir};
use tower::ServiceExt;

use crate::{build_router, ApiConfig};

const TEST_HOST: &str = "127.0.0.1";
const TEST_PASSWORD: &str = "fixture-password";

/// Deterministic backend fixture rooted in a temporary project directory.
pub(crate) struct TestBackend {
    temp_dir: TempDir,
    storage_config: StorageConfig,
    secret_codec: SecretCodec,
}

impl TestBackend {
    /// Create a backend fixture with initialized data directories and auth schema.
    ///
    /// # Returns
    ///
    /// Backend fixture ready for API and storage tests.
    pub(crate) fn new() -> Self {
        let temp_dir = tempdir().expect("temp dir should be created");
        let storage_config = StorageConfig::from_project_root(temp_dir.path());
        let secret_key_file = temp_dir.path().join("secret.key");
        fs::write(&secret_key_file, [42_u8; 32]).expect("secret key should write");
        fs::create_dir_all(storage_config.index_dir()).expect("index dir should be created");
        migrate_storage(&storage_config).expect("test databases should migrate");
        Self {
            temp_dir,
            storage_config,
            secret_codec: SecretCodec::load(secret_key_file)
                .expect("fixture secret codec should load"),
        }
    }

    /// Return the temporary project root.
    ///
    /// # Returns
    ///
    /// Project root path used by the router.
    pub(crate) fn project_root(&self) -> &Path {
        self.temp_dir.path()
    }

    /// Return storage paths for direct repository calls.
    ///
    /// # Returns
    ///
    /// Storage configuration bound to the temporary root.
    pub(crate) fn storage_config(&self) -> &StorageConfig {
        &self.storage_config
    }

    /// Return the fixture auth database path.
    ///
    /// # Returns
    ///
    /// Auth database path.
    pub(crate) fn auth_db_path(&self) -> &Path {
        self.storage_config.auth_db_path()
    }

    /// Return the fixture deployment secret codec.
    ///
    /// # Returns
    ///
    /// Secret codec initialized from the temporary key file.
    pub(crate) fn secret_codec(&self) -> &SecretCodec {
        &self.secret_codec
    }

    /// Build an API router bound to this fixture's project root.
    ///
    /// # Returns
    ///
    /// Axum router ready for one-shot test requests.
    pub(crate) fn router(&self) -> Router {
        let mut config = ApiConfig::new(
            self.project_root().to_path_buf(),
            TEST_HOST.to_string(),
            0,
            self.project_root().join("secret.key"),
        );
        config.mcp_allowed_hosts = vec!["localhost".to_string(), TEST_HOST.to_string()];
        build_router(config)
    }

    /// Register a user and create a bearer token.
    ///
    /// # Arguments
    ///
    /// * `username` - Fixture username.
    /// * `is_admin` - Whether the user should have admin privileges.
    ///
    /// # Returns
    ///
    /// Authenticated user fixture.
    pub(crate) fn authenticated_user(&self, username: &str, is_admin: bool) -> TestUser {
        let service = AuthService::new(self.auth_db_path());
        let mut user = if count_users(self.auth_db_path()).expect("user count should load") == 0 {
            service
                .bootstrap_admin(username, TEST_PASSWORD)
                .expect("first fixture administrator should bootstrap")
        } else {
            let invite_code = admin_create_invite_code(self.auth_db_path())
                .expect("invite code should be created")
                .code;
            service
                .register(username, TEST_PASSWORD, Some(&invite_code))
                .expect("invited fixture user should register")
        };
        if user.is_admin != is_admin {
            litradar_storage::set_user_admin(self.auth_db_path(), user.id, is_admin)
                .expect("admin flag should update");
            user.is_admin = is_admin;
        }
        let token = service
            .create_access_token(user.id, "fixture", ACCESS_TOKEN_DEFAULT_TTL)
            .expect("access token should be created")
            .token;
        TestUser { user, token }
    }

    /// Create a deterministic index database with one journal, issue, article, listing, and search row.
    ///
    /// # Arguments
    ///
    /// * `db_name` - SQLite database filename under `data/index`.
    ///
    /// # Returns
    ///
    /// Created index fixture metadata.
    pub(crate) fn create_index_database(&self, db_name: &str) -> FixtureIndexDatabase {
        let path = self.storage_config.index_dir().join(db_name);
        create_fixture_index_database(&path);
        FixtureIndexDatabase {
            db_name: db_name.to_string(),
            path,
            journal_id: 101,
            issue_id: 202401,
            article_id: 9001,
        }
    }
}

/// Authenticated API user fixture.
pub(crate) struct TestUser {
    /// User profile stored in the auth database.
    pub(crate) user: UserResponse,
    token: String,
}

impl TestUser {
    /// Return the user identifier.
    ///
    /// # Returns
    ///
    /// User id.
    pub(crate) fn user_id(&self) -> UserId {
        self.user.id
    }

    /// Return a bearer authorization header value.
    ///
    /// # Returns
    ///
    /// Header value for the `Authorization` header.
    pub(crate) fn authorization_header(&self) -> String {
        format!("Bearer {}", self.token)
    }

    /// Return a session cookie header value.
    ///
    /// # Returns
    ///
    /// Header value for the `Cookie` header.
    pub(crate) fn cookie_header(&self) -> String {
        format!("{SESSION_COOKIE_NAME}={}", self.token)
    }
}

/// Deterministic index database fixture metadata.
pub(crate) struct FixtureIndexDatabase {
    /// Database filename under `data/index`.
    pub(crate) db_name: String,
    /// Absolute fixture database path.
    pub(crate) path: PathBuf,
    /// Fixture journal id.
    pub(crate) journal_id: i64,
    /// Fixture issue id.
    pub(crate) issue_id: i64,
    /// Fixture article id.
    pub(crate) article_id: i64,
}

/// JSON response returned by API test requests.
pub(crate) struct JsonTestResponse {
    /// HTTP response status.
    pub(crate) status: StatusCode,
    /// HTTP response headers.
    pub(crate) headers: HeaderMap,
    /// Parsed JSON response body.
    pub(crate) payload: Value,
}

/// Send a JSON API request through an Axum router.
///
/// # Arguments
///
/// * `app` - Router under test.
/// * `method` - HTTP method.
/// * `uri` - Request URI.
/// * `authorization` - Optional `Authorization` header value.
/// * `cookie` - Optional `Cookie` header value.
/// * `payload` - Optional JSON request body.
///
/// # Returns
///
/// Parsed JSON response metadata and body.
pub(crate) async fn json_request(
    app: &Router,
    method: Method,
    uri: &str,
    authorization: Option<&str>,
    cookie: Option<&str>,
    payload: Option<Value>,
) -> JsonTestResponse {
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(value) = authorization {
        builder = builder.header(AUTHORIZATION, value);
    }
    if let Some(value) = cookie {
        builder = builder.header(COOKIE, value);
    }
    let body = if let Some(value) = payload {
        builder = builder.header(CONTENT_TYPE, "application/json");
        Body::from(value.to_string())
    } else {
        Body::empty()
    };
    let response = app
        .clone()
        .oneshot(builder.body(body).expect("request should build"))
        .await
        .expect("response should be returned");
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body should read");
    let payload = if body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&body).expect("body should be JSON")
    };
    JsonTestResponse {
        status,
        headers,
        payload,
    }
}

fn create_fixture_index_database(path: &Path) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("index db parent should be created");
    }
    litradar_storage::migrate_index_database(path, None)
        .expect("fixture index schema should migrate");
    let connection = Connection::open(path).expect("index db should open");
    connection
        .execute_batch(
            r#"
            PRAGMA foreign_keys = ON;

            INSERT INTO journals (
                journal_id, catalog_id, title, title_aliases_json, issns_json,
                issn, eissn, area, utd_rank, utd_rating, abs_rank, abs_rating,
                fms_rank, fms_rating, fmscn_rank, fmscn_rating
            ) VALUES (
                101, 'fixture-journal', 'Fixture Journal', '["Fixture J."]',
                '["1234-5679","2049-3630"]', '1234-5679', '2049-3630',
                'Medicine', '1', 'A', NULL, NULL, NULL, NULL, NULL, NULL
            );

            INSERT INTO issues (
                issue_id, journal_id, publication_year, title, volume, number, date
            ) VALUES (
                202401, 101, 2024, 'Volume 1 Issue 1', '1', '1', '2024-01-15'
            );

            INSERT INTO articles (
                article_id, journal_id, issue_id, title, publication_year, date,
                authors_json, start_page, end_page, abstract_text, doi, pmid,
                open_access, in_press, retraction_doi
            ) VALUES (
                9001, 101, 202401, 'Fixture Article', 2024, '2024-01-16',
                '["Ada Lovelace","Grace Hopper"]', '1', '9',
                'Fixture abstract for route and storage tests.',
                '10.1234/fixture', '123456', 1, 0, NULL
            );

            INSERT INTO article_listing (
                article_id, journal_id, issue_id, publication_year, date, open_access,
                in_press, doi, pmid, area
            ) VALUES (
                9001, 101, 202401, 2024, '2024-01-16', 1, 0,
                '10.1234/fixture', '123456', 'Medicine'
            );

            INSERT INTO article_search (
                rowid, article_id, title, abstract_text, doi, pmid, authors,
                journal_title
            ) VALUES (
                9001, 9001, 'Fixture Article',
                'Fixture abstract for route and storage tests.',
                '10.1234/fixture', '123456', 'Ada Lovelace Grace Hopper',
                'Fixture Journal'
            );
            "#,
        )
        .expect("fixture index schema and data should be created");
}
