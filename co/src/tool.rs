use std::collections::HashMap;

/// A tool that can be invoked by modules during pipeline execution.
pub trait Tool: Send + Sync {
    /// Tool identifier
    fn name(&self) -> &str;

    /// Human-readable description
    fn description(&self) -> &str;

    /// Check if this tool is available (e.g., CLI binary exists)
    fn available(&self) -> bool;

    /// Run the tool with an action and parameters
    fn run(&self, action: &str, params: &HashMap<String, String>) -> anyhow::Result<String>;
}

/// Registry of available tools
pub struct ToolRegistry {
    tools: Vec<Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self { tools: Vec::new() }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.push(tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.iter().find(|t| t.name() == name).map(|t| &**t)
    }

    pub fn available_tools(&self) -> Vec<&str> {
        self.tools
            .iter()
            .filter(|t| t.available())
            .map(|t| t.name())
            .collect()
    }

    pub fn check_requirements(&self, required: &[&str]) -> Result<(), Vec<String>> {
        let missing: Vec<String> = required
            .iter()
            .filter(|name| self.get(name).map(|t| !t.available()).unwrap_or(true))
            .map(|s| s.to_string())
            .collect();

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}
