use colored::Colorize;
use game_core::{GameError, Result};
use obfstr::obfstr;
use std::env;
use std::process::Command;

pub fn run_update() -> Result<()> {
    println!("\n{}", "Updating game...".cyan().bold());
    println!();

    // Get the current directory
    let cwd = env::current_dir()?;

    // Step 1: Check if running from git repository
    println!("{} Checking for git repository", "[1/5]".blue());
    let status = Command::new("git").arg("status").current_dir(&cwd).status();

    match status {
        Ok(stat) if stat.success() => {
            println!(
                "     {} Found git repository at {}",
                "✓".green(),
                cwd.display()
            );
        }
        _ => {
            println!("     {} Not in git repository. Skipping update.", "✗".red());
            return Err(GameError::Command(
                obfstr!("Not in a git repository").into(),
            ));
        }
    }

    println!();

    // Step 2: Checkout main branch
    println!("{} Checking out main branch", "[2/5]".blue());
    let checkout = Command::new("git")
        .args(["checkout", "main"])
        .current_dir(&cwd)
        .output()?;

    if checkout.status.success() {
        println!("     {} On branch main", "✓".green());
    } else {
        println!(
            "     {} Failed to checkout: {}",
            "✗".red(),
            String::from_utf8_lossy(&checkout.stderr)
        );
        return Err(GameError::Command("Failed to checkout main branch".into()));
    }

    println!();

    // Step 3: Pull latest changes
    println!("{} Pulling latest changes", "[3/5]".blue());
    let pull = Command::new("git")
        .args(["pull", "origin", "main"])
        .current_dir(&cwd)
        .output()?;

    if pull.status.success() {
        let output = String::from_utf8_lossy(&pull.stdout);
        if output.contains("Already up to date") {
            println!("     {} Already up to date", "✓".green());
        } else {
            println!("     {} Pulled latest changes", "✓".green());
        }
    } else {
        println!(
            "     {} Failed to pull: {}",
            "✗".red(),
            String::from_utf8_lossy(&pull.stderr)
        );
        return Err(GameError::Command("Failed to pull changes".into()));
    }

    println!();

    // Step 4: Build release binary
    println!("{} Building release binary", "[4/5]".blue());
    let build = Command::new(obfstr!("cargo"))
        .args([
            obfstr!("build"),
            obfstr!("--release"),
            obfstr!("--bin"),
            obfstr!("game"),
        ])
        .current_dir(&cwd)
        .output()?;

    if build.status.success() {
        println!("     {} Build complete", "✓".green());
    } else {
        println!(
            "     {} Build failed: {}",
            "✗".red(),
            String::from_utf8_lossy(&build.stderr)
        );
        return Err(GameError::Command("Build failed".into()));
    }

    println!();

    // Step 5: Install to ~/.cargo/bin/
    println!("{} Installing", "[5/5]".blue());
    let install = Command::new(obfstr!("cargo"))
        .args([obfstr!("install"), obfstr!("--path"), "."])
        .current_dir(&cwd)
        .output()?;

    if install.status.success() {
        println!("     {} Game updated successfully", "✓".green());
    } else {
        println!(
            "     {} Installation failed: {}",
            "✗".red(),
            String::from_utf8_lossy(&install.stderr)
        );
        return Err(GameError::Command("Installation failed".into()));
    }

    println!();
    println!(
        "{}",
        "Game is ready to use! Run 'game start' to play."
            .green()
            .bold()
    );

    Ok(())
}
