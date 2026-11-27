//! Agent integration for SigmaOS.
//!
//! Agent registration, capability tracking, and command dispatch.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Agent execution result.
#[derive(Debug, Clone)]
pub struct AgentResult {
    pub output: String,
    pub exit_code: i32,
}

/// Agent descriptor.
#[derive(Debug, Clone)]
pub struct Agent {
    pub id: u32,
    pub name: String,
    pub capabilities: Vec<String>,
    pub enabled: bool,
}

/// Agent manager.
pub struct AgentManager {
    agents: Vec<Agent>,
    next_id: u32,
}

impl AgentManager {
    /// Create a new agent manager.
    pub fn new() -> Self {
        Self {
            agents: Vec::new(),
            next_id: 1,
        }
    }

    /// Register an agent with given capabilities.
    pub fn register(&mut self, name: &str, capabilities: &[String]) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        let agent = Agent {
            id,
            name: String::from(name),
            capabilities: capabilities.to_vec(),
            enabled: true,
        };
        self.agents.push(agent);
        id
    }

    /// Execute a command through an agent.
    ///
    /// Dispatches to the agent if it has the required capability.
    /// For now, this is a simulation - in a full implementation it would:
    /// - Spawn the agent in a sandbox
    /// - Send the command via IPC
    /// - Collect the result
    pub fn execute(&mut self, agent_id: u32, command: &str) -> Result<AgentResult, String> {
        let agent = self
            .agents
            .iter()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("agent {} not found", agent_id))?;

        if !agent.enabled {
            return Err(format!("agent {} is disabled", agent_id));
        }

        // Simple command dispatch simulation
        let parts: Vec<&str> = command.split_whitespace().collect();
        if parts.is_empty() {
            return Err("empty command".to_string());
        }

        let cmd = parts[0];
        let args = &parts[1..];

        match cmd {
            "status" => Ok(AgentResult {
                output: alloc::format!("Agent {} ({}) is running", agent.id, agent.name),
                exit_code: 0,
            }),
            "list-caps" => Ok(AgentResult {
                output: agent.capabilities.join(", "),
                exit_code: 0,
            }),
            "echo" => Ok(AgentResult {
                output: args.join(" "),
                exit_code: 0,
            }),
            _ => Err(format!("unknown command: {}", cmd)),
        }
    }

    /// Enable an agent.
    pub fn enable(&mut self, agent_id: u32) -> Result<(), String> {
        let agent = self
            .agents
            .iter_mut()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("agent {} not found", agent_id))?;
        agent.enabled = true;
        Ok(())
    }

    /// Disable an agent.
    pub fn disable(&mut self, agent_id: u32) -> Result<(), String> {
        let agent = self
            .agents
            .iter_mut()
            .find(|a| a.id == agent_id)
            .ok_or_else(|| format!("agent {} not found", agent_id))?;
        agent.enabled = false;
        Ok(())
    }

    /// Get an agent by ID.
    pub fn get(&self, agent_id: u32) -> Option<&Agent> {
        self.agents.iter().find(|a| a.id == agent_id)
    }

    /// List all agents.
    pub fn list(&self) -> Vec<&Agent> {
        self.agents.iter().collect()
    }

    /// Find agents with a specific capability.
    pub fn find_by_capability(&self, capability: &str) -> Vec<&Agent> {
        self.agents
            .iter()
            .filter(|a| a.capabilities.iter().any(|c| c == capability))
            .collect()
    }
}

impl Default for AgentManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Plugin descriptor.
#[derive(Debug, Clone)]
pub struct Plugin {
    pub id: u32,
    pub name: String,
    pub version: String,
    pub loaded: bool,
}

/// Plugin manager.
pub struct PluginManager {
    plugins: Vec<Plugin>,
    next_id: u32,
}

impl PluginManager {
    /// Create a new plugin manager.
    pub fn new() -> Self {
        Self {
            plugins: Vec::new(),
            next_id: 1,
        }
    }

    /// Load a plugin.
    ///
    /// In a full implementation, this would:
    /// - Load the plugin binary/module
    /// - Initialize the plugin
    /// - Register its capabilities
    pub fn load(&mut self, name: &str, version: &str) -> Result<u32, String> {
        let id = self.next_id;
        self.next_id += 1;
        let plugin = Plugin {
            id,
            name: String::from(name),
            version: String::from(version),
            loaded: true,
        };
        self.plugins.push(plugin);
        Ok(id)
    }

    /// Unload a plugin.
    ///
    /// In a full implementation, this would:
    /// - Call plugin cleanup/shutdown
    /// - Unload the module
    /// - Free resources
    pub fn unload(&mut self, plugin_id: u32) -> Result<(), String> {
        let index = self
            .plugins
            .iter()
            .position(|p| p.id == plugin_id)
            .ok_or_else(|| format!("plugin {} not found", plugin_id))?;

        let plugin = &self.plugins[index];
        if !plugin.loaded {
            return Err(format!("plugin {} is not loaded", plugin_id));
        }

        self.plugins.remove(index);
        Ok(())
    }

    /// Get a plugin by ID.
    pub fn get(&self, plugin_id: u32) -> Option<&Plugin> {
        self.plugins.iter().find(|p| p.id == plugin_id)
    }

    /// List all plugins.
    pub fn list(&self) -> Vec<&Plugin> {
        self.plugins.iter().collect()
    }

    /// List loaded plugins.
    pub fn list_loaded(&self) -> Vec<&Plugin> {
        self.plugins.iter().filter(|p| p.loaded).collect()
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}
