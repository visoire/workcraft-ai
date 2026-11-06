# scripts

`publish.ps1` (Windows PowerShell 5.1 and PowerShell 7) and `publish.sh` (bash) put this repository on GitHub safely. They read
`.github/repo.json` and print a dry run unless you add `-Execute` / `--execute`. Full guide: [docs/PUBLISHING.md](../docs/PUBLISHING.md).

| Command | What it does |
| :--- | :--- |
| `status` | branch, remote, version, changes |
| `check` | scans the files that would be published for keys, tokens, private keys, personal paths, binaries and big files; checks the community files and the changelog; exit code 1 on errors |
| `init` | `git init`, branch `main`, remote `origin` |
| `create` | creates the empty GitHub repository with `gh` |
| `push` | `check`, `git add -A`, commit, push (asks you to type `yes`) |
| `meta` | description, homepage, topics, Issues and Discussions |
| `labels` | creates or updates the labels in `.github/labels.yml` |
| `changelog` | prints the entry of the current version; `-New` adds an empty one |
| `release` | tags `v<version>` and publishes a pre-release (the workflow builds the Windows files; `-Local` builds them here) |

```powershell
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 check
powershell -ExecutionPolicy Bypass -File scripts/publish.ps1 push -Message "Initial public alpha" -Execute
```
