use crate::context::{Context, Stage};
use crate::engine::LoggedToolRegistry;

/// A module handles one stage of the pipeline.
pub trait Module: Send + Sync {
    /// Module identifier
    fn name(&self) -> &str;

    /// Which pipeline stage this module handles
    fn stage(&self) -> Stage;

    /// Tools required by this module
    fn required_tools(&self) -> Vec<&str>;

    /// Execute the module, mutating context with results
    fn run(&self, ctx: &mut Context, tools: &LoggedToolRegistry) -> anyhow::Result<()>;
}

/// Registry of loaded modules
pub struct ModuleRegistry {
    modules: Vec<Box<dyn Module>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self {
            modules: Vec::new(),
        }
    }

    pub fn register(&mut self, module: Box<dyn Module>) {
        self.modules.push(module);
    }

    pub fn get_for_stage(&self, stage: Stage) -> Option<&dyn Module> {
        self.modules
            .iter()
            .find(|m| m.stage() == stage)
            .map(|m| &**m)
    }

    pub fn list(&self) -> Vec<(&str, Stage)> {
        self.modules.iter().map(|m| (m.name(), m.stage())).collect()
    }
}

impl Default for ModuleRegistry {
    fn default() -> Self {
        Self::new()
    }
}
