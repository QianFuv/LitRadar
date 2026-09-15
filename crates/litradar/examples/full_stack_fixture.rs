//! Marker-guarded deterministic data seeder for real-backend browser tests.

use std::error::Error;
use std::fs;
use std::io::{self, Read, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use litradar_auth::AuthService;
use litradar_domain::cfp::CfpSeed;
use litradar_domain::{
    ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft,
    JournalRankings, ProviderBatch, ProviderProgress,
};
use litradar_index::schema::{open_content_db, reconcile_catalog_identities, write_content_batch};
use litradar_index::transforms::CATALOG_CSV_V3_COLUMNS;
use litradar_sources::cfp::{
    CfpAdapter, CfpDocument, CfpSourceConfig, CfpSourceError, CfpTransport, CfpUrlRule,
};
use litradar_storage::business::cfp::import_cfp_seed;
use litradar_storage::StorageConfig;
use serde_json::json;

const FIXTURE_MARKER_FILE: &str = ".litradar-e2e-root";
const FIXTURE_MARKER_CONTENT: &str = "litradar-full-stack-e2e-v1\n";
const FIXTURE_DATABASE_NAME: &str = "full-stack.sqlite";
const FIXTURE_ADMIN_USERNAME: &str = "fullstack_admin";
const FIXTURE_ADMIN_PASSWORD: &str = "FullStackAdmin!2026";
const FIXTURE_MEMBER_USERNAME: &str = "fullstack_member";
const FIXTURE_MEMBER_PASSWORD: &str = "FullStackMember!2026";
const FIXTURE_ARTICLE_TITLE: &str = "Evidence Graphs for Living Literature Reviews";
const FIXTURE_ARTICLE_DOI: &str = "10.5555/litradar.fullstack";
const FIXTURE_CFP_URL: &str = "http://cfp-fixture.example/calls";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("full-stack fixture seeding failed: {error}");
        std::process::exit(1);
    }
}

fn run(mut args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let project_root = extract_path_option(&mut args, "--project-root")?
        .ok_or_else(|| invalid_fixture("--project-root is required"))?;
    let should_refresh_cfp = args
        .iter()
        .position(|argument| argument == "--refresh-cfp")
        .map(|index| args.remove(index))
        .is_some();
    if !args.is_empty() {
        return Err(
            invalid_fixture(&format!("unexpected fixture arguments: {}", args.join(" "))).into(),
        );
    }
    let report = if should_refresh_cfp {
        refresh_cfp_fixture(&project_root)?
    } else {
        seed_fixture(&project_root)?
    };
    println!("{}", serde_json::to_string(&report)?);
    Ok(())
}

fn extract_path_option(
    args: &mut Vec<String>,
    name: &str,
) -> Result<Option<PathBuf>, Box<dyn Error>> {
    let Some(index) = args.iter().position(|argument| argument == name) else {
        return Ok(None);
    };
    if index + 1 >= args.len() {
        return Err(invalid_fixture(&format!("{name} requires a path")).into());
    }
    let value = PathBuf::from(args.remove(index + 1));
    args.remove(index);
    Ok(Some(value))
}

