# Security policy

## Supported versions

| Version | Supported |
| :--- | :--- |
| `0.1.0-alpha.N` (the newest alpha) | yes |
| earlier alphas and development builds | no |

WorkCraft AI is in its alpha phase. Only the newest alpha receives fixes.

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Report them privately through [GitHub security advisories](https://github.com/visoire/workcraft-ai/security/advisories/new).
Include the version (`workcraft-cli --version`), your system, the steps to reproduce and what an attacker could do.
You get an answer within a few days; this is a small project, so fixes are made as fast as one person can.

## Security model in short

- **Local by default.** There is no account and no telemetry. The only network traffic is to the model providers and MCP servers you configure.
- **Keys.** A provider key is read from an environment variable or saved from Settings into `~/workcraft/chat/config.json` (plain text, like the workspace does).
  `config.get`, `config.set` and `provider.info`, and therefore also the control channel and MCP, return only `has_key` and a short hint. Keys are never written to chats.
- **Control channel.** Loopback TCP only. A 64 hex character token is created at every start and kept in `control.json` while the app runs (removed on exit).
- **Files.** The built-in file tools are confined to the workspace folder. Craft apps get sandbox roots that are the workspace plus folders you add, never your home folder.
- **Approvals.** In manual mode every write waits for Allow or Deny. Paid generation, code execution, permanent deletion and signing ask even in Auto mode.
- **Unsigned binaries.** The alpha builds are not code-signed. Verify downloads against `SHA256SUMS.txt` from the release, or build from source.

In scope: the desktop app, the CLI, the MCP server, the control channel, the tool router and the approval logic.
Out of scope: the Craft apps themselves (report to [github.com/storytold](https://github.com/storytold)), and the models or providers you connect.
