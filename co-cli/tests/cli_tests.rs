//! CLI integration tests

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::tempdir;

fn co_command() -> Command {
    Command::cargo_bin("co").unwrap()
}

#[test]
fn test_help() {
    co_command()
        .arg("--help")
        .assert()
        .success()
        .stdout(predicate::str::contains("co"));
}

#[test]
fn test_version() {
    co_command().arg("--version").assert().success();
}

#[test]
fn test_status() {
    co_command()
        .arg("status")
        .assert()
        .success()
        .stdout(predicate::str::contains("CO Graph Status"));
}

#[test]
fn test_languages_alias_works() {
    // `languages` is an alias for `lang --list`
    // Both should show the same language list
    co_command()
        .args(["languages", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("english"))
        .stdout(predicate::str::contains("portuguese"))
        .stdout(predicate::str::contains("math"));

    // `lang --list` should produce the same output
    co_command()
        .args(["lang", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("english"))
        .stdout(predicate::str::contains("portuguese"))
        .stdout(predicate::str::contains("math"));
}

#[test]
fn test_config_show() {
    co_command().args(["config", "show"]).assert().success();
}

// ============================================================================
// US-1.2: Context Isolation Tests
// ============================================================================

#[test]
fn test_init_creates_context_directory() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Verify context directory created (no fixed subdirs - structure is flexible)
    assert!(tmp.path().join("private").exists());
    assert!(tmp.path().join("private/README.md").exists());
}

#[test]
fn test_init_prevents_duplicate_context() {
    let tmp = tempdir().unwrap();

    // Create the context directory first
    std::fs::create_dir(tmp.path().join("private")).unwrap();

    // Attempt to init should fail
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn test_init_creates_context_readme() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Verify README.md exists with correct frontmatter
    let readme_path = tmp.path().join("private/README.md");
    assert!(readme_path.exists(), "README.md should exist");

    let readme = std::fs::read_to_string(&readme_path).unwrap();
    assert!(readme.contains("type: context"), "README should contain type: context");
    assert!(readme.contains("id: private"), "README should contain id: private");
    assert!(readme.contains("language: english"), "README should contain language: english");
}

#[test]
fn test_list_shows_contexts_and_languages() {
    let tmp = tempdir().unwrap();

    // Create a language directory (en) with README marking it as language
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(&en_path).unwrap();
    std::fs::write(
        en_path.join("README.md"),
        "---\ntype: language\nid: en\n---\n# English\n",
    )
    .unwrap();

    // Create a context directory (private) using init
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // List should show both
    co_command()
        .current_dir(tmp.path())
        .arg("list")
        .assert()
        .success()
        .stdout(predicate::str::contains("en"))
        .stdout(predicate::str::contains("language"))
        .stdout(predicate::str::contains("private"))
        .stdout(predicate::str::contains("context"));
}

#[test]
fn test_list_stats_shows_file_counts() {
    let tmp = tempdir().unwrap();

    // Create a scope with some files
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Add some definition files
    let scope_path = tmp.path().join("private");
    std::fs::write(
        scope_path.join("word1.md"),
        "---\ntype: definition\n---\nA definition.",
    )
    .unwrap();
    std::fs::write(
        scope_path.join("word2.md"),
        "---\ntype: definition\n---\nAnother definition.",
    )
    .unwrap();

    // List with stats should show file counts
    co_command()
        .current_dir(tmp.path())
        .args(["list", "--stats"])
        .assert()
        .success()
        .stdout(predicate::str::contains("private"))
        .stdout(predicate::str::contains("files"));
}

// ============================================================================
// US-2.1: Create Content Tests
// ============================================================================

#[test]
fn test_new_creates_file_in_scope() {
    let tmp = tempdir().unwrap();

    // Create scope first
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create a new task in that scope
    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "my-task", "--in", "private"])
        .assert()
        .success();

    // Verify file was created
    let task_path = tmp.path().join("private/tasks/my-task.md");
    assert!(task_path.exists(), "Task file should be created at private/tasks/my-task.md");
}

#[test]
fn test_new_auto_populates_frontmatter() {
    let tmp = tempdir().unwrap();

    // Create scope first
    co_command()
        .current_dir(tmp.path())
        .args(["init", "private"])
        .assert()
        .success();

    // Create a new task
    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "my-task", "--in", "private"])
        .assert()
        .success();

    // Read the created file and verify frontmatter
    let task_path = tmp.path().join("private/tasks/my-task.md");
    let content = std::fs::read_to_string(&task_path).unwrap();

    assert!(content.contains("schema_version: 2"), "Should have schema_version: 2");
    assert!(content.contains("language: en"), "Should have language: en");
    assert!(content.contains("scope: private"), "Should have scope: private");
    assert!(content.contains("type: task"), "Should have type: task");
    assert!(content.contains("status: todo"), "Should have status: todo");
}

