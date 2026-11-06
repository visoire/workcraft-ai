use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::message::{Message, Usage};
use crate::team::TaskStatus;
use crate::tool::ToolCall;

/// Everything that happens in the engine, as one stream. The UI renders it, the control channel
/// forwards it, the CLI prints it, and the WorkCraft MCP server turns it into notifications.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Event {
    TurnStarted {
        session: String,
    },
    Text {
        session: String,
        agent: Option<String>,
        delta: String,
    },
    Reasoning {
        session: String,
        agent: Option<String>,
        delta: String,
    },
    MessageAdded {
        session: String,
        message: Message,
    },
    ToolStarted {
        session: String,
        agent: Option<String>,
        call: ToolCall,
    },
    ToolFinished {
        session: String,
        agent: Option<String>,
        call_id: String,
        ok: bool,
        output: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        artifacts: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        mode: Option<String>,
    },
    /// A call waits for a person. Answer with the `approval.respond` command.
    ApprovalRequested {
        id: String,
        session: String,
        agent: Option<String>,
        call: ToolCall,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        destructive: bool,
    },
    /// A short status line while a model call waits or retries ("rate limited, retrying in 4 s").
    Notice {
        session: String,
        agent: Option<String>,
        message: String,
    },
    TurnFinished {
        session: String,
        usage: Usage,
        /// Cost of the turn in US dollars; `None` when a model used has no known price.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cost_usd: Option<f64>,
    },
    TeamPlan {
        session: String,
        run: String,
        tasks: Value,
    },
    TaskUpdate {
        session: String,
        run: String,
        task: String,
        status: TaskStatus,
        note: Option<String>,
    },
    Error {
        session: String,
        message: String,
    },
}

fn is_false(b: &bool) -> bool {
    !*b
}
