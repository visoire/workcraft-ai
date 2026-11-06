//! WorkCraft as an MCP server: every command of the bus is a tool (`chat.send` becomes `chat_send`),
//! so another agent can drive the chat exactly like the UI does. Names follow the Craft apps:
//! `command_list` lists ids, `command_run` runs any id.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{Value, json};
use workcraft_control::CommandBus;
use workcraft_core::{ToolResult, ToolSpec, truncate_bytes};
use workcraft_mcp::ToolHandler;

pub struct WorkCraftMcp {
    bus: Arc<dyn CommandBus>,
}

impl WorkCraftMcp {
    #[must_use]
    pub fn new(bus: Arc<dyn CommandBus>) -> Self {
        Self { bus }
    }
}

fn tool_name(command: &str) -> String {
    command.replace('.', "_")
}

#[async_trait]
impl ToolHandler for WorkCraftMcp {
    fn info(&self) -> (String, String) {
        (
            "workcraft".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        )
    }

    fn instructions(&self) -> Option<String> {
        Some(
            "WorkCraft chat and agent control. chat_send talks to a model that can use the Craft apps; team_run gives a goal to a \
             team of agents. In manual approval mode a write waits for a person: use approval_list and approval_respond."
                .to_string(),
        )
    }

    async fn list_tools(&self) -> Vec<ToolSpec> {
        let mut tools: Vec<ToolSpec> = self
            .bus
            .commands()
            .into_iter()
            .map(|c| {
                let read_only = c.id.ends_with(".list")
                    || c.id.ends_with(".get")
                    || c.id == "mcp.status"
                    || c.id == "approval.list";
                let t = ToolSpec::new(&tool_name(&c.id), &c.description, c.params);
                if read_only { t.read_only() } else { t }
            })
            .collect();
        tools.push(
            ToolSpec::new(
                "command_list",
                "List WorkCraft command ids.",
                json!({ "type": "object", "properties": { "filter": { "type": "string" } } }),
            )
            .read_only(),
        );
        tools.push(ToolSpec::new(
            "command_run",
            "Run a WorkCraft command by id.",
            json!({ "type": "object", "properties": { "command": { "type": "string" }, "params": { "type": "object" } }, "required": ["command"] }),
        ));
        tools
    }

    async fn call_tool(&self, name: &str, arguments: Value) -> ToolResult {
        let (command, params) = match name {
            "command_list" => {
                let f = arguments
                    .get("filter")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_lowercase();
                let ids: Vec<String> = self
                    .bus
                    .commands()
                    .into_iter()
                    .map(|c| c.id)
                    .filter(|i| f.is_empty() || i.to_lowercase().contains(&f))
                    .collect();
                return ToolResult::ok(ids.join("\n"));
            }
            "command_run" => {
                let Some(c) = arguments.get("command").and_then(Value::as_str) else {
                    return ToolResult::err("missing 'command'");
                };
                (
                    c.to_string(),
                    arguments.get("params").cloned().unwrap_or(Value::Null),
                )
            }
            other => {
                let found = self
                    .bus
                    .commands()
                    .into_iter()
                    .find(|c| tool_name(&c.id) == other);
                match found {
                    Some(c) => (c.id, arguments),
                    None => return ToolResult::err(format!("unknown tool '{other}'")),
                }
            }
        };
        match self.bus.execute(&command, params).await {
            Ok(v) => ToolResult::ok(truncate_bytes(
                &serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
                64 * 1024,
            )),
            Err(e) => ToolResult::err(e.to_string()),
        }
    }
}
