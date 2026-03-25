use crate::config::EngineConfig;
use crate::context::{Context, Stage};
use crate::module::ModuleRegistry;
use crate::telemetry::RunLog;
use crate::tool::ToolRegistry;
use std::path::PathBuf;

/// The engine orchestrates pipeline execution across modules.
pub struct Engine {
    pub config: EngineConfig,
    pub tools: ToolRegistry,
    pub modules: ModuleRegistry,
    logs_dir: PathBuf,
}

impl Engine {
    pub fn new(config: EngineConfig) -> Self {
        let logs_dir = config.logs_dir.clone().unwrap_or_else(default_logs_dir);
        Self {
            config,
            tools: ToolRegistry::new(),
            modules: ModuleRegistry::new(),
            logs_dir,
        }
    }

    /// Run a single pipeline stage with telemetry
    pub fn run_stage(&self, ctx: &mut Context, stage: Stage) -> anyhow::Result<RunLog> {
        let mut log = RunLog::new(&ctx.run_id, &ctx.repo, &ctx.title, &ctx.description);
        self.run_stage_logged(ctx, stage, &mut log)?;
        log.finalize(&ctx.artifacts, None);

        let path = log.save(&self.logs_dir)?;
        println!("[engine] Log saved: {}", path.display());

        Ok(log)
    }

    /// Internal: run a stage and record into existing log
    fn run_stage_logged(
        &self,
        ctx: &mut Context,
        stage: Stage,
        log: &mut RunLog,
    ) -> anyhow::Result<()> {
        let module = self
            .modules
            .get_for_stage(stage)
            .ok_or_else(|| anyhow::anyhow!("No module registered for stage: {}", stage))?;

        let required = module.required_tools();
        if let Err(missing) = self.tools.check_requirements(&required) {
            anyhow::bail!(
                "Module '{}' requires unavailable tools: {}",
                module.name(),
                missing.join(", ")
            );
        }

        ctx.stage = stage;
        let stage_idx = log.start_stage(&stage.to_string(), module.name());

        println!(
            "[engine] Running stage: {} (module: {})",
            stage,
            module.name()
        );

        let logged_tools = LoggedToolRegistry {
            inner: &self.tools,
            log: std::cell::RefCell::new(log),
            stage_idx,
        };

        let result = module.run(ctx, &logged_tools);

        let log = logged_tools.log.into_inner();

        // Record which artifacts this stage produced
        let artifact_keys: Vec<String> = ctx.artifacts.keys().cloned().collect();
        if let Some(stage_record) = log.stages.get_mut(stage_idx) {
            stage_record.artifacts_produced = artifact_keys;
        }

        match &result {
            Ok(()) => {
                log.finish_stage(stage_idx, true, None);
                println!("[engine] Stage {} complete", stage);
            }
            Err(e) => {
                log.finish_stage(stage_idx, false, Some(e.to_string()));
                println!("[engine] Stage {} failed: {}", stage, e);
            }
        }

        result
    }

    /// Run the full pipeline: plan → execute → approve
    pub fn run_auto(&self, ctx: &mut Context) -> anyhow::Result<RunLog> {
        let mut log = RunLog::new(&ctx.run_id, &ctx.repo, &ctx.title, &ctx.description);

        self.run_stage_logged(ctx, Stage::Plan, &mut log)?;

        if !self.config.auto_approve {
            println!("\n[engine] Plan complete. Review artifacts before proceeding.");
            println!(
                "  Issue: {}",
                ctx.artifacts.get("issue_url").unwrap_or(&"(none)".into())
            );
            println!("\nProceed with execution? (y/n)");

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("[engine] Aborted by user.");
                log.finalize(&ctx.artifacts, Some("Aborted after plan stage".into()));
                log.save(&self.logs_dir)?;
                return Ok(log);
            }
        }

        self.run_stage_logged(ctx, Stage::Execute, &mut log)?;

        if !self.config.auto_approve {
            println!("\n[engine] Execution complete. Review PR before approving.");
            println!(
                "  PR: {}",
                ctx.artifacts.get("pr_url").unwrap_or(&"(none)".into())
            );
            println!("\nProceed with approval? (y/n)");

            let mut input = String::new();
            std::io::stdin().read_line(&mut input)?;
            if !input.trim().eq_ignore_ascii_case("y") {
                println!("[engine] Skipping approval.");
                log.finalize(&ctx.artifacts, Some("Skipped approval stage".into()));
                log.save(&self.logs_dir)?;
                return Ok(log);
            }
        }

        self.run_stage_logged(ctx, Stage::Approve, &mut log)?;

        log.finalize(&ctx.artifacts, Some("Pipeline complete".into()));
        let path = log.save(&self.logs_dir)?;
        println!("\n[engine] Pipeline complete. Log: {}", path.display());

        Ok(log)
    }

    /// Register default tools and modules
    pub fn with_defaults(mut self) -> Self {
        use crate::modules::{approve::ApproveModule, execute::ExecuteModule, plan::PlanModule};
        use crate::tools::{claude::ClaudeTool, github::GitHubTool, obsidian::ObsidianTool};

        self.tools.register(Box::new(GitHubTool));
        self.tools
            .register(Box::new(ClaudeTool::new(self.config.claude_model.clone())));
        self.tools
            .register(Box::new(ObsidianTool::new(self.config.vault_path.clone())));

        self.modules.register(Box::new(PlanModule));
        self.modules.register(Box::new(ExecuteModule));
        self.modules.register(Box::new(ApproveModule));

        self
    }
}

/// A wrapper around ToolRegistry that records every tool call into the RunLog.
/// Uses interior mutability so modules can call tools while the registry is shared.
pub struct LoggedToolRegistry<'a> {
    inner: &'a ToolRegistry,
    log: std::cell::RefCell<&'a mut RunLog>,
    stage_idx: usize,
}

impl LoggedToolRegistry<'_> {
    pub fn get(&self, name: &str) -> Option<LoggedToolHandle> {
        self.inner.get(name).map(|tool| LoggedToolHandle {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            is_available: tool.available(),
        })
    }

    /// Run a tool by name, recording the invocation in the log
    pub fn run_tool(
        &self,
        name: &str,
        action: &str,
        params: &std::collections::HashMap<String, String>,
    ) -> anyhow::Result<String> {
        let tool = self
            .inner
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("Tool not found: {}", name))?;

        let start = std::time::Instant::now();
        let result = tool.run(action, params);
        let duration_ms = start.elapsed().as_millis() as u64;

        let (output, success) = match &result {
            Ok(out) => (out.clone(), true),
            Err(e) => (e.to_string(), false),
        };

        self.log.borrow_mut().record_tool_call(
            self.stage_idx,
            name,
            action,
            params,
            &output,
            success,
            duration_ms,
        );

        result
    }
}

/// Lightweight handle returned by get() — no lifetime entanglement
pub struct LoggedToolHandle {
    pub name: String,
    pub description: String,
    pub is_available: bool,
}

impl LoggedToolHandle {
    pub fn available(&self) -> bool {
        self.is_available
    }
}

fn default_logs_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".config/co-engine/logs")
}
