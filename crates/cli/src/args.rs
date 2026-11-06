//! Argument parsing. Unknown flags are errors, like the Craft app CLIs: no silent fallback.

#[derive(Debug, PartialEq, Eq)]
pub enum Cmd {
    Help,
    Version,
    Chat {
        prompt: Option<String>,
    },
    Serve {
        port: u16,
    },
    Mcp {
        action: McpAction,
    },
    Commands {
        filter: Option<String>,
    },
    Run {
        command: String,
        params: Option<String>,
    },
    Models,
}

/// What `workcraft-cli mcp` does.
#[derive(Debug, PartialEq, Eq)]
pub enum McpAction {
    /// The server on stdio (what other apps start).
    Serve,
    /// Write a `workcraft` entry into the MCP settings of installed IDEs and agent apps.
    Setup,
    /// Which apps are known and found on this computer.
    Clients,
    /// The entry to paste into one app's settings by hand.
    Snippet { client: String },
}

#[derive(Debug, PartialEq, Eq, Default)]
pub struct Opts {
    /// `--client a,b`: the apps `mcp setup` pairs with (default: every installed one).
    pub clients: Option<Vec<String>>,
    /// `--dry-run`: say what `mcp setup` would do and write nothing.
    pub dry_run: bool,
    pub model: Option<String>,
    pub session: Option<String>,
    pub team: Option<String>,
    pub auto: bool,
    pub no_mcp: bool,
}

pub const USAGE: &str = "workcraft-cli: chat and agents for the Craft apps

USAGE
  workcraft-cli chat [PROMPT]        chat in the terminal (one answer when PROMPT is given)
  workcraft-cli serve [--port N]     control channel on 127.0.0.1 (token in <data>/control.json)
  workcraft-cli mcp                  MCP server on stdio that exposes WorkCraft itself
  workcraft-cli mcp setup            add WorkCraft to the MCP settings of installed IDEs and agent apps
                                     (Claude Code and Desktop, Cursor, Windsurf, Antigravity, Gemini CLI,
                                     OpenCode, VS Code); --client a,b picks apps, --dry-run only reports
  workcraft-cli mcp clients          which of those apps are found, and their settings files
  workcraft-cli mcp snippet <APP>    the entry to paste by hand (also for Trae)
  workcraft-cli commands [FILTER]    list command ids
  workcraft-cli run <ID> [JSON]      run one command id and print the result
  workcraft-cli models               models of every configured provider

OPTIONS
  --model <provider:model>   model for this run, for example ollama:qwen3:4b
  --session <id>             continue a chat
  --team <name>              run the message as a team job (preset: craft)
  --auto                     run tools without asking (default: ask before every write)
  --no-mcp                   do not start the MCP servers from mcp.json
  --port <n>                 control channel port (default: pick a free one)
  --client <a,b>             apps for mcp setup, for example claude-code,cursor
  --dry-run                  mcp setup: show what would change, write nothing
  -h, --help / -V, --version

DATA  $WORKCRAFT_CHAT_HOME, default ~/workcraft/chat   (config.json, mcp.json, agents/, teams/, skills/, sessions/)
";

pub fn parse(args: &[String]) -> Result<(Cmd, Opts), String> {
    let mut opts = Opts::default();
    let mut pos: Vec<String> = Vec::new();
    let mut port: Option<u16> = None;
    let mut help = false;
    let mut version = false;
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("{name} needs a value"))
        };
        match a.as_str() {
            "-h" | "--help" => help = true,
            "-V" | "--version" => version = true,
            "--auto" => opts.auto = true,
            "--no-mcp" => opts.no_mcp = true,
            "--dry-run" => opts.dry_run = true,
            "--client" => {
                opts.clients = Some(
                    value("--client")?
                        .split(',')
                        .map(|s| s.trim().to_ascii_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect(),
                );
            }
            "--model" => opts.model = Some(value("--model")?),
            "--session" => opts.session = Some(value("--session")?),
            "--team" => opts.team = Some(value("--team")?),
            "--port" => {
                port = Some(
                    value("--port")?
                        .parse()
                        .map_err(|_| "--port must be a number from 0 to 65535".to_string())?,
                )
            }
            s if s.starts_with('-') && s.len() > 1 => return Err(format!("unknown option '{s}'")),
            _ => pos.push(a.clone()),
        }
    }
    if help {
        return Ok((Cmd::Help, opts));
    }
    if version {
        return Ok((Cmd::Version, opts));
    }
    let mut p = pos.into_iter();
    let cmd = match p.next().as_deref() {
        None => Cmd::Help,
        Some("chat") => Cmd::Chat {
            prompt: {
                let rest: Vec<String> = p.collect();
                (!rest.is_empty()).then(|| rest.join(" "))
            },
        },
        Some("serve") => Cmd::Serve {
            port: port.unwrap_or(0),
        },
        Some("mcp") => Cmd::Mcp {
            action: match p.next().as_deref() {
                None | Some("serve") => McpAction::Serve,
                Some("setup" | "install") => McpAction::Setup,
                Some("clients") => McpAction::Clients,
                Some("snippet") => McpAction::Snippet {
                    client: p
                        .next()
                        .ok_or("mcp snippet needs an app name, for example cursor")?,
                },
                Some(other) => return Err(format!("unknown mcp command '{other}'")),
            },
        },
        Some("commands") => Cmd::Commands { filter: p.next() },
        Some("run") => Cmd::Run {
            command: p.next().ok_or("run needs a command id")?,
            params: p.next(),
        },
        Some("models") => Cmd::Models,
        Some(other) => return Err(format!("unknown command '{other}'")),
    };
    Ok((cmd, opts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_string).collect()
    }

    #[test]
    fn parses_commands_and_options() {
        let (c, o) = parse(&a("chat hello there --model ollama:x --auto")).unwrap();
        assert_eq!(
            c,
            Cmd::Chat {
                prompt: Some("hello there".into())
            }
        );
        assert_eq!((o.model.as_deref(), o.auto), (Some("ollama:x"), true));
        assert_eq!(
            parse(&a("serve --port 7878")).unwrap().0,
            Cmd::Serve { port: 7878 }
        );
        assert_eq!(
            parse(&a("run chat.list")).unwrap().0,
            Cmd::Run {
                command: "chat.list".into(),
                params: None
            }
        );
        assert_eq!(parse(&a("")).unwrap().0, Cmd::Help);
        assert_eq!(
            parse(&a("mcp")).unwrap().0,
            Cmd::Mcp {
                action: McpAction::Serve
            }
        );
        let (c, o) = parse(&a("mcp setup --client Claude-Code,cursor --dry-run")).unwrap();
        assert_eq!(
            c,
            Cmd::Mcp {
                action: McpAction::Setup
            }
        );
        assert_eq!(
            o.clients,
            Some(vec!["claude-code".to_string(), "cursor".to_string()])
        );
        assert!(o.dry_run);
        assert_eq!(
            parse(&a("mcp snippet trae")).unwrap().0,
            Cmd::Mcp {
                action: McpAction::Snippet {
                    client: "trae".into()
                }
            }
        );
        assert_eq!(parse(&a("-V")).unwrap().0, Cmd::Version);
    }

    #[test]
    fn rejects_unknown_things() {
        assert!(parse(&a("chat --wat")).is_err());
        assert!(parse(&a("explode")).is_err());
        assert!(parse(&a("run")).is_err());
        assert!(parse(&a("serve --port abc")).is_err());
        assert!(parse(&a("chat --model")).is_err());
        assert!(parse(&a("mcp snippet")).is_err());
        assert!(parse(&a("mcp frobnicate")).is_err());
    }
}
