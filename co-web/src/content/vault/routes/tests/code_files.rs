use super::super::*;
use super::support::*;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tempfile::tempdir;
use tower::ServiceExt;

// ---------------------------------------------------------------------------
// CO-245: is_code_file / code_file_mime unit tests
// ---------------------------------------------------------------------------

#[test]
fn test_is_code_file_known_extensions() {
    for ext in &[
        "py", "rs", "ts", "js", "sh", "sql", "go", "r", "rb", "csv", "tsv", "html", "css", "xml",
        "cpp", "c", "java", "php",
    ] {
        let path = format!("uploads/script.{ext}");
        assert!(
            is_code_file(&path),
            "is_code_file should return true for .{ext}"
        );
    }
}

#[test]
fn test_is_code_file_rejects_non_code() {
    for path in &[
        "file.md",
        "file.pdf",
        "photo.jpg",
        "archive.zip",
        "data.yaml",
        "config.toml",
    ] {
        assert!(
            !is_code_file(path),
            "is_code_file should return false for {path}"
        );
    }
}

#[test]
fn test_code_file_mime_known_extensions() {
    assert_eq!(code_file_mime("script.py"), "text/x-python");
    assert_eq!(code_file_mime("main.rs"), "text/x-rust");
    assert_eq!(code_file_mime("app.ts"), "application/typescript");
    assert_eq!(code_file_mime("app.js"), "application/javascript");
    assert_eq!(code_file_mime("run.sh"), "text/x-shellscript");
    assert_eq!(code_file_mime("query.sql"), "text/x-sql");
    assert_eq!(code_file_mime("main.go"), "text/x-go");
    assert_eq!(code_file_mime("data.csv"), "text/csv");
    assert_eq!(code_file_mime("data.tsv"), "text/tab-separated-values");
    assert_eq!(code_file_mime("page.html"), "text/html");
    assert_eq!(code_file_mime("style.css"), "text/css");
}

#[tokio::test]
async fn test_vault_put_python_file_indexes_as_asset_code() {
    let dir = tempdir().unwrap();
    unsafe {
        std::env::set_var("JWT_SECRET", "test-jwt-secret");
    }
    let router = build_test_router(dir.path());
    let bearer = test_bearer();

    // Create a universe first
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/v1/universes")
                .header("Authorization", &bearer)
                .header("Content-Type", "application/json")
                .body(Body::from(
                    r#"{"name":"Test","key":"testco245","is_public":false}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        res.status().is_success(),
        "create universe: {:?}",
        res.status()
    );

    // PUT a Python file
    let python_code = b"def hello():\n    return 'world'\n";
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/api/v1/universes/testco245/vault/scripts/hello.py")
                .header("Authorization", &bearer)
                .header("Content-Type", "text/x-python")
                .body(Body::from(python_code.as_ref()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::CREATED, "put python file");

    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["frontmatter"]["type"].as_str().unwrap(),
        "asset.code",
        "entry_type should be asset.code"
    );
    assert_eq!(
        json["frontmatter"]["mime"].as_str().unwrap(),
        "text/x-python",
    );

    // Re-GET the entry to confirm indexing
    let res = router
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/v1/universes/testco245/entries/scripts/hello.py")
                .header("Authorization", &bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "get entry");
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["entry_type"].as_str().unwrap_or_default(),
        "asset.code",
        "re-fetched entry should be asset.code"
    );
}
