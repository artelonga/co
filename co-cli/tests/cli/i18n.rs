//! Multilingual UI support tests (US-1.3)

use super::co_command;
use predicates::prelude::*;
use tempfile::tempdir;

#[test]
fn test_english_ui_file_parseable() {
    let content = include_str!("../../../ui/en.yaml");
    let labels: co::UiLabels = serde_yaml::from_str(content).expect("Should parse en.yaml");
    assert_eq!(labels.lang, "en");
    assert!(labels.types.contains_key("definition"));
    assert!(labels.fields.contains_key("status"));
    assert!(labels.messages.contains_key("initialized"));
}

#[test]
fn test_portuguese_ui_file_parseable() {
    let content = include_str!("../../../ui/pt.yaml");
    let labels: co::UiLabels = serde_yaml::from_str(content).expect("Should parse pt.yaml");
    assert_eq!(labels.lang, "pt");
    assert_eq!(labels.type_label("definition"), "definição");
    assert_eq!(labels.field_label("status"), "estado");
    assert_eq!(labels.message("initialized"), "Inicializado com sucesso");
}

#[test]
fn test_lang_sets_system_language() {
    let tmp = tempdir().unwrap();

    co_command()
        .current_dir(tmp.path())
        .args(["lang", "pt"])
        .assert()
        .success();

    let config_path = tmp.path().join(".co/config.yaml");
    assert!(config_path.exists());

    let config = std::fs::read_to_string(&config_path).unwrap();
    assert!(config.contains("system_language: pt"));
}

#[test]
fn test_languages_alias_works() {
    co_command()
        .args(["languages", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("english"))
        .stdout(predicate::str::contains("portuguese"))
        .stdout(predicate::str::contains("math"));

    co_command()
        .args(["lang", "--list"])
        .assert()
        .success()
        .stdout(predicate::str::contains("english"))
        .stdout(predicate::str::contains("portuguese"))
        .stdout(predicate::str::contains("math"));
}
