use crate::core::config::data::{Config, McpServerConfig};

#[derive(Debug, Clone)]
pub struct McpRegistry {
    servers: Vec<McpServerConfig>,
}

impl McpRegistry {
    pub fn from_config(config: &Config) -> Self {
        let servers = config
            .mcp_servers
            .iter()
            .filter(|server| server.is_enabled())
            .cloned()
            .collect();
        Self { servers }
    }

    pub fn servers(&self) -> &[McpServerConfig] {
        &self.servers
    }

    pub fn find_server(&self, id: &str) -> Option<&McpServerConfig> {
        self.servers
            .iter()
            .find(|server| server.id.eq_ignore_ascii_case(id))
    }
}
