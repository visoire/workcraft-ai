# Contributing to WorkCraft AI

Thanks for helping. WorkCraft AI is a small community project in its alpha phase, so clear issues and focused pull
requests go a long way. You do not need to be a Rust expert: bug reports, Craft app test results and doc fixes matter just as much.

By taking part you agree to the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to help

- **Report a bug.** Use the *Bug report* issue form. The version (Settings, or `workcraft-cli --version`), your system and the steps to reproduce are what we need most.
- **Test on another system.** Windows is the verified platform. macOS and Linux reports (builds, crashes, odd rendering) are very welcome.
- **Try it with a Craft app or a model** and tell us what worked and what did not (the *Craft app or model report* form).
- **Suggest or build a feature.** Open a *Feature request* first for anything bigger than a fix, so the direction is agreed before you spend time.
- **Improve docs and skills.** The Craft skills live in `crates/agents/skills/<app>/SKILL.md`.

Problems inside a Craft app itself (WordCraft, PhotoCraft and so on) belong to that app's repository under
[github.com/storytold](https://github.com/storytold). Problems with the browser workspace belong to
[visoire/workcraft](https://github.com/visoire/workcraft).

## Set up

You need Rust 1.90 or newer. On Windows you also need the MSVC Build Tools; on Linux the GUI libraries listed in `.github/workflows/ci.yml`.

```sh
git clone https://github.com/visoire/workcraft-ai.git
cd workcraft-ai
cargo build
cargo run -p workcraft-app --bin workcraft
```

Read [AGENTS.md](AGENTS.md) once: it explains the crates, the command bus and the traps. It is written for AI coding agents
and works just as well for people.

## Making a change

1. Fork the repository and branch from `main`: `git switch -c fix/short-description`.
2. Keep the change focused: one fix or feature per pull request, no drive-by reformatting.
3. Match the style: pure Rust (edition 2024), `unsafe` is denied, user-facing text is plain English that says what to do next, no URLs or keys in error messages.
4. Add or update tests. The scripted-model tests in `crates/agents/tests` need no network or API key.
5. Run the checks (CI runs the same):

   ```sh
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

6. Update the docs that describe your change (README, `docs/`, `AGENTS.md`) and add an entry at the top of `CHANGELOG.md`.
   While the project is in alpha, a meaningful change raises the alpha number (`0.1.0-alpha.N`) in `Cargo.toml` and
   `crates/app/build.rs`; maintainers can do that for you if you prefer.
7. Open the pull request and fill in the template. Say what you tested and on which system.

### Testing against the real Craft apps

The live tests start real apps, so they are opt in and never run in CI:

```sh
cargo test -p workcraft-agents --test apps_contract -- --include-ignored     # on PowerShell: $env:WORKCRAFT_LIVE_APPS = "1" first
cargo test -p workcraft-agents --test apps_headless -- --include-ignored --test-threads=4
```

They use a scratch folder, skip an app that is already running and stop only the process they started. Never kill processes by name.
Craft Anything runs only with `--clean-room`, and no test may call a paid tool.

## Please never commit

API keys, tokens, passwords, personal files, local paths of your computer, chat logs from `~/workcraft/chat`, or binaries.
`.gitignore` covers the usual places, but you are responsible for what you stage. If you commit a key by mistake, tell us
and **revoke the key first**; deleting the commit is not enough.

## Licensing

The project is licensed under MIT OR Apache-2.0. By submitting a contribution you agree that it is dual licensed the same way,
without additional terms or conditions.

## Questions

Open a [discussion](https://github.com/visoire/workcraft-ai/discussions) or an issue. There is no chat server, and answers come as fast as a small project can.
