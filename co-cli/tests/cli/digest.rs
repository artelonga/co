//! `co universe digest` integration tests (CO-428).
//!
//! Seeds a 3-level universe tree (root → mid → leaf) into a temp data dir via
//! `co_web::storage::Storage`, then runs the CLI against it. The must-have is
//! **byte-stable determinism**: running the digest twice over the same DB must
//! produce identical stdout (it is used as a cacheable prompt prefix).

use super::co_command;
use co_web::models::CreateUniverse;
use co_web::storage::Storage;
use tempfile::tempdir;

/// Seed a deterministic 3-level tree into `data_dir`.
///
/// root
/// ├── mid
/// │   └── leaf   (2 tasks: todo, done)
fn seed_tree(data_dir: &std::path::Path) {
    let mut storage = Storage::new(data_dir);

    for (key, name, desc) in [
        ("root", "Root Universe", "The top of the forest."),
        ("mid", "Mid Universe", "A middle layer with children."),
        ("leaf", "Leaf Universe", "A leaf with tasks."),
    ] {
        storage
            .create_universe(
                CreateUniverse {
                    key: key.to_string(),
                    name: name.to_string(),
                    description: desc.to_string(),
                },
                "tester",
            )
            .expect("create_universe");
    }

    // Wire the hierarchy via parent_key (registry, not hardcoded).
    storage
        .conn()
        .execute(
            "UPDATE universes SET parent_key = ?1 WHERE key = ?2",
            rusqlite::params!["root", "mid"],
        )
        .unwrap();
    storage
        .conn()
        .execute(
            "UPDATE universes SET parent_key = ?1 WHERE key = ?2",
            rusqlite::params!["mid", "leaf"],
        )
        .unwrap();

    // Add two task entries to `leaf` with distinct statuses.
    let conn = storage.universe_conn("leaf");
    let guard = conn.lock().unwrap();
    for (path, status) in [("tasks/1.md", "todo"), ("tasks/2.md", "done")] {
        guard
            .execute(
                "INSERT INTO entries (path, universe_key, entry_type, title, frontmatter_json) \
                 VALUES (?1, 'leaf', 'task', 'T', ?2)",
                rusqlite::params![path, format!("{{\"status\":\"{}\"}}", status)],
            )
            .unwrap();
    }
}

#[test]
fn digest_is_byte_stable_across_runs() {
    let tmp = tempdir().unwrap();
    seed_tree(tmp.path());

    let run = || {
        co_command()
            .args(["universe", "digest", "--data-dir"])
            .arg(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    let first = run();
    let second = run();
    assert_eq!(
        first, second,
        "digest output must be byte-identical across runs"
    );
}

#[test]
fn digest_json_is_byte_stable_across_runs() {
    let tmp = tempdir().unwrap();
    seed_tree(tmp.path());

    let run = || {
        co_command()
            .args(["universe", "digest", "--format", "json", "--data-dir"])
            .arg(tmp.path())
            .assert()
            .success()
            .get_output()
            .stdout
            .clone()
    };

    assert_eq!(run(), run(), "json digest must be byte-identical");
}

#[test]
fn digest_renders_full_forest_recursively() {
    let tmp = tempdir().unwrap();
    seed_tree(tmp.path());

    let out = co_command()
        .args(["universe", "digest", "--data-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();

    // All three levels present.
    assert!(text.contains("**root**"));
    assert!(text.contains("**mid**"));
    assert!(text.contains("**leaf**"));
    // Hierarchy is rendered (parent + children pointers).
    assert!(text.contains("parent: root"));
    assert!(text.contains("parent: mid"));
    assert!(text.contains("children: mid"));
    // Task-by-status counts on the leaf (volatile layer, last).
    assert!(text.contains("[done=1 todo=1]"));
    // root appears before mid before leaf (DFS, stable order).
    let p_root = text.find("**root**").unwrap();
    let p_mid = text.find("**mid**").unwrap();
    let p_leaf = text.find("**leaf**").unwrap();
    assert!(p_root < p_mid && p_mid < p_leaf);
}

#[test]
fn digest_subtree_from_key() {
    let tmp = tempdir().unwrap();
    seed_tree(tmp.path());

    let out = co_command()
        .args(["universe", "digest", "mid", "--data-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();

    // Starting at `mid`: mid + leaf present, root absent from the forest body.
    assert!(text.contains("**mid**"));
    assert!(text.contains("**leaf**"));
    assert!(!text.contains("**root**"));
}

#[test]
fn digest_depth_limit_excludes_grandchildren() {
    let tmp = tempdir().unwrap();
    seed_tree(tmp.path());

    let out = co_command()
        .args(["universe", "digest", "--depth", "1", "--data-dir"])
        .arg(tmp.path())
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let text = String::from_utf8(out).unwrap();

    // depth 1: root + mid present; leaf (grandchild) excluded.
    assert!(text.contains("**root**"));
    assert!(text.contains("**mid**"));
    assert!(!text.contains("**leaf**"));
}
