use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};
use workcraft_core::{Error, Result};

/// One MCP server. Same shape as Claude Desktop and `.mcp.json`: a `command` for stdio, or a `url`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ServerEntry {
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default, rename = "mcpServers")]
    pub servers: BTreeMap<String, ServerEntry>,
}

impl McpConfig {
    pub fn parse(text: &str) -> Result<Self> {
        serde_json::from_str(text).map_err(|e| Error::Invalid(format!("mcp.json: {e}")))
    }

    /// A missing file is an empty config.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(t) => Self::parse(&t),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_desktop_shape() {
        let c = McpConfig::parse(
            r#"{"mcpServers":{"photocraft":{"command":"photocraft-cli","args":["mcp"],"env":{"A":"1"}},
                "remote":{"url":"http://localhost:5175/mcp"}}}"#,
        )
        .unwrap();
        assert_eq!(c.servers["photocraft"].args, vec!["mcp"]);
        assert_eq!(
            c.servers["remote"].url.as_deref(),
            Some("http://localhost:5175/mcp")
        );
        assert!(McpConfig::parse("{oops").is_err());
    }

    #[test]
    fn missing_file_is_empty() {
        let c = McpConfig::load(Path::new("definitely/not/here/mcp.json")).unwrap();
        assert!(c.servers.is_empty());
    }
}