fn seed_fixture(project_root: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let project_root = validate_fixture_root(project_root)?;
    let storage = StorageConfig::from_project_root(&project_root);
    if project_root.join("data").exists() {
        return Err(invalid_fixture("fixture data already exists").into());
    }

    litradar_storage::migrate_storage(&storage)?;
    let auth_service = AuthService::new(storage.auth_db_path());
    let administrator =
        auth_service.bootstrap_admin(FIXTURE_ADMIN_USERNAME, FIXTURE_ADMIN_PASSWORD)?;
    let member_invite = auth_service.create_invite_code(administrator.id)?;
    let member = auth_service.register(
        FIXTURE_MEMBER_USERNAME,
        FIXTURE_MEMBER_PASSWORD,
        Some(&member_invite.code),
    )?;
    litradar_storage::create_folder(storage.auth_db_path(), member.id, "Reading", false)?;
    litradar_storage::create_announcement(
        storage.auth_db_path(),
        "Seeded full-stack notice",
        "This announcement proves the real auth database is visible to the frontend.",
        "normal",
        true,
    )?;

    fs::create_dir_all(storage.index_dir())?;
    let content_path = storage.index_dir().join(FIXTURE_DATABASE_NAME);
    let connection = open_content_db(&content_path)?;
    let catalog = fixture_catalog();
    seed_cfp_fixture(&storage, &catalog)?;
    reconcile_catalog_identities(&connection, std::slice::from_ref(&catalog))?;
    let outcome = write_content_batch(
        &connection,
        &catalog,
        &fixture_batch(),
        "full-stack-seed-v1",
        "2026-07-22T00:00:00Z",
    )?;
    let article_id: i64 = connection.query_row(
        "SELECT article_id FROM articles WHERE doi = ?1",
        [FIXTURE_ARTICLE_DOI],
        |row| row.get(0),
    )?;
    let authors_json = serde_json::to_string(&["Ada Lovelace", "Grace Hopper"])?;
    connection.execute(
        "UPDATE articles SET authors_json = ?1 WHERE article_id = ?2",
        (&authors_json, article_id),
    )?;
    drop(connection);

    let push_state_dir = project_root.join("data").join("push_state");
    fs::create_dir_all(&push_state_dir)?;
    fs::write(
        push_state_dir.join("full-stack.changes.json"),
        serde_json::to_vec_pretty(&json!({
            "db_name": FIXTURE_DATABASE_NAME,
            "generated_at": current_epoch_seconds_text()?,
            "run_id": "full-stack-seed-v1",
            "notifiable_article_ids": [article_id]
        }))?,
    )?;

    Ok(json!({
        "status": "seeded",
        "database": FIXTURE_DATABASE_NAME,
        "user_count": 2,
        "article_count": outcome.articles_changed,
        "weekly_article_count": 1
    }))
}

/// Seed one metadata-only CFP catalog and immutable original announcement.
fn seed_cfp_fixture(
    storage: &StorageConfig,
    catalog: &JournalCatalogEntry,
) -> Result<(), Box<dyn Error>> {
    fs::create_dir_all(storage.meta_dir())?;
    let mut row = vec![String::new(); 16];
    row[0] = catalog.catalog_id.clone();
    row[2] = catalog.title.clone();
    row[3] = catalog.issn.clone().unwrap_or_default();
    row[5] = catalog.all_issns.join(";");
    row[7] = catalog.area.clone().unwrap_or_default();
    fs::write(
        storage.meta_dir().join("full-stack.csv"),
        format!("{}\n{}\n", CATALOG_CSV_V3_COLUMNS.join(","), row.join(",")),
    )?;
    let source = serde_json::from_value(
        json!({"catalogIds":[catalog.catalog_id],"journalTitle":catalog.title,"title":"Initial original CFP","scope":"Original research on reproducible evidence synthesis.","requirements":"Original manuscripts are welcome.","typeText":"Special Issue","dateText":"Submission deadline: 30 November 2099","sourceUrl":FIXTURE_CFP_URL,"checkedOn":"2026-09-15"}),
    )?;
    let seed = CfpSeed {
        format_version: 1,
        sources: vec![source],
        empty_journals: Vec::new(),
        expected_journals: Some(1),
        expected_notices: Some(1),
    };
    import_cfp_seed(
        storage.auth_db_path(),
        "full-stack-cfp-v1",
        &serde_json::to_vec(&seed)?,
    )?;
    Ok(())
}

struct FixtureCfpHttpTransport {
    client: reqwest::blocking::Client,
}

impl CfpTransport for FixtureCfpHttpTransport {
    fn fetch(
        &self,
        config: &CfpSourceConfig,
        url: &str,
        deadline: Instant,
    ) -> Result<CfpDocument, CfpSourceError> {
        if url != FIXTURE_CFP_URL
            || !config
                .permits_url(&reqwest::Url::parse(url).map_err(|_| CfpSourceError::DisallowedUrl)?)
        {
            return Err(CfpSourceError::DisallowedUrl);
        }
        let response = self
            .client
            .get(url)
            .timeout(deadline.saturating_duration_since(Instant::now()))
            .send()
            .map_err(|_| CfpSourceError::Request)?;
        if !response.status().is_success() {
            return Err(CfpSourceError::HttpStatus(response.status().as_u16()));
        }
        let final_url = response.url().to_string();
        let mut text = String::new();
        response
            .take(65537)
            .read_to_string(&mut text)
            .map_err(|_| CfpSourceError::Encoding)?;
        if text.len() > 65536 {
            return Err(CfpSourceError::TooLarge);
        }
        Ok(CfpDocument {
            final_url,
            text,
            format: "html".into(),
        })
    }
}

