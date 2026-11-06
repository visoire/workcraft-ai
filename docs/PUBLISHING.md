# Publishing WorkCraft AI on GitHub

A step-by-step guide for the maintainer. **Nothing in this repository uploads anything by itself.** Every script prints what it
would do (a *dry run*) until you add `-Execute` (PowerShell) or `--execute` (bash). No token or key is stored anywhere: the GitHub CLI
(`gh`) keeps its own sign-in.

Target repository: **`visoire/workcraft-ai`** (public, MIT OR Apache-2.0). Its sibling is `visoire/workcraft` (the workspace); that
repository has the same scripts and its own guide.

## 0. What is ready

| Part | Where |
| :--- | :--- |
| README with badges, what it is and is not, install, usage, platforms, alpha warning | `README.md` (long reference: `docs/USAGE.md`) |
| License | `LICENSE-MIT`, `LICENSE-APACHE` (MIT OR Apache-2.0) |
| Community files | `CONTRIBUTING.md`, `CODE_OF_CONDUCT.md`, `SECURITY.md` |
| Issue forms, PR template, labels | `.github/ISSUE_TEMPLATE/*`, `.github/pull_request_template.md`, `.github/labels.yml` |
| Description, topics, features | `.github/repo.json` (single source for the scripts) |
| CI and release workflows | `.github/workflows/ci.yml`, `.github/workflows/release.yml` |
| Changelog and alpha release notes | `CHANGELOG.md`, `docs/releases/<version>.md` |
| Scripts | `scripts/publish.ps1` (Windows PowerShell) and `scripts/publish.sh` (bash) |

Status of the scripts: `publish.ps1 check`, `status` and the dry runs were run on Windows. `publish.sh` has the same commands but was **not run**
(no bash on the machine it was written on): try `scripts/publish.sh check` first and report a bug if it misbehaves.

## 1. One-time setup on your computer

1. Install Git and the GitHub CLI, then sign in. The CLI opens the browser; you never paste a token into a file or a chat.

   ```powershell
   winget install --id GitHub.cli
   gh auth login          # choose GitHub.com, HTTPS, "Login with a web browser"
   gh auth status
   ```

2. Open a terminal in the repository folder (`workcraft-ai`). Windows may block downloaded scripts; the examples use
   `-ExecutionPolicy Bypass` for that one run only.

## 2. Before the first push (10 minutes)

1. **Screenshots.** Take 2 to 4 PNG files (or a short GIF) of the real app and put them in `docs/images/`:
   `chat-light.png`, `team-dark.png`, `settings-blip.png`. Then uncomment the screenshot block in `README.md`. Do not show real
   chats, file names, API keys or e-mail addresses in them.
