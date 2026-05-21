//! `co deploy` — publish a universe to a target platform.

use co::deploy::{DeployerAdapter, EnvSecretResolver, StaticOnR2Adapter};
use std::path::Path;

pub fn run(target: &str, dist: &str, manifest_path: &str, universe_id: &str) {
    if target != "static-on-r2" {
        eprintln!(
            "error: unsupported target '{target}'; only 'static-on-r2' is currently supported"
        );
        std::process::exit(1);
    }

    let manifest = co::deploy::parse_file(Path::new(manifest_path)).unwrap_or_else(|e| {
        eprintln!("error: {e}");
        std::process::exit(1);
    });

    let dist_path = Path::new(dist);
    if !dist_path.exists() {
        eprintln!("error: dist directory '{dist}' does not exist");
        std::process::exit(1);
    }

    let account_id = require_env("R2_ACCOUNT_ID");
    let access_key = require_env("R2_ACCESS_KEY_ID");
    let secret_key = require_env("R2_SECRET_ACCESS_KEY");
    let bucket = require_env("R2_BUCKET");
    let base_domain = std::env::var("CO_BASE_DOMAIN").unwrap_or_else(|_| "co.app".to_string());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let adapter = StaticOnR2Adapter::from_credentials(
            &account_id,
            &access_key,
            &secret_key,
            &bucket,
            &base_domain,
        )
        .await;

        println!("Deploying '{universe_id}' → {target}…");

        match adapter
            .deploy_from_dir(dist_path, universe_id, &manifest, &EnvSecretResolver)
            .await
        {
            Ok(result) => {
                println!("Deployed successfully!");
                println!("  Deploy ID: {}", result.deploy_id);
                println!("  URL:       {}", result.url);
                println!("  Snapshot:  {}", result.snapshot_id);
            }
            Err(e) => {
                eprintln!("error: deploy failed: {e}");
                std::process::exit(1);
            }
        }
    });
}

pub fn rollback(deploy_id: &str) {
    let account_id = require_env("R2_ACCOUNT_ID");
    let access_key = require_env("R2_ACCESS_KEY_ID");
    let secret_key = require_env("R2_SECRET_ACCESS_KEY");
    let bucket = require_env("R2_BUCKET");
    let base_domain = std::env::var("CO_BASE_DOMAIN").unwrap_or_else(|_| "co.app".to_string());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let adapter = StaticOnR2Adapter::from_credentials(
            &account_id,
            &access_key,
            &secret_key,
            &bucket,
            &base_domain,
        )
        .await;

        println!("Rolling back to '{deploy_id}'…");

        match adapter.rollback(deploy_id).await {
            Ok(()) => println!("Rollback complete."),
            Err(e) => {
                eprintln!("error: rollback failed: {e}");
                std::process::exit(1);
            }
        }
    });
}

fn require_env(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("error: {name} environment variable is required");
        std::process::exit(1);
    })
}