/// Acquire a changed original over an isolated loopback HTTP proxy and publish it atomically.
fn refresh_cfp_fixture(project_root: &Path) -> Result<serde_json::Value, Box<dyn Error>> {
    let project_root = validate_fixture_root(project_root)?;
    let storage = StorageConfig::from_project_root(project_root);
    let catalog = fixture_catalog();
    let listener = TcpListener::bind("127.0.0.1:0")?;
    listener.set_nonblocking(true)?;
    let proxy_address = listener.local_addr()?;
    let server = std::thread::spawn(move || -> io::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error)
                    if error.kind() == io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10))
                }
                Err(error) => return Err(error),
            }
        };
        stream.set_read_timeout(Some(Duration::from_secs(2)))?;
        stream.set_write_timeout(Some(Duration::from_secs(2)))?;
        let mut request = [0_u8; 4096];
        let length = stream.read(&mut request)?;
        if !String::from_utf8_lossy(&request[..length])
            .starts_with("GET http://cfp-fixture.example/calls ")
        {
            return Err(invalid_fixture("unexpected CFP proxy request"));
        }
        let body="<h1>Journal of Reproducible Literature</h1><h2>Call for papers</h2><h3>Updated original CFP after backend refresh</h3><p>New original research scope from the HTTP source.</p><p>Submission deadline: 31 December 2099</p><h4>Submission instructions</h4><p>Original manuscripts are welcome.</p>";
        write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body)?;
        Ok(())
    });
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::http(format!("http://{proxy_address}"))?)
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .build()?;
    let config = CfpSourceConfig {
        source_key: format!("journal:{}", catalog.catalog_id),
        catalog_ids: vec![catalog.catalog_id],
        journal_title: catalog.title.clone(),
        discovery_url: FIXTURE_CFP_URL.into(),
        adapter: CfpAdapter::ElsevierCalls,
        config_version: 1,
        allowed_urls: vec![CfpUrlRule {
            host: "cfp-fixture.example".into(),
            path_prefix: "/calls".into(),
        }],
        identity_texts: vec![catalog.title],
        empty_statements: Vec::new(),
        capability_note: None,
        retains_previous_notices: false,
    };
    let result = litradar_worker::cfp::refresh_cfp_source(
        storage.auth_db_path(),
        &config,
        &FixtureCfpHttpTransport { client },
        Instant::now() + Duration::from_secs(5),
    );
    server
        .join()
        .map_err(|_| invalid_fixture("CFP source fixture thread failed"))??;
    let result = result?;
    if result.status != "success" {
        return Err(invalid_fixture("CFP source fixture did not publish").into());
    }
    Ok(json!({"status":"cfp_updated","notices":result.notices}))
}

fn current_epoch_seconds_text() -> Result<String, std::time::SystemTimeError> {
    Ok(SystemTime::now()
        .duration_since(UNIX_EPOCH)?
        .as_secs()
        .to_string())
}

fn validate_fixture_root(project_root: &Path) -> Result<PathBuf, Box<dyn Error>> {
    let root_metadata = fs::symlink_metadata(project_root)?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(invalid_fixture("fixture root must be a real directory").into());
    }
    let canonical_root = fs::canonicalize(project_root)?;
    let canonical_temp = fs::canonicalize(std::env::temp_dir())?;
    if canonical_root == canonical_temp || !canonical_root.starts_with(&canonical_temp) {
        return Err(
            invalid_fixture("fixture root must be below the OS temporary directory").into(),
        );
    }
    let marker_path = canonical_root.join(FIXTURE_MARKER_FILE);
    let marker_metadata = fs::symlink_metadata(&marker_path)
        .map_err(|_| invalid_fixture("fixture marker is missing"))?;
    if !marker_metadata.is_file() || marker_metadata.file_type().is_symlink() {
        return Err(invalid_fixture("fixture marker must be a regular file").into());
    }
    if fs::read_to_string(marker_path)? != FIXTURE_MARKER_CONTENT {
        return Err(invalid_fixture("fixture marker content is invalid").into());
    }
    Ok(canonical_root)
}

