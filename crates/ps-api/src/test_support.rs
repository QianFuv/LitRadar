//! Test fixtures shared by API route tests.

use std::fs;
use std::path::{Path, PathBuf};

use axum::body::{to_bytes, Body};
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE};
use axum::http::{HeaderMap, Method, Request, StatusCode};
use axum::Router;
use ps_auth::{AuthService, ACCESS_TOKEN_DEFAULT_TTL, SESSION_COOKIE_NAME};
use ps_domain::{UserId, UserResponse};
use ps_storage::{admin_create_invite_code, count_users, migrate_storage, StorageConfig};
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
        fs::create_dir_all(storage_config.index_dir()).expect("index dir should be created");
        migrate_storage(&storage_config).expect("test databases should migrate");
        Self {
            temp_dir,
            storage_config,
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

    /// Build an API router bound to this fixture's project root.
    ///
    /// # Returns
    ///
    /// Axum router ready for one-shot test requests.
    pub(crate) fn router(&self) -> Router {
        let mut config =
            ApiConfig::new(self.project_root().to_path_buf(), TEST_HOST.to_string(), 0);
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
            ps_storage::set_user_admin(self.auth_db_path(), user.id, is_admin)
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
    let connection = Connection::open(path).expect("index db should open");
    connection
        .execute_batch(
            "
            PRAGMA foreign_keys = ON;

            CREATE TABLE journals (
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

            CREATE TABLE journal_meta (
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

            CREATE TABLE issues (
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

            CREATE TABLE articles (
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

            CREATE TABLE article_listing (
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

            CREATE VIRTUAL TABLE article_search
            USING fts5(
                article_id UNINDEXED,
                title,
                abstract,
                doi,
                authors,
                journal_title
            );

            INSERT INTO journals (
                journal_id, library_id, platform_journal_id, title, issn, eissn,
                scimago_rank, cover_url, available, toc_data_approved_and_live,
                has_articles
            ) VALUES (
                101, 'scholarly', 'J-101', 'Fixture Journal', '1234-5678',
                '8765-4321', 1.25, 'https://example.test/cover.png', 1, 1, 1
            );

            INSERT INTO journal_meta (
                journal_id, source_csv, area, csv_title, csv_issn, csv_library,
                resolved_source, resolved_source_id, resolved_title, resolved_issn,
                resolved_eissn
            ) VALUES (
                101, 'fixture.csv', 'Medicine', 'Fixture Journal', '1234-5678',
                'Library A', 'openalex', 'S101', 'Fixture Journal', '1234-5678',
                '8765-4321'
            );

            INSERT INTO issues (
                issue_id, journal_id, publication_year, title, volume, number, date,
                is_valid_issue, suppressed, embargoed, within_subscription
            ) VALUES (
                202401, 101, 2024, 'Volume 1 Issue 1', '1', '1', '2024-01-15',
                1, 0, 0, 1
            );

            INSERT INTO articles (
                article_id, journal_id, issue_id, title, date, authors, start_page,
                end_page, abstract, doi, pmid, permalink, suppressed, in_press,
                open_access, platform_id, retraction_doi, within_library_holdings,
                content_location, full_text_file
            ) VALUES (
                9001, 101, 202401, 'Fixture Article', '2024-01-16',
                'Ada Lovelace; Grace Hopper', '1', '9',
                'Fixture abstract for route and storage tests.',
                '10.1234/fixture', '123456', 'https://example.test/article',
                0, 0, 1, 'P-9001', NULL, 1,
                'https://example.test/content', 'https://example.test/fulltext.pdf'
            );

            INSERT INTO article_listing (
                article_id, journal_id, issue_id, publication_year, date, open_access,
                in_press, suppressed, within_library_holdings, doi, pmid, area
            ) VALUES (
                9001, 101, 202401, 2024, '2024-01-16', 1, 0, 0, 1,
                '10.1234/fixture', '123456', 'Medicine'
            );

            INSERT INTO article_search (
                article_id, title, abstract, doi, authors, journal_title
            ) VALUES (
                9001, 'Fixture Article',
                'Fixture abstract for route and storage tests.',
                '10.1234/fixture', 'Ada Lovelace; Grace Hopper',
                'Fixture Journal'
            );
            ",
        )
        .expect("fixture index schema and data should be created");
}
