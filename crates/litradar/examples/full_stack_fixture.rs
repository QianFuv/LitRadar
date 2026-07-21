//! Marker-guarded deterministic data seeder for real-backend browser tests.

use std::error::Error;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use litradar_auth::AuthService;
use litradar_domain::{
    ArticleAuthorDraft, ArticleDraft, IssueDraft, JournalCatalogEntry, JournalDraft,
    JournalRankings, ProviderBatch,
};
use litradar_index::schema::{open_content_db, reconcile_catalog_identities, write_content_batch};
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

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("full-stack fixture seeding failed: {error}");
        std::process::exit(1);
    }
}

fn run(mut args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let project_root = extract_path_option(&mut args, "--project-root")?
        .ok_or_else(|| invalid_fixture("--project-root is required"))?;
    if !args.is_empty() {
        return Err(
            invalid_fixture(&format!("unexpected fixture arguments: {}", args.join(" "))).into(),
        );
    }
    let report = seed_fixture(&project_root)?;
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
            "generated_at": "2026-07-22T00:00:00Z",
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
        is_complete: true,
        next_checkpoint: None,
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
