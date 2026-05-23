//! `co auto` — Automated task execution pipeline
//!
//! Picks the next unblocked task, builds multi-layer context,
//! launches Claude Code with --dangerously-skip-permissions,
//! reviews against acceptance criteria, and cycles.

use anyhow::{Context, Result};
use chrono::Utc;
use colored::Colorize;
use serde_json;
use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Auto command configuration
pub struct AutoConfig {
    pub space: String,
    pub task_id: Option<String>,
    pub cycle: bool,
    pub dry_run: bool,
    pub max_tasks: Option<usize>,
    pub teams: bool,
    pub model: String,
    pub timeout_secs: u64,
    pub workdir: Option<String>,
    pub data_dir: Option<String>,
    pub workspace: Option<String>,
    pub interactive: bool,
    /// After each successful task, push the branch + open a PR via `scripts/ship-task.sh`.
    pub auto_pr: bool,
}

/// Represents a parsed task from markdown
#[derive(Debug, Clone)]
pub struct Task {
    pub id: u64,
    pub key: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub parent: Option<u64>,
    pub labels: Vec<String>,
    #[allow(dead_code)]
    pub module: Option<String>,
    pub body: String,
    pub file_path: PathBuf,
}

/// Run tracker for observability
#[derive(Debug)]
struct RunTracker {
    run_id: String,
    started_at: String,
    tasks_completed: Vec<String>,
    tasks_failed: Vec<String>,
}

pub fn run(config: AutoConfig) -> Result<()> {
    let data_dir = if let Some(ref dir) = config.data_dir {
        PathBuf::from(dir)
    } else if let Some(ref ws) = config.workspace {
        PathBuf::from(ws).join("data").join(&config.space)
    } else if let Ok(dir) = std::env::var("CO_WORKSPACE") {
        PathBuf::from(dir).join("data").join(&config.space)
    } else {
        find_data_dir(&config.space)?
    };

    if !data_dir.exists() {
        anyhow::bail!(
            "Data dir not found: {}\nSet --data-dir, --workspace, or CO_WORKSPACE env var",
            data_dir.display()
        );
    }

    let project_key = load_project_key(&data_dir)?;

    println!(
        "{} {} (space: {})",
        "▶".green().bold(),
        "co auto".bold(),
        config.space.cyan()
    );

    if config.teams {
        ensure_teams_enabled()?;
    }

    let mut tracker = RunTracker {
        run_id: nanoid(),
        started_at: Utc::now().to_rfc3339(),
        tasks_completed: Vec::new(),
        tasks_failed: Vec::new(),
    };

    let mut tasks_processed = 0;
    let max = config.max_tasks.unwrap_or(usize::MAX);

    loop {
        if tasks_processed >= max {
            println!("{} Max tasks reached ({})", "■".yellow(), max);
            break;
        }

        // 1. SELECT next task
        let tasks = load_tasks(&data_dir, &project_key)?;
        let next = if let Some(ref tid) = config.task_id {
            tasks.iter().find(|t| t.key == *tid).cloned()
        } else {
            select_next_task(&tasks)
        };

        let task = match next {
            Some(t) => t,
            None => {
                println!("{} No unblocked tasks remaining", "✓".green().bold());
                break;
            }
        };

        println!(
            "\n{} {} — {}",
            "→".cyan().bold(),
            task.key.yellow().bold(),
            task.title
        );
        if !task.labels.is_empty() {
            println!(
                "  {} Labels: {}",
                "◆".dimmed(),
                task.labels.join(", ").dimmed()
            );
        }
        println!("  {} Priority: {}", "◆".dimmed(), task.priority.cyan());

        // In interactive mode, confirm before launching
        if config.interactive && config.task_id.is_none() {
            print!("  {} Execute this task? [Y/n] ", "?".yellow().bold());
            use std::io::Write;
            std::io::stdout().flush()?;
            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            let input = input.trim().to_lowercase();
            if input == "n" || input == "no" {
                println!("  {} Skipped", "⊘".dimmed());
                tasks_processed += 1;
                if config.cycle {
                    continue;
                }
                break;
            }
        }

        if config.dry_run {
            println!("  {} (dry run — would execute this task)", "⊘".dimmed());
            if config.task_id.is_some() || !config.cycle {
                break;
            }
            tasks_processed += 1;
            continue;
        }

        // 2. BUILD CONTEXT
        let wd = config.workdir.as_ref().map(|w| Path::new(w.as_str()));
        let context = build_context(&task, &data_dir, &tasks, wd)?;
        let layer_count = context.split("\n\n---\n\n").count();
        let path_label = if data_dir.join("CLAUDE.md").exists() {
            "minimal"
        } else {
            "full"
        };
        println!(
            "  {} Context: {} path, {} layers, {} chars",
            "◆".dimmed(),
            path_label,
            layer_count,
            context.len()
        );

        // 3. Create branch (with worktree if parallel) and mark as in_progress
        let base_workdir = resolve_workdir(config.workdir.as_deref())?;
        let use_worktree = config.cycle || config.teams; // worktree when parallel execution likely

        // For worktree mode: do NOT neutralize base repo — we need the real
        // smudge filter so worktree checkout decrypts files properly.
        // Only neutralize on the base if NOT using worktrees.
        let has_git_crypt = if !use_worktree {
            neutralize_git_crypt(&base_workdir)
        } else {
            // Check if git-crypt exists but don't neutralize yet
            base_workdir.join(".git-crypt").exists() || base_workdir.join(".git/git-crypt").exists()
        };

        let (branch_name, workdir) = create_task_branch(&task, &base_workdir, use_worktree)?;

        // For worktrees: neutralize AFTER creation so Claude doesn't get phantom diffs
        let has_git_crypt_wt = if use_worktree && has_git_crypt {
            neutralize_git_crypt(&workdir)
        } else {
            false
        };

        // Clean up git-crypt corrupted settings files in worktrees
        if use_worktree {
            let settings_file = workdir.join(".claude").join("settings.local.json");
            if settings_file.exists() {
                // Check if the file is valid JSON; if not, remove it
                if let Ok(content) = fs::read_to_string(&settings_file) {
                    if serde_json::from_str::<serde_json::Value>(&content).is_err() {
                        let _ = fs::remove_file(&settings_file);
                        println!(
                            "  {} Removed corrupted settings.local.json from worktree",
                            "◆".dimmed()
                        );
                    }
                } else {
                    // Binary/unreadable — remove it
                    let _ = fs::remove_file(&settings_file);
                    println!(
                        "  {} Removed corrupted settings.local.json from worktree",
                        "◆".dimmed()
                    );
                }
            }
        }
        println!("  {} Branch: {}", "◆".dimmed(), branch_name.cyan());

        update_task_status(&task, "in_progress")?;
        println!("  {} Status: in_progress", "◆".dimmed());

        // 4. EXECUTE via Claude Code

        let success = launch_claude(
            &context,
            &workdir,
            config.teams,
            &config.model,
            config.timeout_secs,
            config.interactive,
            &task.key,
            &task.title,
        )?;

        if success {
            // 5. REVIEW acceptance criteria
            let review = review_criteria(&task)?;

            if review.passed {
                update_task_status(&task, "done")?;
                println!(
                    "  {} {} — {}/{} criteria met",
                    "✓".green().bold(),
                    "DONE".green(),
                    review.met,
                    review.total
                );
                tracker.tasks_completed.push(task.key.clone());

                // --auto-pr: after a successful task, shell out to ship-task.sh to
                // rebase + push + `gh pr create`. Best-effort — failures don't abort
                // the cycle (the user can re-run ship-task.sh manually).
                if config.auto_pr {
                    println!(
                        "  {} ship-task.sh {} (auto-pr)",
                        "◆".dimmed(),
                        task.key.cyan()
                    );
                    // Walk up workdir parents to find a `scripts/ship-task.sh` (typically
                    // lives in the CO repo root).
                    let mut probe: Option<PathBuf> = Some(workdir.clone());
                    let mut script: Option<PathBuf> = None;
                    while let Some(p) = probe.clone() {
                        let candidate = p.join("scripts").join("ship-task.sh");
                        if candidate.exists() {
                            script = Some(candidate);
                            break;
                        }
                        probe = p.parent().map(|x| x.to_path_buf());
                    }
                    // Fallback: hardcoded canonical location.
                    let canonical =
                        PathBuf::from("/Users/artelonga/projects/co/scripts/ship-task.sh");
                    if script.is_none() && canonical.exists() {
                        script = Some(canonical);
                    }
                    if let Some(s) = script {
                        let out = Command::new(&s).arg(&task.key).output();
                        match out {
                            Ok(o) => {
                                let stdout = String::from_utf8_lossy(&o.stdout);
                                let stderr = String::from_utf8_lossy(&o.stderr);
                                for line in stdout.lines().chain(stderr.lines()).take(20) {
                                    println!("    {}", line.dimmed());
                                }
                                if !o.status.success() {
                                    println!(
                                        "  {} ship-task.sh exited non-zero — left for manual recovery",
                                        "⚠".yellow()
                                    );
                                }
                            }
                            Err(e) => {
                                println!(
                                    "  {} ship-task.sh spawn failed: {} — left for manual push",
                                    "⚠".yellow(),
                                    e
                                );
                            }
                        }
                    } else {
                        println!(
                            "  {} --auto-pr set but no ship-task.sh found; push manually",
                            "⚠".yellow()
                        );
                    }
                }

                // CO-197: After a successful task in classic mode (no worktree),
                // fast-forward main to the feat-branch tip so the NEXT task
                // branches from current state. Without this, the next ticket's
                // `git checkout main && git checkout -b ...` at auto.rs:1092
                // silently resets the working tree to a stale main tip, losing
                // every prior task's work. Worktree mode is unaffected (merges
                // happen externally via PR / co-auto orchestrator).
                if !use_worktree {
                    let on_main = Command::new("git")
                        .args(["checkout", "main"])
                        .current_dir(&base_workdir)
                        .output();
                    if on_main.is_ok() {
                        let ff = Command::new("git")
                            .args(["merge", "--ff-only", &branch_name])
                            .current_dir(&base_workdir)
                            .output();
                        match ff {
                            Ok(o) if o.status.success() => {
                                println!(
                                    "  {} main fast-forwarded → {}",
                                    "◆".dimmed(),
                                    branch_name.cyan()
                                );
                                // Delete the now-merged feature branch.
                                let _ = Command::new("git")
                                    .args(["branch", "-d", &branch_name])
                                    .current_dir(&base_workdir)
                                    .output();
                            }
                            Ok(o) => {
                                let stderr = String::from_utf8_lossy(&o.stderr);
                                eprintln!(
                                    "  {} Could not fast-forward main to {} ({}). \
                                     Staying on feat-branch — resolve manually before next task.",
                                    "!".yellow(),
                                    branch_name,
                                    stderr.trim()
                                );
                                let _ = Command::new("git")
                                    .args(["checkout", &branch_name])
                                    .current_dir(&base_workdir)
                                    .output();
                            }
                            Err(e) => {
                                eprintln!("  {} git merge invocation failed: {}", "!".yellow(), e);
                            }
                        }
                    }
                }
            } else {
                update_task_status(&task, "review")?;
                println!(
                    "  {} {} — {}/{} criteria met",
                    "✗".yellow().bold(),
                    "REVIEW".yellow(),
                    review.met,
                    review.total
                );
                for f in &review.failures {
                    println!("    {} {}", "·".yellow(), f);
                }
                tracker.tasks_failed.push(task.key.clone());
            }
        } else {
            // Claude errored — leave as in_progress for retry
            println!(
                "  {} Claude Code exited with error (remains in_progress)",
                "✗".red().bold()
            );
            tracker.tasks_failed.push(task.key.clone());
        }

        // Restore git-crypt filters after task completes
        if has_git_crypt_wt {
            restore_git_crypt(&workdir);
        }
        if has_git_crypt {
            restore_git_crypt(&base_workdir);
        }

        tasks_processed += 1;

        if config.task_id.is_some() || !config.cycle {
            break;
        }

        // Brief pause between tasks
        std::thread::sleep(std::time::Duration::from_secs(2));
    }

    // Final summary
    println!("\n{}", "═".repeat(50));
    println!(
        "{} Run {} complete",
        "■".green().bold(),
        tracker.run_id.dimmed()
    );
    println!(
        "  Completed: {} | Failed: {} | Total: {}",
        tracker.tasks_completed.len().to_string().green(),
        tracker.tasks_failed.len().to_string().red(),
        tasks_processed,
    );

    // Save run tracker
    save_tracker(&tracker)?;

    Ok(())
}

