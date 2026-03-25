use co_engine::config::EngineConfig;
use co_engine::context::{Context, KnowledgeEntry, Stage};
use co_engine::engine::{Engine, LoggedToolRegistry};
use co_engine::module::{Module, ModuleRegistry};
use co_engine::tool::{Tool, ToolRegistry};
use std::collections::HashMap;

// --- Context tests ---

#[test]
fn test_context_creation() {
    let ctx = Context::new("owner/repo", "Add login", "We need OAuth2 support");
    assert_eq!(ctx.repo, "owner/repo");
    assert_eq!(ctx.title, "Add login");
    assert_eq!(ctx.stage, Stage::Plan);
    assert!(ctx.artifacts.is_empty());
    assert!(!ctx.run_id.is_empty());
}

#[test]
fn test_context_prompt_rendering() {
    let mut ctx = Context::new("owner/repo", "Fix bug", "The widget crashes");
    ctx.knowledge.push(KnowledgeEntry {
        source: "notes/widgets.md".into(),
        content: "Widgets use React hooks".into(),
        tags: vec!["#frontend".into()],
    });
    ctx.artifacts.insert(
        "issue_url".into(),
        "https://github.com/owner/repo/issues/1".into(),
    );

    let prompt = ctx.to_prompt_context();
    assert!(prompt.contains("Fix bug"));
    assert!(prompt.contains("owner/repo"));
    assert!(prompt.contains("Widgets use React hooks"));
    assert!(prompt.contains("issue_url"));
}

// --- Tool registry tests ---

struct MockTool {
    name: String,
    is_available: bool,
}

impl Tool for MockTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn description(&self) -> &str {
        "mock tool"
    }
    fn available(&self) -> bool {
        self.is_available
    }
    fn run(&self, action: &str, _params: &HashMap<String, String>) -> anyhow::Result<String> {
        Ok(format!("mock:{}:{}", self.name, action))
    }
}

#[test]
fn test_tool_registry() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool {
        name: "alpha".into(),
        is_available: true,
    }));
    registry.register(Box::new(MockTool {
        name: "beta".into(),
        is_available: false,
    }));

    assert!(registry.get("alpha").is_some());
    assert!(registry.get("beta").is_some());
    assert!(registry.get("gamma").is_none());

    let available = registry.available_tools();
    assert_eq!(available, vec!["alpha"]);
}

#[test]
fn test_tool_requirements_check() {
    let mut registry = ToolRegistry::new();
    registry.register(Box::new(MockTool {
        name: "github".into(),
        is_available: true,
    }));
    registry.register(Box::new(MockTool {
        name: "claude".into(),
        is_available: false,
    }));

    assert!(registry.check_requirements(&["github"]).is_ok());
    assert!(registry.check_requirements(&["claude"]).is_err());
    assert!(registry.check_requirements(&["nonexistent"]).is_err());

    let missing = registry
        .check_requirements(&["claude", "nonexistent"])
        .unwrap_err();
    assert_eq!(missing.len(), 2);
}

// --- Module registry tests ---

struct MockModule {
    stage: Stage,
}

impl Module for MockModule {
    fn name(&self) -> &str {
        "mock"
    }
    fn stage(&self) -> Stage {
        self.stage
    }
    fn required_tools(&self) -> Vec<&str> {
        vec![]
    }
    fn run(&self, ctx: &mut Context, _tools: &LoggedToolRegistry) -> anyhow::Result<()> {
        ctx.artifacts
            .insert("mock_ran".into(), ctx.stage.to_string());
        Ok(())
    }
}

#[test]
fn test_module_registry() {
    let mut registry = ModuleRegistry::new();
    registry.register(Box::new(MockModule { stage: Stage::Plan }));

    assert!(registry.get_for_stage(Stage::Plan).is_some());
    assert!(registry.get_for_stage(Stage::Execute).is_none());
}

// --- Engine tests ---

#[test]
fn test_engine_run_stage_no_module() {
    let config = EngineConfig::default();
    let engine = Engine::new(config);

    let mut ctx = Context::new("owner/repo", "Test", "test");
    let result = engine.run_stage(&mut ctx, Stage::Plan);
    assert!(result.is_err());
}

#[test]
fn test_engine_run_stage_with_mock() {
    let config = EngineConfig::default();
    let mut engine = Engine::new(config);

    engine
        .modules
        .register(Box::new(MockModule { stage: Stage::Plan }));

    let mut ctx = Context::new("owner/repo", "Test", "test");
    let log = engine.run_stage(&mut ctx, Stage::Plan).unwrap();

    assert_eq!(ctx.artifacts.get("mock_ran").unwrap(), "plan");
    assert_eq!(ctx.stage, Stage::Plan);
    assert_eq!(log.stages.len(), 1);
    assert!(log.stages[0].success);
}

// --- Config tests ---

#[test]
fn test_default_config() {
    let config = EngineConfig::default();
    assert!(config.vault_path.is_none());
    assert!(!config.auto_approve);
    assert_eq!(config.claude_model, "claude-sonnet-4-20250514");
}

// --- Stage display ---

#[test]
fn test_stage_display() {
    assert_eq!(format!("{}", Stage::Plan), "plan");
    assert_eq!(format!("{}", Stage::Execute), "execute");
    assert_eq!(format!("{}", Stage::Approve), "approve");
}
