//! Integration tests for invite-only registration and local administrator bootstrap.

use std::sync::{Arc, Barrier};
use std::thread;

use litradar_storage::{
    bootstrap_admin, count_users, migrate_auth_database, register_user_with_invite,
    AuthRepositoryError,
};
use tempfile::tempdir;

#[test]
fn auth_registration_cannot_create_first_user_with_or_without_invite() {
    let temp_dir = tempdir().expect("temporary directory should be created");
    let auth_db_path = temp_dir.path().join("auth.sqlite");
    migrate_auth_database(&auth_db_path).expect("auth database should migrate");

    for invite_code in [None, Some("fabricated-invite")] {
        let error = register_user_with_invite(
            &auth_db_path,
            "remote-user",
            "password-hash",
            "salt",
            invite_code,
            1.0,
        )
        .expect_err("public registration should fail on an empty database");

        assert!(matches!(
            error,
            AuthRepositoryError::AdministratorBootstrapRequired
        ));
    }
    assert_eq!(
        count_users(&auth_db_path).expect("user count should load"),
        0
    );
}

#[test]
fn auth_bootstrap_allows_exactly_one_concurrent_administrator() {
    let temp_dir = tempdir().expect("temporary directory should be created");
    let auth_db_path = temp_dir.path().join("auth.sqlite");
    migrate_auth_database(&auth_db_path).expect("auth database should migrate");
    let barrier = Arc::new(Barrier::new(3));

    let handles = ["first-admin", "second-admin"].map(|username| {
        let auth_db_path = auth_db_path.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            barrier.wait();
            bootstrap_admin(auth_db_path, username, "password-hash", "salt", 2.0)
        })
    });
    barrier.wait();
    let results = handles.map(|handle| handle.join().expect("bootstrap thread should finish"));

    let created = results
        .iter()
        .filter_map(|result| result.as_ref().ok())
        .collect::<Vec<_>>();
    let refused = results
        .iter()
        .filter_map(|result| result.as_ref().err())
        .collect::<Vec<_>>();
    assert_eq!(created.len(), 1);
    assert!(created[0].is_admin);
    assert_eq!(refused.len(), 1);
    assert!(matches!(
        refused[0],
        AuthRepositoryError::AdministratorBootstrapAlreadyCompleted
    ));
    assert_eq!(
        count_users(&auth_db_path).expect("user count should load"),
        1
    );
}
