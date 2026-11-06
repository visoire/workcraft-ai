use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// A tool as a model sees it. `input_schema` is JSON Schema, passed through unchanged from MCP servers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    #[serde(default = "empty_schema")]
    pub input_schema: Value,
    /// The tool only reads. Reads never wait for approval; everything else does in manual mode.
    #[serde(default)]
    pub read_only: bool,
    /// Explicit `annotations.readOnlyHint` from the MCP server, when supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only_hint: Option<bool>,
    /// Explicit `annotations.destructiveHint == true` from the MCP server.
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn empty_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

impl ToolSpec {
    #[must_use]
    pub fn new(name: &str, description: &str, input_schema: Value) -> Self {
        Self {
            name: name.to_string(),
            description: description.to_string(),
            input_schema,
            read_only: false,
            read_only_hint: None,
            destructive: false,
        }
    }

    #[must_use]
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self.read_only_hint = Some(true);
        self
    }

    #[must_use]
    pub fn destructive(mut self) -> Self {
        self.destructive = true;
        self
    }
}

/// Guess whether an MCP tool only reads, from its name. Used when the server gives no
/// `readOnlyHint`. Conservative: anything not clearly a read counts as a write.
#[must_use]
pub fn looks_read_only(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    const READ: [&str; 25] = [
        "list",
        "get",
        "read",
        "inspect",
        "info",
        "status",
        "search",
        "find",
        "describe",
        "preview",
        "commands",
        "query",
        "render",
        "screenshot",
        "measure",
        "compare",
        "check",
        "pending",
        "running",
        "wait",
        "show",
        "browse",
        "topology",
        "functions",
        "elements",
    ];
    // "command_run", "doc_save" and friends contain no read word at the verb position, or also
    // contain an explicit write verb.
    n.split(['_', '-', '.']).any(|part| READ.contains(&part))
        && !n.split(['_', '-', '.']).any(|p| {
            matches!(
                p,
                "run"
                    | "write"
                    | "set"
                    | "delete"
                    | "save"
                    | "export"
                    | "execute"
                    | "create"
                    | "add"
                    | "import"
                    | "open"
                    | "new"
                    | "apply"
                    | "place"
                    | "replace"
            )
        })
}

/// An image returned inside an MCP `tools/call` content array (`data` is base64).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImagePart {
    pub mime_type: String,
    pub data: String,
}

/// A call the model asked for.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub arguments: Value,
}

/// What a tool returned. `ok = false` goes back to the model as text so it can correct itself.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolResult {
    pub ok: bool,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<ImagePart>,
}

impl ToolResult {
    #[must_use]
    pub fn ok(text: impl Into<String>) -> Self {
        Self {
            ok: true,
            text: text.into(),
            images: Vec::new(),
        }
    }

    #[must_use]
    pub fn err(text: impl Into<String>) -> Self {
        Self {
            ok: false,
            text: text.into(),
            images: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_images(mut self, images: Vec<ImagePart>) -> Self {
        self.images = images;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn read_only_heuristic_is_conservative() {
        for r in [
            "command_list",
            "doc_inspect",
            "jobs_list",
            "doc_render_preview",
            "read-sheet",
            "get_status",
            "viewport_render",
            "view_render",
            "ui_screenshot",
            "screenshot",
            "measure",
            "running",
            "pending",
            "core_session_show",
        ] {
            assert!(looks_read_only(r), "{r}");
        }
        for w in [
            "command_run",
            "command_batch",
            "doc_save",
            "write-file",
            "doc_export",
            "doc_new",
            "ui_set",
            "create_node",
            "import_audio",
        ] {
            assert!(!looks_read_only(w), "{w}");
        }
    }
}
