# CLAUDE.md

Claude Code reads this file at the start of a session. The project context lives in AGENTS.md:

@AGENTS.md

## Short version

- Alpha phase: no stable release until the maintainer says so. With every meaningful change raise the alpha number
  (`0.1.0-alpha.N`) in `Cargo.toml` and `crates/app/build.rs`, add an entry at the top of `CHANGELOG.md` under
  `## [0.1.0-alpha.N] - YYYY-MM-DD`, update the affected Markdown files (README, AGENTS.md, `docs/`, `packaging/`) and
  `SOURCES.md`. Do it without being asked.
- Checks before a change is done: `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
  `cargo test --workspace`.
- Desktop only; WorkCraft is the product name; say "workspace", never "IDE".