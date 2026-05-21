//! `co validate deploy` — validate a deploy.yaml manifest

use co::deploy;
use colored::Colorize;
use std::path::Path;

pub fn run(path: &str) {
    let file_path = Path::new(path);

    if !file_path.exists() {
        eprintln!(
            "{} {}: file not found",
            "error:".red().bold(),
            file_path.display()
        );
        std::process::exit(1);
    }

    match deploy::parse_file(file_path) {
        Ok(manifest) => {
            println!(
                "{} {} (target: {}, version: {})",
                "✓".green().bold(),
                file_path.display(),
                target_str(&manifest.target),
                manifest.version,
            );
        }
        Err(e) => {
            eprintln!("{} {}", "error:".red().bold(), e);
            std::process::exit(1);
        }
    }
}

fn target_str(target: &deploy::DeployTarget) -> &'static str {
    match target {
        deploy::DeployTarget::StaticOnR2 => "static-on-r2",
        deploy::DeployTarget::CloudflarePages => "cloudflare-pages",
        deploy::DeployTarget::Fly => "fly",
        deploy::DeployTarget::Vercel => "vercel",
        deploy::DeployTarget::Fargate => "fargate",
    }
}
