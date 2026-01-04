//! Unified locate command tests (US-3.x)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

// ============================================================================
// US-3.1: Unified Locate Command Tests
// ============================================================================

#[test]
fn test_locate_global_query() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "status:todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[en]"))
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task1"))
        .stdout(predicate::str::contains("task2"));
}

#[test]
fn test_locate_scoped_query_flag() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "status:todo", "--in", "private"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task2"))
        .stdout(predicate::str::contains("[en]").not())
        .stdout(predicate::str::contains("task1").not());
}

#[test]
fn test_locate_scoped_query_positional() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "private", "status:todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task2"))
        .stdout(predicate::str::contains("[en]").not())
        .stdout(predicate::str::contains("task1").not());
}

#[test]
fn test_locate_nonexistent_context_errors() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "nonexistent", "status:todo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Space 'nonexistent' not found"));
}

#[test]
fn test_locate_multiple_contexts() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    let company_path = tmp.path().join("company");
    std::fs::create_dir_all(company_path.join("tasks")).unwrap();
    std::fs::write(
        company_path.join("tasks/task3.md"),
        "---\ntype: task\nid: task3\nstatus: todo\n---\n\n# Task 3\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "status:todo", "--in", "en,private"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[en]"))
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task1"))
        .stdout(predicate::str::contains("task2"))
        .stdout(predicate::str::contains("[company]").not())
        .stdout(predicate::str::contains("task3").not());
}

#[test]
fn test_locate_type_filter() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::create_dir_all(en_path.join("definitions")).unwrap();

    std::fs::write(
        en_path.join("tasks/my-task.md"),
        "---\ntype: task\nid: my-task\nstatus: todo\n---\n\n# My Task\n",
    )
    .unwrap();

    std::fs::write(
        en_path.join("definitions/hello.md"),
        "---\ntype: definition\nid: hello\n---\n\nA greeting.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "type:task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-task"))
        .stdout(predicate::str::contains("hello").not());

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "type:definition"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains("my-task").not());
}

#[test]
fn test_locate_fulltext_search() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();

    std::fs::write(
        en_path.join("tasks/meeting.md"),
        "---\ntype: task\nid: meeting\nstatus: todo\n---\n\n# Meeting Task\n\nWe have an important meeting tomorrow.\n",
    )
    .unwrap();

    std::fs::write(
        en_path.join("tasks/review.md"),
        "---\ntype: task\nid: review\nstatus: todo\n---\n\n# Code Review\n\nReview the pull request.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "important meeting"])
        .assert()
        .success()
        .stdout(predicate::str::contains("meeting"))
        .stdout(predicate::str::contains("review").not());
}

#[test]
fn test_locate_combined_filter_and_search() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();

    std::fs::write(
        en_path.join("tasks/meeting.md"),
        "---\ntype: task\nid: meeting\nstatus: todo\n---\n\n# Meeting Task\n\nWe have an important meeting tomorrow.\n",
    )
    .unwrap();

    std::fs::write(
        en_path.join("tasks/done-meeting.md"),
        "---\ntype: task\nid: done-meeting\nstatus: done\n---\n\n# Done Meeting\n\nAn important meeting that is done.\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "status:todo", "important"])
        .assert()
        .success()
        .stdout(predicate::str::contains("meeting"))
        .stdout(predicate::str::contains("done-meeting").not());
}

// ============================================================================
// US-3.2: Index Performance Tests
// ============================================================================

#[test]
fn test_locate_build_creates_index() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\nlanguage: english\n---\n\n# Task 1\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "build"])
        .assert()
        .success()
        .stdout(predicate::str::contains("Index built"));

    let index_path = tmp.path().join(".co/index.bin");
    assert!(index_path.exists());
}

#[test]
fn test_locate_build_indexes_all_spaces() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: done\nlanguage: portuguese\n---\n\n# Task 2\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "build"])
        .assert()
        .success()
        .stdout(predicate::str::contains("2 entries"));
}

#[test]
fn test_locate_update_only_modified() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "build"])
        .assert()
        .success();

    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: done\n---\n\n# Task 1 Updated\n",
    )
    .unwrap();

    std::fs::write(
        en_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "update"])
        .assert()
        .success()
        .stdout(predicate::str::contains("updated"))
        .stdout(predicate::str::contains("added"));
}

#[test]
fn test_locate_stats_shows_breakdown() {
    let tmp = tempdir().unwrap();

    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\nlanguage: english\n---\n\n# Task 1\n",
    )
    .unwrap();

    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: done\nlanguage: portuguese\n---\n\n# Task 2\n",
    )
    .unwrap();
    std::fs::write(
        private_path.join("tasks/task3.md"),
        "---\ntype: task\nid: task3\nstatus: todo\n---\n\n# Task 3\n",
    )
    .unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "build"])
        .assert()
        .success();

    co_command()
        .current_dir(tmp.path())
        .args(["locate", "stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("3"))
        .stdout(predicate::str::contains("en"))
        .stdout(predicate::str::contains("private"));
}