#[test]
fn test_new_defaults_to_en_scope() {
    let tmp = tempdir().unwrap();

    // Create en/ language directory (simulating existing language)
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(&en_path).unwrap();
    std::fs::write(
        en_path.join("README.md"),
        "---\ntype: language\nid: en\n---\n# English\n",
    )
    .unwrap();

    // Create a new task WITHOUT --in flag
    co_command()
        .current_dir(tmp.path())
        .args(["new", "task", "foo"])
        .assert()
        .success();

    // Verify file was created in en/tasks/
    let task_path = tmp.path().join("en/tasks/foo.md");
    assert!(task_path.exists(), "Task file should be created at en/tasks/foo.md");

    // Verify scope is set to en
    let content = std::fs::read_to_string(&task_path).unwrap();
    assert!(content.contains("scope: en"), "Should have scope: en");
}

// ============================================================================
// US-2.2: Read Content Tests
// ============================================================================

#[test]
fn test_show_displays_content() {
    let tmp = tempdir().unwrap();

    // Create en/ with a task file
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n\nTask content here.\n",
    )
    .unwrap();

    // Show the content
    co_command()
        .current_dir(tmp.path())
        .args(["show", "todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("My Task"))
        .stdout(predicate::str::contains("Task content here"));
}

#[test]
fn test_show_meta_only() {
    let tmp = tempdir().unwrap();

    // Create en/ with a task file
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n\nTask content here.\n",
    )
    .unwrap();

    // Show only metadata
    co_command()
        .current_dir(tmp.path())
        .args(["show", "todo", "--meta"])
        .assert()
        .success()
        .stdout(predicate::str::contains("type: task"))
        .stdout(predicate::str::contains("status: todo"))
        .stdout(predicate::str::contains("My Task").not()); // Should NOT contain body content
}

// ============================================================================
// US-2.3: Update Content Tests
// ============================================================================

#[test]
fn test_update_changes_status() {
    let tmp = tempdir().unwrap();

    // Create en/ with a task file
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/todo.md"),
        "---\ntype: task\nid: todo\nstatus: todo\n---\n\n# My Task\n",
    )
    .unwrap();

    // Update the status
    co_command()
        .current_dir(tmp.path())
        .args(["update", "todo", "--status", "done"])
        .assert()
        .success();

    // Verify the file was updated
    let content = std::fs::read_to_string(en_path.join("tasks/todo.md")).unwrap();
    assert!(content.contains("status: done"), "Status should be updated to done");
    assert!(!content.contains("status: todo"), "Old status should be replaced");
}

// ============================================================================
// US-2.4: Delete Content Tests
// ============================================================================

#[test]
fn test_delete_removes_file() {
    let tmp = tempdir().unwrap();

    // Create en/ with a task file
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    let task_path = en_path.join("tasks/old.md");
    std::fs::write(
        &task_path,
        "---\ntype: task\nid: old\nstatus: todo\n---\n\n# Old Task\n",
    )
    .unwrap();

    // Verify file exists before delete
    assert!(task_path.exists(), "File should exist before delete");

    // Delete with confirmation
    co_command()
        .current_dir(tmp.path())
        .args(["delete", "old", "--confirm"])
        .assert()
        .success();

    // Verify file was removed
    assert!(!task_path.exists(), "File should be removed after delete");
}

// ============================================================================
// US-1.3: Multilingual UI Support Tests
// ============================================================================

#[test]
fn test_english_ui_file_parseable() {
    // Verify the English UI translation file can be parsed
    let content = include_str!("../../en/ui/en.yaml");
    let labels: co::UiLabels = serde_yaml::from_str(content).expect("Should parse en.yaml");
    assert_eq!(labels.lang, "en");
    assert!(labels.types.contains_key("definition"));
    assert!(labels.fields.contains_key("status"));
    assert!(labels.messages.contains_key("initialized"));
}

#[test]
fn test_portuguese_ui_file_parseable() {
    // Verify the Portuguese UI translation file can be parsed
    let content = include_str!("../../en/ui/pt.yaml");
    let labels: co::UiLabels = serde_yaml::from_str(content).expect("Should parse pt.yaml");
    assert_eq!(labels.lang, "pt");
    assert_eq!(labels.type_label("definition"), "definição");
    assert_eq!(labels.field_label("status"), "estado");
    assert_eq!(labels.message("initialized"), "Inicializado com sucesso");
}

