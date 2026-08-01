//! Process-boundary cleanup for SQLite WAL sidecars.

use std::io;
use std::path::{Path, PathBuf};

use litradar_storage::{cleanup_sqlite_sidecars, SqliteSidecarCleanup, StorageConfig};
use litradar_worker::scheduler::INTERNAL_PARENT_RUN_ID_ARGUMENT;

const LIVE_INDEX_WORKER_REQUEST_ARGUMENT: &str = "--live-worker-request";

/// Safely converge idle SQLite sidecars after a top-level command or service stops.
///
/// # Arguments
///
/// * `args` - Original process arguments without the executable name.
pub(crate) fn cleanup_after_process(args: &[String]) {
    if is_internal_child(args) {
        return;
    }
    let paths = match database_paths(args) {
        Ok(paths) => paths,
        Err(_) => {
            tracing::warn!(
                event = "storage.sqlite_sidecars.cleanup_failed",
                component = "storage",
                error_kind = "path_discovery",
            );
            return;
        }
    };

    for path in paths {
        let database = database_label(&path);
        match cleanup_sqlite_sidecars(&path) {
            Ok(SqliteSidecarCleanup::NotPresent) => {}
            Ok(SqliteSidecarCleanup::Cleaned) => tracing::debug!(
                event = "storage.sqlite_sidecars.cleaned",
                component = "storage",
                database,
                outcome = "success",
            ),
            Ok(SqliteSidecarCleanup::Busy) => tracing::debug!(
                event = "storage.sqlite_sidecars.retained",
                component = "storage",
                database,
                outcome = "skipped",
                reason = "active_connection",
            ),
            Ok(SqliteSidecarCleanup::Retained) => tracing::debug!(
                event = "storage.sqlite_sidecars.retained",
                component = "storage",
                database,
                outcome = "skipped",
                reason = "sqlite_retained",
            ),
            Err(_) => tracing::warn!(
                event = "storage.sqlite_sidecars.cleanup_failed",
                component = "storage",
                database,
                error_kind = "sqlite_error",
            ),
        }
    }
}

fn is_internal_child(args: &[String]) -> bool {
    args.iter().any(|argument| {
        matches!(
            argument.as_str(),
            INTERNAL_PARENT_RUN_ID_ARGUMENT | LIVE_INDEX_WORKER_REQUEST_ARGUMENT
        )
    })
}

fn database_paths(args: &[String]) -> io::Result<Vec<PathBuf>> {
    let current_dir = std::env::current_dir()?;
    let project_root = option_value(args, "--project-root")?
        .map(PathBuf::from)
        .unwrap_or(current_dir);
    let storage = StorageConfig::from_project_root(project_root);
    let auth_db_path = option_value(args, "--auth-db")?
        .map(PathBuf::from)
        .unwrap_or_else(|| storage.auth_db_path().to_path_buf());
    let mut paths = vec![auth_db_path];
    append_sqlite_files(storage.index_dir(), &mut paths)?;
    append_sqlite_files(storage.index_control_dir(), &mut paths)?;
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn option_value<'arguments>(
    args: &'arguments [String],
    option: &str,
) -> io::Result<Option<&'arguments str>> {
    let Some(index) = args.iter().position(|argument| argument == option) else {
        return Ok(None);
    };
    args.get(index + 1)
        .map(String::as_str)
        .map(Some)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "missing option value"))
}

fn append_sqlite_files(directory: &Path, paths: &mut Vec<PathBuf>) -> io::Result<()> {
    if !directory.exists() {
        return Ok(());
    }
    for entry in std::fs::read_dir(directory)? {
        let entry = entry?;
        if entry.file_type()?.is_file()
            && entry.path().extension().and_then(|value| value.to_str()) == Some("sqlite")
        {
            paths.push(entry.path());
        }
    }
    Ok(())
}

fn database_label(path: &Path) -> &str {
    path.file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("sqlite")
}

#[cfg(test)]
mod tests {
    use std::fs::File;

    use tempfile::tempdir;

    use super::{database_paths, is_internal_child, option_value};

    #[test]
    fn discovers_auth_content_and_control_databases() {
        let directory = tempdir().expect("temporary directory should be created");
        let data_dir = directory.path().join("data");
        let index_dir = data_dir.join("index");
        let control_dir = data_dir.join("index-control");
        std::fs::create_dir_all(&index_dir).expect("index directory should be created");
        std::fs::create_dir_all(&control_dir).expect("control directory should be created");
        let auth = data_dir.join("auth.sqlite");
        let content = index_dir.join("english.sqlite");
        let control = control_dir.join("english.sqlite");
        File::create(&auth).expect("auth database should be created");
        File::create(&content).expect("content database should be created");
        File::create(&control).expect("control database should be created");
        File::create(index_dir.join("ignored.txt")).expect("other file should be created");

        let paths = database_paths(&[
            "index".to_string(),
            "--project-root".to_string(),
            directory.path().to_string_lossy().into_owned(),
        ])
        .expect("database paths should be discovered");

        assert_eq!(paths, vec![auth, content, control]);
    }

    #[test]
    fn internal_children_defer_cleanup_to_their_parent() {
        assert!(is_internal_child(&[
            "index".to_string(),
            "--live-worker-request".to_string(),
            "request.json".to_string(),
        ]));
        assert!(is_internal_child(&[
            "delivery-run".to_string(),
            "--litradar-parent-run-id".to_string(),
            "parent".to_string(),
        ]));
        assert!(!is_internal_child(&["index".to_string()]));
    }

    #[test]
    fn missing_path_option_value_prevents_broad_cleanup() {
        assert!(option_value(&["--project-root".to_string()], "--project-root").is_err());
        assert!(database_paths(&["--auth-db".to_string()]).is_err());
    }
}
