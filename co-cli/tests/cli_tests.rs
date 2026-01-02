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