#[test]
fn test_lang_sets_system_language() {
    let tmp = tempdir().unwrap();

    // Set system language to Portuguese
    co_command()
        .current_dir(tmp.path())
        .args(["lang", "pt"])
        .assert()
        .success();

    // Verify config file was created with the language setting
    let config_path = tmp.path().join(".co/config.yaml");
    assert!(config_path.exists(), "Config file should exist");

    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(
        config.contains("system_language: pt"),
        "Config should contain system_language: pt"
    );
}

// ============================================================================
// US-3.1: Query with Scope Filtering Tests
// ============================================================================

#[test]
fn test_locate_find_global_query() {
    let tmp = tempdir().unwrap();

    // Create en/ context with a task
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    // Create private/ context with another task
    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    // Query all contexts (global query by default)
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "status:todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[en]"))
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task1"))
        .stdout(predicate::str::contains("task2"));
}

#[test]
fn test_locate_find_scoped_query_flag() {
    let tmp = tempdir().unwrap();

    // Create en/ context with a task
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    // Create private/ context with another task
    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    // Query only private context using --in flag
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "status:todo", "--in", "private"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task2"))
        .stdout(predicate::str::contains("[en]").not())
        .stdout(predicate::str::contains("task1").not());
}

#[test]
fn test_locate_find_scoped_query_positional() {
    let tmp = tempdir().unwrap();

    // Create en/ context with a task
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    // Create private/ context with another task
    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    // Query using positional context syntax: `co locate find private status:todo`
    // This should be equivalent to `--in private`
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "private", "status:todo"])
        .assert()
        .success()
        .stdout(predicate::str::contains("[private]"))
        .stdout(predicate::str::contains("task2"))
        .stdout(predicate::str::contains("[en]").not())
        .stdout(predicate::str::contains("task1").not());
}

#[test]
fn test_locate_find_nonexistent_context_errors() {
    let tmp = tempdir().unwrap();

    // Create en/ context only
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();

    // Query with non-existent context should error
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "nonexistent", "status:todo"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("Context 'nonexistent' not found"));
}

#[test]
fn test_locate_find_multiple_contexts() {
    let tmp = tempdir().unwrap();

    // Create en/ context
    let en_path = tmp.path().join("en");
    std::fs::create_dir_all(en_path.join("tasks")).unwrap();
    std::fs::write(
        en_path.join("tasks/task1.md"),
        "---\ntype: task\nid: task1\nstatus: todo\n---\n\n# Task 1\n",
    )
    .unwrap();

    // Create private/ context
    let private_path = tmp.path().join("private");
    std::fs::create_dir_all(private_path.join("tasks")).unwrap();
    std::fs::write(
        private_path.join("tasks/task2.md"),
        "---\ntype: task\nid: task2\nstatus: todo\n---\n\n# Task 2\n",
    )
    .unwrap();

    // Create company/ context (should be excluded)
    let company_path = tmp.path().join("company");
    std::fs::create_dir_all(company_path.join("tasks")).unwrap();
    std::fs::write(
        company_path.join("tasks/task3.md"),
        "---\ntype: task\nid: task3\nstatus: todo\n---\n\n# Task 3\n",
    )
    .unwrap();

    // Query multiple contexts using comma-separated --in flag
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "status:todo", "--in", "en,private"])
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
fn test_locate_find_type_filter() {
    let tmp = tempdir().unwrap();

    // Create en/ context with a task and a definition
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

    // Query for type:task should only return tasks
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "type:task"])
        .assert()
        .success()
        .stdout(predicate::str::contains("my-task"))
        .stdout(predicate::str::contains("hello").not());

    // Query for type:definition should only return definitions
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "find", "type:definition"])
        .assert()
        .success()
        .stdout(predicate::str::contains("hello"))
        .stdout(predicate::str::contains("my-task").not());
}

#[test]
fn test_locate_search_fulltext() {
    let tmp = tempdir().unwrap();

    // Create en/ context with tasks
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

    // Search for "important meeting" should find the meeting task
    co_command()
        .current_dir(tmp.path())
        .args(["locate", "search", "important meeting"])
        .assert()
        .success()
        .stdout(predicate::str::contains("meeting"))
        .stdout(predicate::str::contains("review").not());
}
