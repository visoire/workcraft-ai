use serde::{Deserialize, Serialize};

use crate::message::ModelRef;

/// An agent definition: a role with a prompt, an optional model override and a tool scope.
/// Loaded from `agents/*.json` or created at run time through the `agent.define` command.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentDef {
    pub id: String,
    pub name: String,
    /// One line the leader sees when it assigns work.
    #[serde(default)]
    pub scope: String,
    /// System prompt of this agent.
    #[serde(default)]
    pub prompt: String,
    /// Empty: use the session model.
    #[serde(default)]
    pub model: Option<ModelRef>,
    /// Tool name patterns this agent may call. `*` matches anything, `photocraft__*` a prefix.
    /// Empty means no tools.
    #[serde(default)]
    pub tools: Vec<String>,
    /// Skills (by name) this agent may read.
    #[serde(default)]
    pub skills: Vec<String>,
    /// Approval of this agent's tool calls. `None` follows the team. `manual` asks first, `auto` runs without asking
    /// (paid generation, code execution, permanent deletion and signing always ask, whatever is set here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval: Option<ApprovalMode>,
}

impl AgentDef {
    /// Does `tool` fall inside this agent's scope?
    #[must_use]
    pub fn allows(&self, tool: &str) -> bool {
        self.tools.iter().any(|p| matches_pattern(p, tool))
    }
}

/// Glob with a single trailing or leading `*`, or exact. Enough for tool scopes.
#[must_use]
pub fn matches_pattern(pattern: &str, name: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return name.starts_with(prefix);
    }
    if let Some(suffix) = pattern.strip_prefix('*') {
        return name.ends_with(suffix);
    }
    pattern == name
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalMode {
    /// Every write or app-changing tool call waits for a person.
    #[default]
    Manual,
    /// Calls run without asking. Reads never ask in either mode.
    Auto,
}

/// A team: one leader and its workers, plus the rules of the run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TeamConfig {
    pub name: String,
    pub leader: AgentDef,
    pub workers: Vec<AgentDef>,
    /// Workers running at the same time.
    #[serde(default = "default_concurrency")]
    pub concurrency: usize,
    /// Retries the leader may order per task.
    #[serde(default = "default_retries")]
    pub max_retries: u32,
    /// Hard cap on tasks in one run, so a swarm cannot grow without bound.
    #[serde(default = "default_max_tasks")]
    pub max_tasks: usize,
    /// The leader reviews every finished task.
    #[serde(default = "default_true")]
    pub review: bool,
    /// Review only what the automatic checks doubt: a worker that called tools and met no error is accepted
    /// without a leader call. Saves one model call per task. Turn off to review every result.
    #[serde(default = "default_true")]
    pub smart_review: bool,
    /// A short request about exactly one specialist's apps goes straight to that specialist: no plan, no
    /// review, no summary.
    #[serde(default = "default_true")]
    pub direct_route: bool,
    #[serde(default)]
    pub approval: ApprovalMode,
    /// Ask before the plan of a run starts. `None`: only when `approval` is manual.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_approval: Option<bool>,
}

impl TeamConfig {
    /// Does the plan of a run wait for a person?
    #[must_use]
    pub fn plan_needs_approval(&self) -> bool {
        self.plan_approval
            .unwrap_or(self.approval == ApprovalMode::Manual)
    }
}

fn default_concurrency() -> usize {
    3
}
fn default_retries() -> u32 {
    1
}
fn default_max_tasks() -> usize {
    24
}
fn default_true() -> bool {
    true
}

/// One node of the task graph the leader plans.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub title: String,
    /// Agent id of a worker.
    pub agent: String,
    pub instructions: String,
    #[serde(default)]
    pub depends_on: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    Running,
    Reviewing,
    Done,
    Failed,
    Skipped,
}

#[cfg(test)]
pub(crate) mod tests_support {
    use super::*;

    pub fn team() -> TeamConfig {
        let a = |id: &str| AgentDef {
            id: id.into(),
            name: id.into(),
            scope: String::new(),
            prompt: String::new(),
            model: None,
            tools: vec![],
            skills: vec![],
            approval: None,
        };
        TeamConfig {
            name: "t".into(),
            leader: a("leader"),
            workers: vec![a("w")],
            concurrency: 1,
            max_retries: 1,
            max_tasks: 4,
            review: true,
            smart_review: true,
            direct_route: true,
            approval: ApprovalMode::Manual,
            plan_approval: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns() {
        assert!(matches_pattern("*", "x"));
        assert!(matches_pattern("photocraft__*", "photocraft__command_run"));
        assert!(!matches_pattern("photocraft__*", "filmcraft__command_run"));
        assert!(matches_pattern("*_list", "command_list"));
        assert!(matches_pattern("read_file", "read_file"));
        assert!(!matches_pattern("read_file", "read_files"));
    }

    #[test]
    fn the_plan_asks_when_the_team_asks_unless_told_otherwise() {
        let mut t = crate::team::tests_support::team();
        assert!(t.plan_needs_approval());
        t.approval = ApprovalMode::Auto;
        assert!(!t.plan_needs_approval());
        t.plan_approval = Some(true);
        assert!(t.plan_needs_approval());
        t.approval = ApprovalMode::Manual;
        t.plan_approval = Some(false);
        assert!(!t.plan_needs_approval());
    }

    #[test]
    fn empty_scope_allows_nothing() {
        let a = AgentDef {
            id: "a".into(),
            name: "A".into(),
            scope: String::new(),
            prompt: String::new(),
            model: None,
            tools: vec![],
            skills: vec![],
            approval: None,
        };
        assert!(!a.allows("anything"));
    }
}
