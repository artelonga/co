use co_engine::config::EngineConfig;
use co_engine::context::Context;
use co_engine::engine::Engine;
use std::path::PathBuf;

pub enum EngineAction {
    Plan {
        repo: String,
        title: String,
        description: String,
    },
    Execute {
        repo: String,
        issue: u64,
        workdir: Option<String>,
    },
    Approve {
        repo: String,
        pr: u64,
    },
    Auto {
        repo: String,
        title: String,
        description: String,
        workdir: Option<String>,
    },
    Query {
        query: String,
    },
    Status,
}

pub fn run(action: EngineAction, vault: Option<PathBuf>, auto_approve: bool) {
    if let Err(e) = run_inner(action, vault, auto_approve) {
        eprintln!("error: {}", e);
        std::process::exit(1);
    }
}

fn run_inner(
    action: EngineAction,
    vault: Option<PathBuf>,
    auto_approve: bool,
) -> anyhow::Result<()> {
    let mut config = EngineConfig::load();
    if let Some(vault) = vault {
        config.vault_path = Some(vault);
    }
    if auto_approve {
        config.auto_approve = true;
    }

    let engine = Engine::new(config).with_defaults();

    match action {
        EngineAction::Plan {
            repo,
            title,
            description,
        } => {
            let mut ctx = Context::new(&repo, &title, &description);
            let log = engine.run_stage(&mut ctx, co_engine::context::Stage::Plan)?;
            println!(
                "\nIssue: {}",
                ctx.artifacts.get("issue_url").unwrap_or(&"(none)".into())
            );
            println!("\n{}", log.render_summary());
        }

        EngineAction::Execute {
            repo,
            issue,
            workdir,
        } => {
            // Fetch issue details via gh directly (outside engine pipeline)
            let issue_json = fetch_issue(&repo, issue)?;
            let issue_data: serde_json::Value = serde_json::from_str(&issue_json)?;
            let title = issue_data["title"]
                .as_str()
                .unwrap_or("untitled")
                .to_string();
            let body = issue_data["body"].as_str().unwrap_or("").to_string();

            let mut ctx = Context::new(&repo, &title, &body);
            ctx.workdir = workdir;
            ctx.artifacts
                .insert("issue_number".into(), issue.to_string());
            ctx.artifacts.insert("issue_body".into(), body);

            let log = engine.run_stage(&mut ctx, co_engine::context::Stage::Execute)?;
            println!(
                "\nPR: {}",
                ctx.artifacts.get("pr_url").unwrap_or(&"(none)".into())
            );
            println!("\n{}", log.render_summary());
        }

        EngineAction::Approve { repo, pr } => {
            let mut ctx = Context::new(&repo, "PR Review", "");
            ctx.artifacts.insert("pr_number".into(), pr.to_string());
            let log = engine.run_stage(&mut ctx, co_engine::context::Stage::Approve)?;
            println!(
                "\nVerdict: {}",
                ctx.artifacts
                    .get("review_action")
                    .unwrap_or(&"(none)".into())
            );
            println!("\n{}", log.render_summary());
        }

        EngineAction::Auto {
            repo,
            title,
            description,
            workdir,
        } => {
            let mut ctx = Context::new(&repo, &title, &description);
            ctx.workdir = workdir;
            let log = engine.run_auto(&mut ctx)?;
            println!("\n{}", log.render_summary());
        }

        EngineAction::Query { query } => {
            let obsidian = engine
                .tools
                .get("obsidian")
                .ok_or_else(|| anyhow::anyhow!("Obsidian tool not available"))?;

            if !obsidian.available() {
                anyhow::bail!("No Obsidian vault configured. Use --vault or set CO_ENGINE_VAULT");
            }

            let results = obsidian.run(
                "search",
                &std::collections::HashMap::from([("query".into(), query)]),
            )?;
            println!("{}", results);
        }

        EngineAction::Status => {
            println!("co engine — Pipeline Status\n");
            println!("Tools:");
            for tool_name in &["github", "claude", "obsidian"] {
                if let Some(tool) = engine.tools.get(tool_name) {
                    let status = if tool.available() {
                        "OK"
                    } else {
                        "UNAVAILABLE"
                    };
                    println!("  {} — {} [{}]", tool_name, tool.description(), status);
                }
            }
            println!("\nModules:");
            for (name, stage) in engine.modules.list() {
                println!("  {} → {}", name, stage);
            }
        }
    }

    Ok(())
}

/// Fetch issue details directly via gh CLI (outside engine pipeline)
fn fetch_issue(repo: &str, number: u64) -> anyhow::Result<String> {
    let output = std::process::Command::new("gh")
        .args([
            "issue",
            "view",
            &number.to_string(),
            "--repo",
            repo,
            "--json",
            "title,body,labels,state",
        ])
        .output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        anyhow::bail!("gh issue view failed: {}", stderr);
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Handle identity subcommands
pub fn run_identity(action: crate::IdentitySubcommand) {
    use co_engine::identity::{GitIdentity, IdentityStore};

    match action {
        crate::IdentitySubcommand::List => {
            let store = IdentityStore::load();
            println!("Git Identities:\n");
            for (name, id, active) in store.list() {
                let marker = if active { " *" } else { "" };
                println!(
                    "  {}{} — {} <{}> (ssh: {})",
                    name, marker, id.github_user, id.git_email, id.ssh_host
                );
            }
        }

        crate::IdentitySubcommand::Current => {
            let store = IdentityStore::load();
            if let Some(id) = store.active_identity() {
                println!("Active: {} ({})", store.active, id.github_user);
                println!("  Email: {}", id.git_email);
                println!("  Name: {}", id.git_name);
                println!("  SSH host: {}", id.ssh_host);
            } else {
                eprintln!("No active identity configured");
            }
        }

        crate::IdentitySubcommand::Switch { name } => {
            let mut store = IdentityStore::load();
            match store.switch(&name) {
                Ok(()) => println!("Switched to identity: {}", name),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }

        crate::IdentitySubcommand::Add {
            name,
            github_user,
            ssh_host,
            email,
            git_name,
        } => {
            let mut store = IdentityStore::load();
            let identity = GitIdentity {
                name: name.clone(),
                github_user,
                ssh_host,
                git_email: email,
                git_name: git_name.unwrap_or_else(|| name.clone()),
            };
            match store.add(identity) {
                Ok(()) => println!("Added identity: {}", name),
                Err(e) => {
                    eprintln!("error: {}", e);
                    std::process::exit(1);
                }
            }
        }
    }
}
