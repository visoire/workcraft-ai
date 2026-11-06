# Packaging

Not done yet. This folder only records the plan so it is not lost. Versions are alpha (`0.1.0-alpha.N`, see the
[changelog](../CHANGELOG.md)); the Windows binaries are named `WorkCraft.exe` and `WorkCraft-CLI.exe`.

Target matrix, taken from the Craft apps:

| Platform | Artifact |
|---|---|
| macOS | universal binary (`aarch64-apple-darwin` and `x86_64-apple-darwin`, joined with `lipo`), signed and notarized `.app` |
| Windows | x64 and ARM64 `.msi` (and a portable zip) |
| Linux | AppImage, `.deb`, `.rpm` |

Two binaries ship together: `workcraft` (the app) and `workcraft-cli` (headless, also the MCP server).

Both embed the 17 Craft skills (`crates/agents/skills/`) and the 17 64 px Craft app icons
(`crates/app/assets/icons/`) at build time; nothing else needs to be installed next to them. The Craft apps themselves
are separate installs, found at run time in `Program Files\<App>`, `Program Files\ArtCraft\<App>` and the other
locations listed in [docs/CRAFT-APPS.md](../docs/CRAFT-APPS.md). The Windows version resource (currently
`0.1.0-alpha.9`) comes from `crates/app/build.rs`.

The Craft apps reuse the shared release actions in `craft-common-actions`. Wiring this repository to
them needs the repository to live under the same organisation and the signing secrets, so it waits for
that decision. `.github/workflows/ci.yml` already checks formatting, clippy and tests on all three
operating systems.