2. **Re-read `README.md`** once and the alpha warning. The README must never promise more than is verified.
3. **Look at what will be published.** The scripts honor `.gitignore`. Not published on purpose: `target/`, `dist/`, the `.exe` copies in the
   repository root, `*.zip`, `.env*`, `keys.json`, `config.json`, `control.json`. Published on purpose: the source, `docs/`, `examples/`, `PLAN.md`,
   `CHAT-PLAN.MD`, `STATUS.md`, `USED-REPOS.md` (the maintainers' notes). If you do not want the notes public, add them to `.gitignore` first.
4. **Run the check:**

   ```powershell
   powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 check
   ```

   It scans every file that would be published for API keys, tokens, private keys, personal home paths, binaries and files over 10 MB,
   and checks that the community files, the changelog entry and the alpha wording exist. Fix every `FAIL` and read every `WARN`.
   A real key found in a file must be **revoked at the provider**, not only deleted from the file.
5. **Run the project checks** (the same as CI):

   ```powershell
   cargo fmt --all --check
   cargo clippy --workspace --all-targets -- -D warnings
   cargo test --workspace
   ```

## 3. Create the empty repository

In the browser: <https://github.com/new> > owner `visoire`, name `workcraft-ai`, **Public**, and leave *Add a README*, *.gitignore* and
*license* **unchecked** (the repository already has them). Or with the CLI:

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 create -Execute
```

## 4. First push

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 init -Execute        # git init, branch main, remote origin
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 push -Message "Initial public alpha (0.1.0-alpha.9)"     # dry run: shows the plan
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 push -Message "Initial public alpha (0.1.0-alpha.9)" -Execute
```

`push` runs `check` again, stages everything, shows the top-level folders and asks you to type `yes` before it commits and pushes. Afterwards open
the repository in the browser and look at the file list, the README rendering and the Actions tab (CI starts by itself).

## 5. Repository settings

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 meta -Execute        # description, homepage, topics, Issues and Discussions on
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 labels -Execute      # the labels from .github/labels.yml
```

Edit `.github/repo.json` and run `meta` again whenever the description or topics change.

**Suggested short description** (GitHub limit 350 characters):

> Alpha: local-first AI chat and multi-agent desktop app in Rust (egui). Ollama or your own API keys, MCP, 17 Craft apps. MIT OR Apache-2.0.

**Topics** (GitHub allows 20): `ai`, `ai-agents`, `multi-agent`, `chatbot`, `llm`, `mcp`, `model-context-protocol`, `ollama`, `rust`, `egui`,
`local-first`, `privacy`, `desktop-app`, `windows`, `claude`, `openai`, `deepseek`, `agent-skills`, `alpha`, `open-source`.

By hand in the browser (not scriptable): *Settings > General > Social preview* (upload a 1280x640 image, for example the logo on a plain
background), *Settings > Code security* (enable Dependabot alerts, secret scanning and push protection: free for public repositories), and
*Settings > Branches* (protect `main`: require a pull request and the `check` jobs, once you have collaborators).

## 6. Publish the alpha release (pre-release)

Releases are always **pre-releases** while the project is alpha. The notes come from `docs/releases/<version>.md` (or the changelog entry).

**Option A, recommended: the workflow builds the Windows files in GitHub's cloud.** Nothing from your computer is uploaded.

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 release -Execute
```

This creates the tag `v0.1.0-alpha.9` and pushes it. `.github/workflows/release.yml` then builds `WorkCraft.exe` and `WorkCraft-CLI.exe`,
writes `SHA256SUMS.txt` and publishes the pre-release (watch the *Actions* tab). Rerun a failed build from the Actions tab with the tag as input.

**Option B: build and upload from your computer** (the files are the ones you tested):

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 release -Local -Execute
```

Either way, open the release page, read the notes, download the `.exe` once and compare it with `SHA256SUMS.txt`. The binaries are not code-signed;
the notes say so.

## 7. Every later alpha

1. Change the code, then raise the alpha number in `Cargo.toml` and `crates/app/build.rs`, and run `scripts/publish.ps1 changelog -New -Execute` to add the
   changelog entry (then fill it in). Add `docs/releases/<version>.md` for a release with highlights.
2. `scripts/publish.ps1 check`, then `push -Message "..." -Execute`, then `release -Execute`.

## 8. If a secret was committed

1. **Revoke or rotate the key at the provider first.** That is the only fix that counts.
2. Remove it from the files, commit, and (if it was pushed) tell the users who might have copied it.
3. Rewriting history (`git filter-repo`) is optional and does not undo exposure. GitHub push protection will block most keys before they leave your computer.

## 9. Troubleshooting

| Problem | Fix |
| :--- | :--- |
| `gh: command not found` | Open a new terminal after installing, or `winget install --id GitHub.cli` |
| `gh auth status` says not logged in | `gh auth login` |
| `remote origin already exists` | `scripts/publish.ps1 init -Execute` sets the right URL |
| Push rejected (the remote has commits) | You created the repo with a README. Either delete and recreate it empty, or `git pull --rebase origin main` once |
| `check` flags a path like `C:\Users\<name>\` | It is a personal path: replace it with `<home>` or `%USERPROFILE%` |
| A file is "not published" but you want it | It matches `.gitignore`; `git check-ignore -v <file>` tells you which rule |
| The release workflow fails | Open the failed run in the Actions tab; the build uses stable Rust on `windows-latest` |
