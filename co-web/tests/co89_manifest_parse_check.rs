//! CO-89: smoke-check that the co-dev manifest (with commit/profile/event types
//! + views) parses with the real manifest parser. Temporary guard for this PR.
extern crate co;
#[test]
fn co_dev_manifest_parses() {
    let bytes = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../work/co/_universe.yaml"
    ))
    .expect("read manifest");
    let result = co::manifest::parse(&bytes).expect("manifest must parse");
    let names: Vec<&str> = result
        .manifest
        .content_types
        .iter()
        .map(|c| c.name.as_str())
        .collect();
    assert!(names.contains(&"commit"), "commit type present");
    assert!(names.contains(&"profile"), "profile type present");
    let views: Vec<&str> = result
        .manifest
        .views
        .iter()
        .map(|v| v.name.as_str())
        .collect();
    for v in ["history", "contributors", "events", "roadmap"] {
        assert!(views.contains(&v), "view {v} present");
    }
}
