---
name: craft-anything
description: >
  Craft Anything is ArtCraft's structured software exploration, clean-room reverse-engineering,
  hypothesis/evidence ledger, and guided asset investigation workbench. Use this skill ONLY when the
  user explicitly activates Craft Anything to structure a technical investigation, record briefs,
  hypotheses, unknowns, and evidence-backed findings, or generate clean-room reports.
---

# Craft Anything

## Safety & Activation Model in WorkCraft

- **Off by Default**: Unlike creative document/media apps, `craft-anything` starts **disabled (`Off`)** until explicitly enabled by the user (in Settings > Tools & MCP, via `@craft-anything`, or via `craft_activate` with approval).
- **Clean-Room Default**: WorkCraft launches `craft-anything-cli mcp` **strictly with `--clean-room --inbox <dir>`** unless the user explicitly opts into deeper modes in config (`craft_anything_deep`, `craft_anything_allow_run`).
- **Human-in-the-Loop Inbox**: When Craft Anything calls `core_person_ask` or `core_action_approve`, it writes `<id>.request.json` into the session inbox; WorkCraft surfaces this as a native approval/question card and writes `<id>.answer.json` back automatically.
- **Always-Gated Generation & Execution**: Paid cloud asset tools (`assets_image_generate`, `assets_model_generate`), dynamic toolset expansion (`enable_toolset`), and target execution always prompt for explicit user approval—even in `Auto` approval mode.

## WorkCraft MCP Surface (`craft-anything__*`)

### Lean Core Toolset (16 tools)
- **Session & Toolsets**: `core_session_show` (), `core_investigation_show` (), `list_toolsets` (), `enable_toolset` (`toolset`).
- **Targets**: `core_targets_list` (), `core_target_select` (`target_id`), `core_target_close` (`target_id`).
- **Structured Investigation Ledger**:
  - `core_brief_write` (`question`, `purpose`, `constraints`, `done_when`, `targets`)
  - `core_plan_update` (`steps`, `based_on_revision`)
  - `core_hypothesis_record` (`claim`, `prediction`, `evidence_for`, `evidence_against`)
  - `core_finding_record` (`statement`, `evidence_ids`, `confidence`)
  - `core_hypothesis_update`, `core_finding_edit`, `core_unknown_add` / `core_unknown_edit` / `core_unknown_verify` (Craft Anything 0.5.0, in the default set; ask `list_toolsets` for the rest)
  - `core_evidence_show` (`evidence_id`, `detail`, `operation`)
  - `core_journal_read` (`day`, `kind`, `tool`, `limit`, `offset`)
  - `core_handoff_write` (`text`)
  - `core_report_write` (`path`, `format`, `include_evidence`)
- **Human Consultation**: `core_person_ask` (`question`, `context`, `choices`, `allow_text`).

### Dynamic Toolset Expansion
- Craft Anything uses `notifications/tools/list_changed` when `enable_toolset` is called. WorkCraft automatically invalidates its cached `tools/list` so newly enabled domain tools become callable on the next turn.
- Only enable extra toolsets when required for the user's explicit task, as each toolset adds schema tokens and gated capabilities.

## Standard Investigation Workflow

1. **Inspect Session**:
   - Call `craft-anything__core_session_show` and `craft-anything__core_investigation_show` to inspect current targets, clean-room policy, and revision numbers.
2. **Frame the Brief & Plan**:
   - Record the investigation question, constraints, and completion criteria with `craft-anything__core_brief_write`, then outline concrete steps with `craft-anything__core_plan_update`.
3. **Hypothesize -> Probe -> Record Findings**:
   - Record testable claims via `craft-anything__core_hypothesis_record`.
   - Gather and inspect evidence (`core_evidence_show`), then link verified `evidence_ids` into `craft-anything__core_finding_record` (never record unsupported claims as findings).
4. **Export Report or Handoff**:
   - Generate a structured Markdown/HTML/JSON report inside the workspace via `craft-anything__core_report_write` (`{"path": "..."}`) or summarize next steps via `craft-anything__core_handoff_write`.

## Method (from Craft Anything 0.5.0)

- **Resume before you restart.** If `core_investigation_show` has a handoff, continue from it and the plan.
- **Observe, derive, infer, and say which.** Observed facts come straight from bytes or a capture, derived facts are computed from them (a call graph), inferences are judgment. Label every claim.
- **Cite evidence ids** (`ev_...`). A claim without one is a hypothesis, not a finding.
- **Absence is not proof.** "No string found" only covers what was searched: state the search boundary and record an unknown (`core_unknown_add`) for what is still open.
- **Predict, then verify.** Write down what you expect before a tool runs; when the result disagrees, update the hypothesis (`core_hypothesis_update`) instead of explaining it away.
- **Summary first, then page.** Large results come back as a summary plus an evidence id: ask for `offset`/`limit` pages. Never repeat a call that already succeeded.
- **Update the plan after every batch of calls** (`todo`, `doing`, `done`, `blocked` with a reason) and end every turn with `core_handoff_write` as if a stranger takes over.
- If the user says "my app" without naming it, ask which one.

## When something fails

| Error code | What to do |
| :--- | :--- |
| `not_found` | Open a target first (`core_target_open`) or check the path |
| `clean_room` | The question needs Deep mode. Tell the user; never work around it |
| `feature_missing` / `engine_missing` | Read the remediation: it names the toolset, engine or setting needed |
| `over_limit` | Page the result or read it by evidence id |
| `stale_revision` | Re-read with `core_investigation_show`, merge, retry |
| `consent_required` | Explain what the call would do and ask the user |

