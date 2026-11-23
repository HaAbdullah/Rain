//! `rain.db`: opened at start, closed with a checkpoint at the end, and replaced when it is
//! damaged. Foundation section 10.5.

mod common;

use common::TestBackend;
use rain_types::Health;

/// The local folder of a backend holds the database: `<local>\rain.db`.
fn database_file(backend: &TestBackend) -> std::path::PathBuf {
    backend.local_dir.join("rain.db")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_database_is_made_in_the_local_folder_with_the_list_of_migrations() {
    let backend = TestBackend::start().await;
    assert!(database_file(&backend).is_file());

    let tables: Vec<String> = backend
        .state
        .db
        .call(|c| {
            let mut statement = c.prepare("SELECT name FROM sqlite_master WHERE type = 'table'")?;
            statement.query_map([], |row| row.get(0))?.collect()
        })
        .await
        .unwrap();
    assert!(tables.contains(&"core_migrations".to_owned()), "{tables:?}");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn what_a_module_stores_survives_a_restart_and_a_stop_leaves_one_clean_file() {
    let mut backend = TestBackend::start().await;
    backend
        .state
        .db
        .call(|c| {
            c.execute_batch(
                "CREATE TABLE test_history (at TEXT); INSERT INTO test_history VALUES ('kept')",
            )
        })
        .await
        .unwrap();

    backend.restart().await;

    let kept: String = backend
        .state
        .db
        .call(|c| c.query_row("SELECT at FROM test_history", [], |row| row.get(0)))
        .await
        .unwrap();
    assert_eq!(kept, "kept");
    let health: Health = backend.get("/health").await;
    assert!(health.config_issues.is_empty());

    let wal = backend.local_dir.join("rain.db-wal");
    backend.stop().await;
    assert!(
        !wal.exists() || std::fs::metadata(&wal).unwrap().len() == 0,
        "the checkpoint on stop left no log behind"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_damaged_database_is_moved_aside_and_reported_and_the_backend_starts() {
    let backend = TestBackend::builder()
        .local_file("rain.db", &[0x42; 4096])
        .start()
        .await;

    let health: Health = backend.get("/health").await;
    let issue = health
        .config_issues
        .iter()
        .find(|issue| issue.file == "rain.db")
        .expect("the damage is reported");
    assert!(
        issue.message.contains("rain.db.corrupt-"),
        "{}",
        issue.message
    );

    let kept: Vec<_> = std::fs::read_dir(&backend.local_dir)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("rain.db.corrupt-")
        })
        .collect();
    assert_eq!(kept.len(), 1, "the damaged file is kept for the user");
    assert_eq!(std::fs::read(kept[0].path()).unwrap(), vec![0x42; 4096]);
    backend
        .state
        .db
        .call(|_| Ok(()))
        .await
        .expect("a new database works");
    backend.stop().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_folder_that_cannot_hold_a_database_means_history_in_memory_and_an_issue() {
    // The local folder is a file, so nothing can be created in it.
    let temp = tempfile::tempdir().unwrap();
    let blocker = temp.path().join("not-a-folder");
    std::fs::write(&blocker, "x").unwrap();
    let blocker_for_config = blocker.clone();
    let backend = TestBackend::builder()
        .config(move |config| config.local_dir = blocker_for_config.clone())
        .start()
        .await;

    let health: Health = backend.get("/health").await;
    let issue = health
        .config_issues
        .iter()
        .find(|issue| issue.file == "rain.db")
        .expect("the problem is reported");
    assert!(issue.message.contains("memory"), "{}", issue.message);
    backend
        .state
        .db
        .call(|_| Ok(()))
        .await
        .expect("memory works");
    backend.stop().await;
}