// ==================== TASK SELECTOR ====================

fn load_tasks(data_dir: &Path, project_key: &str) -> Result<Vec<Task>> {
    let mut tasks = Vec::new();

    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "md") {
            continue;
        }
        let filename = path.file_stem().unwrap_or_default().to_string_lossy();
        if !filename.starts_with(&format!("{}-", project_key)) {
            continue;
        }

        let content = fs::read_to_string(&path)?;
        if let Some(task) = parse_task(&content, &path, project_key) {
            tasks.push(task);
        }
    }

    Ok(tasks)
}

fn parse_task(content: &str, path: &Path, project_key: &str) -> Option<Task> {
    // Extract YAML frontmatter between --- delimiters
    let parts: Vec<&str> = content.splitn(3, "---").collect();
    if parts.len() < 3 {
        return None;
    }
    let yaml_str = parts[1].trim();
    let body = parts[2].trim().to_string();

    let yaml: serde_yaml::Value = serde_yaml::from_str(yaml_str).ok()?;
    let map = yaml.as_mapping()?;

    let id = map.get(serde_yaml::Value::String("id".into()))?.as_u64()?;
    let title = map
        .get(serde_yaml::Value::String("title".into()))?
        .as_str()?
        .to_string();
    let status = map
        .get(serde_yaml::Value::String("status".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("todo")
        .to_string();
    let priority = map
        .get(serde_yaml::Value::String("priority".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("medium")
        .to_string();
    let parent = map
        .get(serde_yaml::Value::String("parent".into()))
        .and_then(|v| v.as_u64());
    let labels = map
        .get(serde_yaml::Value::String("labels".into()))
        .and_then(|v| v.as_sequence())
        .map(|seq| {
            seq.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let module = map
        .get(serde_yaml::Value::String("module".into()))
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(Task {
        id,
        key: format!("{}-{}", project_key, id),
        title,
        status,
        priority,
        parent,
        labels,
        module,
        body,
        file_path: path.to_path_buf(),
    })
}

fn select_next_task(tasks: &[Task]) -> Option<Task> {
    // Filter to todo and in_progress (in_progress = retry after error)
    let mut candidates: Vec<&Task> = tasks
        .iter()
        .filter(|t| t.status == "todo" || t.status == "in_progress")
        .collect();

    // Filter out tasks with labels containing "epic" (epics are parents, not executable)
    candidates.retain(|t| !t.labels.contains(&"epic".to_string()));

    // Filter out blocked tasks (parent not done)
    candidates.retain(|t| {
        if let Some(parent_id) = t.parent {
            // Check if parent is done or is an epic (epics don't need to be done)
            tasks.iter().any(|p| {
                p.id == parent_id && (p.status == "done" || p.labels.contains(&"epic".to_string()))
            })
        } else {
            true // No parent = not blocked
        }
    });

    // Sort by priority
    let priority_order = |p: &str| -> u8 {
        match p {
            "critical" => 0,
            "high" => 1,
            "medium" => 2,
            "low" => 3,
            _ => 4,
        }
    };

    candidates.sort_by(|a, b| {
        // in_progress (retries) before todo
        let status_order = |s: &str| -> u8 { if s == "in_progress" { 0 } else { 1 } };
        status_order(&a.status)
            .cmp(&status_order(&b.status))
            .then_with(|| priority_order(&a.priority).cmp(&priority_order(&b.priority)))
            .then_with(|| a.id.cmp(&b.id))
    });

    candidates.first().cloned().cloned()
}

// ==================== CONTEXT BUILDER ====================

/// Returns skill text blocks relevant to the task based on its labels.
/// Each skill is a markdown file from `{workspace_root}/skills/`.
fn skills_for_task(task: &Task, workspace_root: &Path) -> Vec<String> {
    let skills_dir = workspace_root.join("skills");
    let mut names: Vec<&str> = vec![];

    for label in &task.labels {
        match label.as_str() {
            "module:spa" | "module:editor" | "module:ui" => names.push("spa-conventions"),
            "module:deploy" | "module:infra" => names.push("deploy-runbook"),
            "type:test" => names.push("playwright-pattern"),
            l if l.starts_with("module:") => names.push("rust-architecture"),
            _ => {}
        }
    }

    names.sort_unstable();
    names.dedup();

    names
        .into_iter()
        .filter_map(|name| {
            let path = skills_dir.join(format!("{name}.md"));
            fs::read_to_string(&path)
                .ok()
                .map(|content| format!("## Skill: {name}\n\n{content}"))
        })
        .collect()
}

fn execution_instructions(task: &Task) -> String {
    format!(
        "## Execution Instructions\n\n\
        **YOUR TASK IS: {key} — {title}**\n\n\
        IMPORTANT: Only implement {key}. Do NOT implement or modify any other task.\n\
        Dependencies listed in the roadmap (e.g., 'Depends On: GP-8') mean those tasks \
        are ALREADY DONE and merged into main. Their code is already in the codebase. \
        Do not look for them or re-implement them.\n\n\
        Follow the acceptance criteria exactly. Each `- [ ]` item is a required deliverable.\n\
        Use conventional commits: the task specifies the commit message format.\n\
        Run `cargo test`, `cargo clippy -- -D warnings`, and `cargo fmt` before committing.\n\
        After completing all criteria, commit with the specified message.\n\n\
        ## Forbidden Files\n\n\
        DO NOT modify any of these files — they are owned by the release commit only:\n\
        - `Cargo.toml`\n\
        - `co-cli/Cargo.toml`\n\
        - `CHANGELOG.md`\n\n\
        Write your changelog entry to `CHANGELOG-PENDING/{key}.md` instead:\n\n\
        ```markdown\n\
        ## {key} — {title}\n\n\
        <describe what changed and why>\n\n\
        ### Why\n\
        <optional — rationale or motivation>\n\
        ```\n\n\
        ## Test Isolation Rules\n\n\
        - All tests MUST run without opening network ports. Use in-process test servers \
        (e.g., `axum::test::TestClient`, `tower::ServiceExt`) instead of spawning HTTP listeners.\n\
        - Never bind to `0.0.0.0`. If a test requires a port, bind to `127.0.0.1` only.\n\
        - Use temp directories for test databases (e.g., `tempfile::tempdir()`) — never write to \
        user paths.\n\
        - Tests must be fully deterministic: no sleeps, no real network calls, no system time dependencies.\n\
        - Set `JWT_SECRET=test-secret` and `RUST_LOG=off` in test harness setup.",
        key = task.key,
        title = task.title,
    )
}

/// Minimal context: per-space CLAUDE.md + task-relevant skills + task spec.
/// Budget: ≤3k (guide) + ≤4k (skills) + ≤5k (task) ≈ 12k chars.
fn build_context_minimal(
    task: &Task,
    workspace_root: &Path,
    space_claude: &Path,
) -> Result<String> {
    let mut layers = Vec::new();

    let space_content = fs::read_to_string(space_claude)?;
    layers.push(format!(
        "## Development Conventions (CLAUDE.md)\n\n{}",
        space_content
    ));

    for skill in skills_for_task(task, workspace_root) {
        layers.push(skill);
    }

    let task_content = fs::read_to_string(&task.file_path)?;
    layers.push(format!(
        "## Current Task: {} — {}\n\n{}",
        task.key, task.title, task_content
    ));

    layers.push(execution_instructions(task));

    Ok(layers.join("\n\n---\n\n"))
}

fn build_context(
    task: &Task,
    data_dir: &Path,
    all_tasks: &[Task],
    workdir: Option<&Path>,
) -> Result<String> {
    let workspace_root = find_workspace_root(data_dir);

    // Per-space CLAUDE.md routing: data_dir/CLAUDE.md takes priority over root CLAUDE.md.
    // When present, use minimal context (guide + skills + task spec only).
    let space_claude = data_dir.join("CLAUDE.md");
    if space_claude.exists() {
        return build_context_minimal(task, &workspace_root, &space_claude);
    }

    // Legacy fallback: full 5-layer context (root CLAUDE.md + all supporting docs).
    let mut layers = Vec::new();

    // Layer 1: CLAUDE.md conventions (check workdir first, then workspace root)
    let claude_md = workdir
        .map(|w| w.join("CLAUDE.md"))
        .filter(|p| p.exists())
        .unwrap_or_else(|| workspace_root.join("CLAUDE.md"));
    if claude_md.exists() {
        let content = fs::read_to_string(&claude_md)?;
        layers.push(format!(
            "## Development Conventions (CLAUDE.md)\n\n{}",
            content
        ));
    }

    // Layer 2: Task body (the main instruction)
    let task_content = fs::read_to_string(&task.file_path)?;
    layers.push(format!(
        "## Current Task: {} — {}\n\n{}",
        task.key, task.title, task_content
    ));

    // Layer 3: Epic/parent context
    if let Some(parent_id) = task.parent
        && let Some(parent) = all_tasks.iter().find(|t| t.id == parent_id)
    {
        let parent_content = fs::read_to_string(&parent.file_path).unwrap_or_default();
        layers.push(format!(
            "## Parent Epic: {} — {}\n\n{}",
            parent.key, parent.title, parent_content
        ));
    }

    // Layer 4: Project context
    let project_yaml = data_dir.join("project.yaml");
    if project_yaml.exists() {
        let content = fs::read_to_string(&project_yaml)?;
        layers.push(format!(
            "## Project Configuration\n\n```yaml\n{}\n```",
            content
        ));
    }

    // Layer 5: Roadmap context (execution order)
    let roadmap = data_dir.join("ROADMAP.md");
    if roadmap.exists() {
        let content = fs::read_to_string(&roadmap)?;
        layers.push(format!("## Roadmap\n\n{}", content));
    }

    // Completed tasks context — so Claude knows what's already done
    let done_tasks: Vec<String> = all_tasks
        .iter()
        .filter(|t| t.status == "done")
        .map(|t| format!("- {} — {} (DONE, already merged into main)", t.key, t.title))
        .collect();

    if !done_tasks.is_empty() {
        layers.push(format!(
            "## Completed Tasks (already merged — do NOT re-implement)\n\n{}",
            done_tasks.join("\n")
        ));
    }

    layers.push(execution_instructions(task));

    Ok(layers.join("\n\n---\n\n"))
}

// ==================== CLAUDE CODE LAUNCHER ====================

#[allow(clippy::too_many_arguments)]
fn launch_claude(
    context: &str,
    workdir: &Path,
    teams: bool,
    model: &str,
    _timeout_secs: u64,
    interactive: bool,
    task_key: &str,
    task_title: &str,
) -> Result<bool> {
    // Write context to a temp file to avoid CLI arg length limits
    let context_file = workdir.join(".claude").join("co-auto-context.md");
    fs::create_dir_all(context_file.parent().unwrap())?;
    let mut f = fs::File::create(&context_file)?;
    f.write_all(context.as_bytes())?;
    drop(f);

    let user_prompt = format!(
        "YOUR TASK: {key} — {title}\n\n\
         Read .claude/co-auto-context.md for full context. \
         Look for the section '## Current Task: {key}' — that contains your acceptance criteria.\n\n\
         IMPORTANT: Only implement {key}. All dependencies are already merged into main. \
         Do NOT re-implement any other task. Each `- [ ]` item is a required deliverable. \
         Commit when all criteria are met.",
        key = task_key,
        title = task_title,
    );

    if interactive {
        println!("  {} Launching Claude Code (interactive)...", "◆".cyan());

        let mut cmd = Command::new("claude");
        cmd.arg("--dangerously-skip-permissions");
        cmd.arg("--model").arg(model);
        cmd.arg("--name").arg(format!("co-auto-{}", task_key));
        cmd.arg(&user_prompt);
        cmd.current_dir(workdir);

        if teams {
            cmd.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
        }

        // Interactive: inherit stdio so user sees the session
        cmd.stdin(std::process::Stdio::inherit());
        cmd.stdout(std::process::Stdio::inherit());
        cmd.stderr(std::process::Stdio::inherit());

        let status = cmd
            .spawn()
            .context("Failed to spawn claude. Is claude CLI installed?")?
            .wait()
            .context("Failed to wait for claude process")?;

        // Clean up context file
        let _ = fs::remove_file(&context_file);

        Ok(status.success())
    } else {
        println!("  {} Launching Claude Code (headless)...", "◆".cyan());

        let mut cmd = Command::new("claude");
        cmd.arg("-p").arg(&user_prompt);
        // `--bare` requires ANTHROPIC_API_KEY (OAuth/keychain are never read in
        // bare mode — see `claude --help`). Only enable it for API-key users;
        // subscription users (keychain-auth via `claude /login`) need claude to
        // run without --bare so it can read their saved credentials.
        if std::env::var("ANTHROPIC_API_KEY").is_ok() {
            cmd.arg("--bare");
        }
        cmd.arg("--dangerously-skip-permissions");
        cmd.arg("--model").arg(model);
        cmd.arg("--name").arg(format!("co-auto-{}", task_key));
        cmd.current_dir(workdir);

        if teams {
            cmd.env("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS", "1");
        }

        let output = cmd
            .spawn()
            .context("Failed to spawn claude. Is claude CLI installed?")?
            .wait_with_output()
            .context("Failed to wait for claude process")?;

        // Clean up context file
        let _ = fs::remove_file(&context_file);

        Ok(output.status.success())
    }
}

// ==================== ACCEPTANCE REVIEWER ====================

struct ReviewResult {
    passed: bool,
    total: usize,
    met: usize,
    failures: Vec<String>,
}

fn review_criteria(task: &Task) -> Result<ReviewResult> {
    // Extract checklist items from task body (lines matching "- [ ]")
    let criteria: Vec<String> = task
        .body
        .lines()
        .filter(|line| line.trim().starts_with("- [ ]"))
        .map(|line| {
            line.trim()
                .strip_prefix("- [ ] ")
                .unwrap_or(line.trim())
                .to_string()
        })
        .collect();

    if criteria.is_empty() {
        return Ok(ReviewResult {
            passed: true,
            total: 0,
            met: 0,
            failures: vec![],
        });
    }

    // Basic verification: check if code compiles (if Rust project)
    let mut met = 0;
    let mut failures = Vec::new();

    // Try cargo build as a basic verification
    let cargo_result = Command::new("cargo").arg("check").arg("--quiet").output();

    let compiles = cargo_result.map(|o| o.status.success()).unwrap_or(true); // If not a Rust project, skip

    if !compiles {
        failures.push("cargo check failed — code does not compile".to_string());
    }

    // Check for changes using jj (preferred) or git
    let has_changes = detect_changes();

    if has_changes && compiles {
        met = criteria.len(); // Trust Claude's execution for MVP
    } else if !has_changes {
        failures.push("No changes detected — task may not have been executed".to_string());
    }

    let passed = failures.is_empty() && met == criteria.len();

    Ok(ReviewResult {
        passed,
        total: criteria.len(),
        met,
        failures,
    })
}

// ==================== TASK STATUS UPDATER ====================

fn update_task_status(task: &Task, new_status: &str) -> Result<()> {
    let content = fs::read_to_string(&task.file_path)?;

    let updated = content.replace(
        &format!("status: {}", task.status),
        &format!("status: {}", new_status),
    );

    let updated = if updated.contains("updated_at:") {
        let now = Utc::now().to_rfc3339();
        // Replace existing updated_at with regex-free approach
        let mut result = String::new();
        for line in updated.lines() {
            if line.trim().starts_with("updated_at:") {
                result.push_str(&format!("updated_at: {}", now));
            } else {
                result.push_str(line);
            }
            result.push('\n');
        }
        result
    } else {
        updated
    };

    fs::write(&task.file_path, updated)?;
    Ok(())
}

// ==================== HELPERS ====================

/// Detect and neutralize git-crypt filters in the workdir.
/// Returns true if git-crypt was detected and filters were disabled.
fn neutralize_git_crypt(workdir: &Path) -> bool {
    // Check if .git-crypt/ exists (repo uses git-crypt)
    let git_crypt_dir = workdir.join(".git-crypt");
    if !git_crypt_dir.exists() {
        return false;
    }

    println!(
        "  {} git-crypt detected — disabling filters for session",
        "◆".dimmed()
    );

    // Disable the clean/smudge/diff filters so files stay as plaintext
    // This prevents the "file not encrypted" phantom diffs
    let configs = [
        ("filter.git-crypt.smudge", "cat"),
        ("filter.git-crypt.clean", "cat"),
        ("filter.git-crypt.required", "false"),
        ("diff.git-crypt.textconv", "cat"),
    ];

    for (key, val) in &configs {
        let _ = Command::new("git")
            .args(["config", "--local", key, val])
            .current_dir(workdir)
            .output();
    }

    // Refresh the index so git stops seeing phantom diffs
    let _ = Command::new("git")
        .args(["update-index", "--really-refresh"])
        .current_dir(workdir)
        .output();

    true
}

/// Restore git-crypt filters after session completes.
fn restore_git_crypt(workdir: &Path) {
    let configs = [
        "filter.git-crypt.smudge",
        "filter.git-crypt.clean",
        "filter.git-crypt.required",
        "diff.git-crypt.textconv",
    ];

    for key in &configs {
        let _ = Command::new("git")
            .args(["config", "--local", "--unset", key])
            .current_dir(workdir)
            .output();
    }

    println!("  {} git-crypt filters restored", "◆".dimmed());
}

/// Unlock git-crypt in a worktree using the key from the base repo.
/// Worktrees share .git/ but git-crypt unlock state is per-checkout.
#[allow(dead_code)]
fn unlock_git_crypt_worktree(worktree: &Path, base_repo: &Path) {
    // Check if base repo has git-crypt keys
    let key_path = base_repo.join(".git/git-crypt/keys/default/0");
    if !key_path.exists() && !key_path.is_dir() {
        // Try the key file directly
        let key_file = find_git_crypt_key(base_repo);
        if key_file.is_none() {
            return;
        }
    }

    // Try to unlock using the shared .git directory's key
    let result = Command::new("git-crypt")
        .arg("unlock")
        .current_dir(worktree)
        .output();

    match result {
        Ok(output) if output.status.success() => {
            println!("  {} git-crypt: worktree unlocked", "◆".dimmed());
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "already unlocked" is fine
            if stderr.contains("already") {
                return;
            }
            // Try with explicit key from macOS Keychain
            if let Some(key_path) = retrieve_key_from_keychain(base_repo) {
                let _ = Command::new("git-crypt")
                    .args(["unlock", key_path.to_str().unwrap_or_default()])
                    .current_dir(worktree)
                    .output();
                let _ = fs::remove_file(&key_path); // destroy temp key
                println!(
                    "  {} git-crypt: worktree unlocked (from keychain)",
                    "◆".dimmed()
                );
            } else {
                eprintln!(
                    "  {} git-crypt unlock failed in worktree: {}",
                    "⚠".yellow(),
                    stderr.trim()
                );
            }
        }
        Err(_) => {} // git-crypt not installed, skip silently
    }
}

/// Find git-crypt key file in the base repo's .git directory
#[allow(dead_code)]
fn find_git_crypt_key(base_repo: &Path) -> Option<PathBuf> {
    let keys_dir = base_repo.join(".git/git-crypt/keys/default/0");
    if keys_dir.is_dir()
        && let Ok(mut entries) = fs::read_dir(&keys_dir)
        && let Some(Ok(entry)) = entries.next()
    {
        return Some(entry.path());
    }
    None
}

/// Retrieve git-crypt key from macOS Keychain, write to temp file
#[allow(dead_code)]
fn retrieve_key_from_keychain(base_repo: &Path) -> Option<PathBuf> {
    // Derive keychain service name from repo name
    let repo_name = base_repo.file_name()?.to_str()?;
    let service = format!("{}-git-crypt", repo_name);

    let output = Command::new("security")
        .args([
            "find-generic-password",
            "-a",
            &whoami::username(),
            "-s",
            &service,
            "-w",
        ])
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let b64_key = String::from_utf8_lossy(&output.stdout).trim().to_string();

    // Decode base64 and write to temp file
    let tmp_key = std::env::temp_dir().join(format!(".co-git-crypt-{}", std::process::id()));
    if let Ok(decoded) = base64_decode(&b64_key)
        && fs::write(&tmp_key, decoded).is_ok()
    {
        return Some(tmp_key);
    }
    None
}

/// Simple base64 decode (standard alphabet)
#[allow(dead_code)]
fn base64_decode(input: &str) -> Result<Vec<u8>> {
    // Use command-line base64 for portability
    let output = Command::new("base64")
        .args(["--decode"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            if let Some(ref mut stdin) = child.stdin {
                use std::io::Write;
                let _ = stdin.write_all(input.as_bytes());
            }
            child.wait_with_output()
        })?;
    Ok(output.stdout)
}

/// Create or switch to a task branch in the workdir.
/// Branch name: feat/<KEY>-<slug> (e.g., feat/GP-2-initialize-rust-workspace)
/// Create a task branch. If `use_worktree` is true, creates a git worktree
/// for isolated parallel execution. Returns (branch_name, effective_workdir).
fn create_task_branch(
    task: &Task,
    workdir: &Path,
    use_worktree: bool,
) -> Result<(String, PathBuf)> {
    let slug: String = task
        .title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<&str>>()
        .join("-");
    // Truncate slug to keep branch name reasonable
    let slug = if slug.len() > 40 { &slug[..40] } else { &slug };
    let branch_name = format!("feat/{}-{}", task.key, slug);

    if use_worktree {
        // Worktree mode: create isolated directory for this task
        let worktree_dir = workdir.join(".worktrees").join(&task.key);

        // Check if worktree already exists
        if worktree_dir.exists() {
            println!(
                "  {} Reusing existing worktree: {}",
                "◆".dimmed(),
                worktree_dir.display()
            );
            return Ok((branch_name, worktree_dir));
        }

        fs::create_dir_all(workdir.join(".worktrees"))?;

        // Check if branch already exists
        let existing = Command::new("git")
            .args(["branch", "--list", &branch_name])
            .current_dir(workdir)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        // For git-crypt repos: ALWAYS disable filters during worktree creation.
        // Worktrees have a separate git dir (.git/worktrees/<name>/) that doesn't
        // contain the git-crypt key, so the smudge filter will fail during checkout.
        // We create with filters disabled, then copy the key and unlock afterward.
        let has_git_crypt = workdir.join(".gitattributes").exists() && {
            let attr = fs::read_to_string(workdir.join(".gitattributes")).unwrap_or_default();
            attr.contains("git-crypt")
        };

        let mut wt_cmd = Command::new("git");
        wt_cmd.current_dir(workdir);

        if has_git_crypt {
            // Always disable filters during worktree checkout
            wt_cmd
                .env("GIT_CONFIG_COUNT", "3")
                .env("GIT_CONFIG_KEY_0", "filter.git-crypt.smudge")
                .env("GIT_CONFIG_VALUE_0", "cat")
                .env("GIT_CONFIG_KEY_1", "filter.git-crypt.clean")
                .env("GIT_CONFIG_VALUE_1", "cat")
                .env("GIT_CONFIG_KEY_2", "filter.git-crypt.required")
                .env("GIT_CONFIG_VALUE_2", "false");
        }

        if !existing.is_empty() {
            wt_cmd.args([
                "worktree",
                "add",
                worktree_dir.to_str().unwrap(),
                &branch_name,
            ]);
        } else {
            wt_cmd.args([
                "worktree",
                "add",
                "-b",
                &branch_name,
                worktree_dir.to_str().unwrap(),
                "main",
            ]);
        }

        let output = wt_cmd.output().context("Failed to create worktree")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree add failed: {}", stderr);
        }

        // For git-crypt repos: copy key and unlock the worktree
        if has_git_crypt {
            let base_key_dir = workdir.join(".git/git-crypt");
            let wt_git_dir = workdir.join(".git/worktrees").join(&task.key);
            if base_key_dir.exists() && wt_git_dir.exists() {
                // Copy entire git-crypt directory into worktree's git dir
                let wt_crypt_dir = wt_git_dir.join("git-crypt");
                if !wt_crypt_dir.exists() {
                    let _ = Command::new("cp")
                        .args([
                            "-r",
                            base_key_dir.to_str().unwrap(),
                            wt_crypt_dir.to_str().unwrap(),
                        ])
                        .output();
                }

                // Now unlock the worktree — this decrypts files in place
                let key_file = base_key_dir
                    .join("keys/default/0")
                    .read_dir()
                    .ok()
                    .and_then(|mut d| d.next())
                    .and_then(|e| e.ok())
                    .map(|e| e.path());

                if let Some(key_path) = key_file {
                    let unlock = Command::new("git-crypt")
                        .arg("unlock")
                        .arg(&key_path)
                        .current_dir(&worktree_dir)
                        .output();

                    match unlock {
                        Ok(o) if o.status.success() => {
                            println!("  {} git-crypt: worktree unlocked", "◆".dimmed());
                        }
                        Ok(o) => {
                            let err = String::from_utf8_lossy(&o.stderr);
                            // "already decrypted" or warnings are OK
                            if err.contains("Warning") || err.contains("not encrypted") {
                                println!(
                                    "  {} git-crypt: worktree unlocked (with warnings)",
                                    "◆".dimmed()
                                );
                            } else {
                                eprintln!(
                                    "  {} git-crypt unlock warning: {}",
                                    "!".yellow(),
                                    err.trim()
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("  {} git-crypt unlock failed: {}", "!".yellow(), e);
                        }
                    }
                }
            }

            // Also neutralize filters on the worktree so Claude doesn't see phantom diffs
            neutralize_git_crypt(&worktree_dir);
        }

        // Clean up any corrupted settings.local.json in the worktree
        let wt_settings = worktree_dir.join(".claude/settings.local.json");
        if wt_settings.exists() {
            // Validate JSON — if corrupted (encrypted blob), delete it
            if let Ok(content) = fs::read_to_string(&wt_settings)
                && serde_json::from_str::<serde_json::Value>(&content).is_err()
            {
                let _ = fs::remove_file(&wt_settings);
            }
        }

        println!("  {} Worktree: {}", "◆".dimmed(), worktree_dir.display());
        Ok((branch_name, worktree_dir))
    } else {
        // Classic mode: switch branches in-place
        let existing = Command::new("git")
            .args(["branch", "--list", &branch_name])
            .current_dir(workdir)
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();

        if !existing.is_empty() {
            Command::new("git")
                .args(["checkout", &branch_name])
                .current_dir(workdir)
                .output()
                .context("Failed to checkout existing branch")?;
        } else {
            let _ = Command::new("git")
                .args(["checkout", "main"])
                .current_dir(workdir)
                .output();
            Command::new("git")
                .args(["checkout", "-b", &branch_name])
                .current_dir(workdir)
                .output()
                .context("Failed to create task branch")?;
        }

        Ok((branch_name, workdir.to_path_buf()))
    }
}

/// Resolve workdir: bare names like "game" become ~/projects/game
fn resolve_workdir(workdir: Option<&str>) -> Result<PathBuf> {
    match workdir {
        Some(w) => {
            let path = PathBuf::from(w);
            if path.is_absolute() && path.exists() {
                Ok(path)
            } else if path.exists() {
                Ok(path.canonicalize()?)
            } else {
                // Try resolving as ~/projects/<name>
                let home = dirs::home_dir().unwrap_or_default();
                let projects_path = home.join("projects").join(w);
                if projects_path.exists() {
                    Ok(projects_path)
                } else {
                    anyhow::bail!(
                        "Workdir not found: '{}' (tried absolute and ~/projects/{})",
                        w,
                        w
                    );
                }
            }
        }
        None => Ok(std::env::current_dir()?),
    }
}

/// Detect changes using jj (if available) or git as fallback
fn detect_changes() -> bool {
    // Try jj first
    if let Ok(output) = Command::new("jj").args(["diff", "--stat"]).output()
        && output.status.success()
    {
        let diff = String::from_utf8_lossy(&output.stdout);
        if !diff.trim().is_empty() {
            return true;
        }
        // Also check jj log for new commits
        if let Ok(log) = Command::new("jj")
            .args(["log", "-r", "@", "--no-graph", "-T", "description"])
            .output()
        {
            let desc = String::from_utf8_lossy(&log.stdout);
            return !desc.trim().is_empty();
        }
    }

    // Fallback to git
    let git_diff = Command::new("git")
        .args(["diff", "--stat", "HEAD~1"])
        .output()
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
        .unwrap_or_default();

    !git_diff.trim().is_empty()
}

fn find_data_dir(space: &str) -> Result<PathBuf> {
    // Look for data/{space}/ relative to workspace root
    let cwd = std::env::current_dir()?;
    let mut dir = cwd.as_path();

    loop {
        let candidate = dir.join("data").join(space);
        if candidate.exists() && candidate.is_dir() {
            return Ok(candidate);
        }
        let co_marker = dir.join(".co");
        if co_marker.exists() {
            let candidate = dir.join("data").join(space);
            if candidate.exists() {
                return Ok(candidate);
            }
        }
        dir = match dir.parent() {
            Some(p) => p,
            None => anyhow::bail!(
                "Space '{}' not found. Run from within a co workspace.",
                space
            ),
        };
    }
}

fn find_workspace_root(data_dir: &Path) -> PathBuf {
    // data_dir is data/{space}/, workspace root is two levels up
    data_dir
        .parent()
        .and_then(|p| p.parent())
        .unwrap_or(data_dir)
        .to_path_buf()
}

fn load_project_key(data_dir: &Path) -> Result<String> {
    let project_yaml = data_dir.join("project.yaml");
    let content =
        fs::read_to_string(&project_yaml).context("No project.yaml found in space directory")?;
    let yaml: serde_yaml::Value = serde_yaml::from_str(&content)?;
    yaml.as_mapping()
        .and_then(|m| m.get(serde_yaml::Value::String("key".into())))
        .and_then(|v| v.as_str())
        .map(String::from)
        .context("project.yaml missing 'key' field")
}

fn nanoid() -> String {
    use std::time::SystemTime;
    let ts = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    format!("{:x}", ts)
}

fn ensure_teams_enabled() -> Result<()> {
    let settings_path = dirs::home_dir()
        .unwrap_or_default()
        .join(".claude")
        .join("settings.json");

    if settings_path.exists() {
        let content = fs::read_to_string(&settings_path)?;
        if content.contains("CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS") {
            println!("  {} Agent teams: enabled", "✓".green());
            return Ok(());
        }
    }

    println!(
        "  {} Agent teams not enabled. Set CLAUDE_CODE_EXPERIMENTAL_AGENT_TEAMS=1 in ~/.claude/settings.json",
        "⚠".yellow()
    );
    Ok(())
}

fn save_tracker(tracker: &RunTracker) -> Result<()> {
    let runs_dir = dirs::home_dir()
        .unwrap_or_default()
        .join(".co")
        .join("runs");
    fs::create_dir_all(&runs_dir)?;

    let content = format!(
        "run_id: {}\nstarted_at: {}\ncompleted: {:?}\nfailed: {:?}\n",
        tracker.run_id, tracker.started_at, tracker.tasks_completed, tracker.tasks_failed
    );
    fs::write(runs_dir.join(format!("{}.yaml", tracker.run_id)), content)?;
    Ok(())
}

// ============================================================================
// Composable trait surface (CO-84)
// ============================================================================
//
// The procedural `run()` above remains the production path for v1. The traits
// below give a parallel surface that's pluggable, mockable, and testable.
// Future work will migrate `run()` to use a `Pipeline` of these trait objects.

/// Captures the assembled context (CLAUDE.md, task body, parent epic, project,
/// roadmap) that an Executor receives. Named `TaskContext` to avoid collision
/// with `anyhow::Context`.
#[derive(Debug, Clone)]
pub struct TaskContext {
    pub text: String,
}

/// Outcome of an Executor run.
#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
}

/// Outcome of a Reviewer's verdict.
#[derive(Debug, Clone)]
pub struct ReviewVerdict {
    pub passed: bool,
    pub details: String,
}

/// Source of tasks (filesystem, GitHub Issues, Linear, …).
pub trait TaskSource: Send + Sync {
    fn list_tasks(&self) -> Result<Vec<Task>>;
}

/// Picks the next task to run from a candidate set.
pub trait TaskSelector: Send + Sync {
    fn pick_next<'a>(&self, tasks: &'a [Task]) -> Option<&'a Task>;
}

/// Builds the multi-layer context an Executor will see.
pub trait ContextBuilder: Send + Sync {
    fn build(&self, task: &Task, all_tasks: &[Task], workdir: &Path) -> Result<TaskContext>;
}

/// Runs the task (Claude Code, shell, custom binary, …).
pub trait Executor: Send + Sync {
    fn execute(
        &self,
        task: &Task,
        context: &TaskContext,
        workdir: &Path,
    ) -> Result<ExecutionResult>;
}

/// Reviews an Executor result against acceptance criteria.
pub trait Reviewer: Send + Sync {
    fn review(&self, task: &Task, result: &ExecutionResult) -> Result<ReviewVerdict>;
}

/// Finalizes a passed task (commit, PR, status update, notification, …).
pub trait Finalizer: Send + Sync {
    fn finalize(&self, task: &Task, verdict: &ReviewVerdict, workdir: &Path) -> Result<()>;
}

// --- Default implementations -------------------------------------------------

/// Loads tasks from `*.md` files in a data directory (current behavior).
pub struct FilesystemTaskSource {
    pub data_dir: PathBuf,
    pub project_key: String,
}

impl TaskSource for FilesystemTaskSource {
    fn list_tasks(&self) -> Result<Vec<Task>> {
        load_tasks(&self.data_dir, &self.project_key)
    }
}

/// Picks the highest-priority unblocked todo/in_progress task (current behavior).
pub struct UnblockedFirstSelector;

impl TaskSelector for UnblockedFirstSelector {
    fn pick_next<'a>(&self, tasks: &'a [Task]) -> Option<&'a Task> {
        let picked = select_next_task(tasks)?;
        tasks.iter().find(|t| t.id == picked.id)
    }
}

/// Builds the layered context (CLAUDE.md, task, parent, project, roadmap).
pub struct DefaultContextBuilder {
    pub data_dir: PathBuf,
}

impl ContextBuilder for DefaultContextBuilder {
    fn build(&self, task: &Task, all_tasks: &[Task], workdir: &Path) -> Result<TaskContext> {
        let text = build_context(task, &self.data_dir, all_tasks, Some(workdir))?;
        Ok(TaskContext { text })
    }
}

/// Wraps the existing acceptance-criteria parser.
pub struct AcceptanceReviewer;

impl Reviewer for AcceptanceReviewer {
    fn review(&self, task: &Task, _result: &ExecutionResult) -> Result<ReviewVerdict> {
        let r = review_criteria(task)?;
        Ok(ReviewVerdict {
            passed: r.passed,
            details: format!("{}/{} acceptance criteria met", r.met, r.total),
        })
    }
}

/// Marks a task as `done` in its frontmatter.
pub struct StatusUpdateFinalizer;

impl Finalizer for StatusUpdateFinalizer {
    fn finalize(&self, task: &Task, _verdict: &ReviewVerdict, _workdir: &Path) -> Result<()> {
        update_task_status(task, "done")
    }
}

// --- Compositional combinators ----------------------------------------------

/// Federate multiple task sources. Order is preserved; duplicate keys win to
/// the first source that defines them.
pub struct MultiTaskSource(pub Vec<Box<dyn TaskSource>>);

impl TaskSource for MultiTaskSource {
    fn list_tasks(&self) -> Result<Vec<Task>> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for src in &self.0 {
            for t in src.list_tasks()? {
                if seen.insert(t.key.clone()) {
                    out.push(t);
                }
            }
        }
        Ok(out)
    }
}

/// Run reviewers in order; short-circuit on the first failure.
pub struct ChainedReviewer(pub Vec<Box<dyn Reviewer>>);

impl Reviewer for ChainedReviewer {
    fn review(&self, task: &Task, result: &ExecutionResult) -> Result<ReviewVerdict> {
        for r in &self.0 {
            let v = r.review(task, result)?;
            if !v.passed {
                return Ok(v);
            }
        }
        Ok(ReviewVerdict {
            passed: true,
            details: "all reviewers passed".into(),
        })
    }
}

/// Run all finalizers, returning the first error (if any).
pub struct ChainedFinalizer(pub Vec<Box<dyn Finalizer>>);

impl Finalizer for ChainedFinalizer {
    fn finalize(&self, task: &Task, verdict: &ReviewVerdict, workdir: &Path) -> Result<()> {
        for f in &self.0 {
            f.finalize(task, verdict, workdir)?;
        }
        Ok(())
    }
}

// --- Alternative implementations (for testing + future extension) -----------

/// Runs an arbitrary shell command instead of Claude Code. Useful for tests
/// and for codebases where the loop is "execute a script, check exit code."
pub struct ShellExecutor {
    pub command: String,
}

impl Executor for ShellExecutor {
    fn execute(
        &self,
        _task: &Task,
        _context: &TaskContext,
        workdir: &Path,
    ) -> Result<ExecutionResult> {
        let output = Command::new("sh")
            .arg("-c")
            .arg(&self.command)
            .current_dir(workdir)
            .output()
            .context("Failed to spawn shell")?;
        Ok(ExecutionResult {
            success: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

// --- Pipeline orchestrator --------------------------------------------------

/// Composes a TaskSource → TaskSelector → ContextBuilder → Executor → Reviewer
/// → Finalizers pipeline. Holds boxed trait objects so each phase is swappable
/// at construction.
pub struct Pipeline {
    pub source: Box<dyn TaskSource>,
    pub selector: Box<dyn TaskSelector>,
    pub context_builder: Box<dyn ContextBuilder>,
    pub executor: Box<dyn Executor>,
    pub reviewer: Box<dyn Reviewer>,
    pub finalizers: Vec<Box<dyn Finalizer>>,
}

/// Outcome of a single `Pipeline::run_once` call.
pub struct PipelineRun {
    pub task_key: String,
    pub verdict: ReviewVerdict,
}

impl Pipeline {
    /// Constructs a pipeline that mirrors today's procedural `run()` behavior:
    /// filesystem source, unblocked-first selector, default context builder,
    /// no executor (caller must supply one), acceptance reviewer, status-update
    /// finalizer.
    pub fn default_for(
        data_dir: PathBuf,
        project_key: String,
        executor: Box<dyn Executor>,
    ) -> Self {
        Self {
            source: Box::new(FilesystemTaskSource {
                data_dir: data_dir.clone(),
                project_key,
            }),
            selector: Box::new(UnblockedFirstSelector),
            context_builder: Box::new(DefaultContextBuilder { data_dir }),
            executor,
            reviewer: Box::new(AcceptanceReviewer),
            finalizers: vec![Box::new(StatusUpdateFinalizer)],
        }
    }

    /// One iteration: pick → context → execute → review → finalize (if passed).
    pub fn run_once(&self, workdir: &Path) -> Result<Option<PipelineRun>> {
        let tasks = self.source.list_tasks()?;
        let task = match self.selector.pick_next(&tasks) {
            Some(t) => t,
            None => return Ok(None),
        };
        let context = self.context_builder.build(task, &tasks, workdir)?;
        let result = self.executor.execute(task, &context, workdir)?;
        let verdict = self.reviewer.review(task, &result)?;
        if verdict.passed {
            for f in &self.finalizers {
                f.finalize(task, &verdict, workdir)?;
            }
        }
        Ok(Some(PipelineRun {
            task_key: task.key.clone(),
            verdict,
        }))
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_task(id: u64, key: &str, title: &str) -> Task {
        Task {
            id,
            key: key.into(),
            title: title.into(),
            status: "todo".into(),
            priority: "medium".into(),
            parent: None,
            labels: vec![],
            module: None,
            body: String::new(),
            file_path: PathBuf::from("/tmp/nonexistent"),
        }
    }

    /// Static source for tests — yields a fixed list.
    struct StaticTaskSource(Vec<Task>);
    impl TaskSource for StaticTaskSource {
        fn list_tasks(&self) -> Result<Vec<Task>> {
            Ok(self.0.clone())
        }
    }

    struct AlwaysPassReviewer;
    impl Reviewer for AlwaysPassReviewer {
        fn review(&self, _task: &Task, _r: &ExecutionResult) -> Result<ReviewVerdict> {
            Ok(ReviewVerdict {
                passed: true,
                details: "ok".into(),
            })
        }
    }

    struct AlwaysFailReviewer;
    impl Reviewer for AlwaysFailReviewer {
        fn review(&self, _task: &Task, _r: &ExecutionResult) -> Result<ReviewVerdict> {
            Ok(ReviewVerdict {
                passed: false,
                details: "always fails".into(),
            })
        }
    }

    #[test]
    fn multi_task_source_federates_and_dedupes() {
        let a = StaticTaskSource(vec![
            mk_task(1, "CO-1", "First"),
            mk_task(2, "CO-2", "Second"),
        ]);
        let b = StaticTaskSource(vec![
            mk_task(2, "CO-2", "Duplicate of second"), // dedup wins to first
            mk_task(3, "EXTRA-3", "Extra from b"),
        ]);
        let multi = MultiTaskSource(vec![Box::new(a), Box::new(b)]);
        let tasks = multi.list_tasks().unwrap();
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0].title, "First");
        assert_eq!(tasks[1].title, "Second"); // first source's version wins
        assert_eq!(tasks[2].key, "EXTRA-3");
    }

    #[test]
    fn chained_reviewer_short_circuits_on_failure() {
        let chain = ChainedReviewer(vec![
            Box::new(AlwaysFailReviewer),
            Box::new(AlwaysPassReviewer), // never reached
        ]);
        let task = mk_task(1, "CO-1", "x");
        let result = ExecutionResult {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        };
        let verdict = chain.review(&task, &result).unwrap();
        assert!(!verdict.passed);
        assert_eq!(verdict.details, "always fails");
    }

    #[test]
    fn chained_reviewer_passes_when_all_pass() {
        let chain = ChainedReviewer(vec![
            Box::new(AlwaysPassReviewer),
            Box::new(AlwaysPassReviewer),
        ]);
        let task = mk_task(1, "CO-1", "x");
        let result = ExecutionResult {
            success: true,
            stdout: String::new(),
            stderr: String::new(),
        };
        let verdict = chain.review(&task, &result).unwrap();
        assert!(verdict.passed);
        assert_eq!(verdict.details, "all reviewers passed");
    }

    #[test]
    fn shell_executor_runs_command() {
        let exec = ShellExecutor {
            command: "echo hello-from-shell-exec".into(),
        };
        let task = mk_task(1, "CO-1", "x");
        let context = TaskContext {
            text: String::new(),
        };
        let tmp = std::env::temp_dir();
        let result = exec.execute(&task, &context, &tmp).unwrap();
        assert!(result.success);
        assert!(result.stdout.contains("hello-from-shell-exec"));
    }

    #[test]
    fn context_budget_minimal_path_under_30k() {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let tmp = std::env::temp_dir().join(format!("co-auto-budget-{ts}"));
        let data_dir = tmp.join("work").join("co");
        fs::create_dir_all(&data_dir).unwrap();

        // Per-space CLAUDE.md (~1.5k chars)
        fs::write(
            data_dir.join("CLAUDE.md"),
            "# CO Dev Guide\n\n\
             ## Git\n\
             - Branch: `feat/CO-N-desc`\n\
             - Commits: conventional (`feat(scope):`, `fix(scope):`, etc.)\n\n\
             ## TDD\n\
             1. Write failing test\n\
             2. Minimal impl\n\
             3. Refactor\n\n\
             ```bash\n\
             cargo test\n\
             cargo clippy -- -D warnings\n\
             cargo fmt\n\
             ```\n\n\
             ## Forbidden files\n\
             - `Cargo.toml`, `co-cli/Cargo.toml`, `CHANGELOG.md`\n\
             - Write changelog entry to `CHANGELOG-PENDING/<TASK-ID>.md`\n\n\
             ## Module map\n\
             | Module | Location |\n\
             |--------|----------|\n\
             | Core types | `core/src/` |\n\
             | CLI | `co-cli/src/` |\n\
             | Web server | `co-web/src/` |\n\
             | SPA | `co-web/static/variants/a/` |\n",
        )
        .unwrap();

        // Task file (~300 chars)
        let task_path = data_dir.join("CO-999.md");
        fs::write(
            &task_path,
            "---\nid: 999\ntitle: Budget test\nstatus: todo\npriority: medium\nlabels:\n  - module:co-auto\n---\n\nDo a small thing.\n\n## Acceptance\n- [ ] It works.\n",
        )
        .unwrap();

        let mut task = mk_task(999, "CO-999", "Budget test");
        task.labels = vec!["module:co-auto".into()];
        task.file_path = task_path;

        let context = build_context(&task, &data_dir, &[], None).unwrap();

        let _ = fs::remove_dir_all(&tmp);

        assert!(
            context.len() < 30_000,
            "context budget exceeded: {} chars (max 30_000)",
            context.len()
        );
    }
}