fn fixture_catalog() -> JournalCatalogEntry {
    JournalCatalogEntry {
        catalog_id: "full-stack-journal".to_string(),
        catalog_aliases: Vec::new(),
        title: "Journal of Reproducible Literature".to_string(),
        issn: Some("1234-5679".to_string()),
        eissn: None,
        all_issns: vec!["1234-5679".to_string()],
        title_aliases: Vec::new(),
        area: Some("Information Science".to_string()),
        rankings: JournalRankings::default(),
    }
}

fn fixture_batch() -> ProviderBatch {
    ProviderBatch {
        catalog_id: "full-stack-journal".to_string(),
        journal: JournalDraft {
            catalog_id: "full-stack-journal".to_string(),
            observed_title: Some("Journal of Reproducible Literature".to_string()),
            observed_issns: vec!["1234-5679".to_string()],
            observed_title_aliases: Vec::new(),
        },
        issues: vec![IssueDraft {
            catalog_id: "full-stack-journal".to_string(),
            publication_year: Some(2026),
            title: Some("Full-stack verification issue".to_string()),
            volume: Some("12".to_string()),
            number: Some("3".to_string()),
            date: Some("2026-07".to_string()),
        }],
        articles: vec![ArticleDraft {
            catalog_id: "full-stack-journal".to_string(),
            title: FIXTURE_ARTICLE_TITLE.to_string(),
            publication_year: Some(2026),
            date: Some("2026-07-21".to_string()),
            issue_title: Some("Full-stack verification issue".to_string()),
            volume: Some("12".to_string()),
            issue_number: Some("3".to_string()),
            authors: vec![
                ArticleAuthorDraft {
                    display_name: "Ada Lovelace".to_string(),
                },
                ArticleAuthorDraft {
                    display_name: "Grace Hopper".to_string(),
                },
            ],
            start_page: Some("101".to_string()),
            end_page: Some("118".to_string()),
            abstract_text: Some(
                "A deterministic nonempty article used to verify SQLite, search, detail, weekly, and favorite persistence."
                    .to_string(),
            ),
            doi: Some(FIXTURE_ARTICLE_DOI.to_string()),
            pmid: None,
            open_access: Some(true),
            in_press: Some(false),
            retraction_dois: Vec::new(),
        }],
        progress: ProviderProgress::Complete { next_anchor: None },
    }
}

fn invalid_fixture(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{seed_fixture, FIXTURE_ARTICLE_TITLE, FIXTURE_MARKER_CONTENT, FIXTURE_MARKER_FILE};

    #[test]
    fn unmarked_existing_root_is_rejected_without_changes() {
        let root = tempdir().expect("temporary fixture root should create");
        let sentinel = root.path().join("operator-data.txt");
        fs::write(&sentinel, "preserve").expect("sentinel should write");

        let error = seed_fixture(root.path()).expect_err("unmarked root should be rejected");

        assert!(error.to_string().contains("marker is missing"));
        assert_eq!(
            fs::read_to_string(sentinel).expect("sentinel should remain"),
            "preserve"
        );
        assert!(!root.path().join("data").exists());
    }

    #[test]
    fn marked_temporary_root_receives_complete_real_backend_state() {
        let root = tempdir().expect("temporary fixture root should create");
        fs::write(
            root.path().join(FIXTURE_MARKER_FILE),
            FIXTURE_MARKER_CONTENT,
        )
        .expect("fixture marker should write");

        let report = seed_fixture(root.path()).expect("marked root should seed");
        let storage = litradar_storage::StorageConfig::from_project_root(root.path());
        let users = litradar_storage::list_all_users(storage.auth_db_path())
            .expect("seeded users should load");
        let weekly = litradar_storage::get_weekly_updates(&storage)
            .expect("seeded weekly updates should load");

        assert_eq!(report["status"], "seeded");
        assert_eq!(report["article_count"], 1);
        assert_eq!(users.len(), 2);
        assert_eq!(weekly.databases.len(), 1);
        assert_eq!(weekly.databases[0].new_article_count, 1);
        assert_eq!(
            weekly.databases[0].journals[0].articles[0].title,
            FIXTURE_ARTICLE_TITLE
        );
    }
}
