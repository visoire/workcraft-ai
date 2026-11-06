# The AI core of WorkCraft chat

How a request is built, sent and paid for; how failures are handled; how tools reach the model; how Team mode and MCP
behave; how other IDEs pair. Everything here is in `workcraft-ai`. The TypeScript workspace (`../workcraft`) has the same
ideas in `docs/AI-TOKENS.md` and `docs/AI-PROVIDERS.md`; this chat follows them where they apply and goes further in the
places listed under "Where this goes beyond the workspace".

Measured on Windows 11 on 2026-11-06 with the real Craft apps installed. "Not verified" says what was not run.

## A request, step by step

```
user message
  -> tool router        which Craft app tools go in (craft/router.rs)
  -> context fit        shorten old results, then drop old turns, to fit the window (context.rs)
  -> provider adapter   quirks per provider, prompt caching, usage (llm/*.rs)
  -> http layer         retries, stall guard, readable errors (llm/http.rs)
  <- stream             text, reasoning, tool calls, usage, notices
  -> tools              approval, MCP call, images saved (tools.rs, craft/*)
  -> cost               per model, from the tokens each call reported (llm/price.rs)
```

## Providers

| Provider | Wire | Rules applied |
| :--- | :--- | :--- |
| Ollama | native `/api/chat` | `num_ctx` always sent (16,384 unless set; Ollama's own default of 4,096 silently cut off the start of the prompt, which is the system prompt and the tools), `truncate: false` so an overflow is an error, `keep_alive`, `think` only when asked |
| OpenAI and compatible | `/chat/completions` SSE | `max_completion_tokens` and no `temperature` for o-series and `gpt-5*` on OpenAI itself; cached tokens read from `prompt_tokens_details` |
| DeepSeek | OpenAI wire | `reasoning_content` sent back with the assistant message (its docs require it for tool chains in thinking mode), `thinking: {type}` switch, cache hit and miss counters, reasoning tokens |
| Z.ai GLM | OpenAI wire | `thinking: {type}` switch |
| Anthropic | Messages SSE | `cache_control` on the system prompt, the last tool and the end of the history; cache reads and writes counted separately; readable `overloaded_error` and `rate_limit_error` |
| Gemini, Grok, Mistral, Groq, OpenRouter, Together, LM Studio, custom | OpenAI wire | no special rules |

Measured on this computer (Ollama 0.40.13, `qwen2.5:7b`): a 12,555-token prompt with a secret word at the start was cut to 2,050 evaluated
tokens when no `num_ctx` was sent, and the model could not find the word; with `num_ctx` 16,384 all 12,555 tokens were read and the answer
was right.

Switching model in the middle of a chat is a change of the next turn's `provider:model`. History is provider neutral: reasoning
is kept in the message and only sent where the provider uses it, and dropped from old messages elsewhere to save tokens.

Not verified: Anthropic, OpenAI, Gemini, Grok and GLM were never called (no keys). Their request shapes follow the documentation
in `artifical-intelligence/documentation` and are covered by unit tests of the request bodies. DeepSeek and Ollama were run live.
DeepSeek's documentation says a tool chain in thinking mode without `reasoning_content` is a 400; the live API answered 200
without it on `deepseek-flash`, `deepseek-v4-pro` and `deepseek-reasoner`, so the replay is kept as documented, not as proven
necessary.

## Failures

The same layer wraps all three adapters (`llm/http.rs`).

- **Retries, before anything is streamed:** HTTP 408, 425, 429, 500, 502, 503, 504, 529 and connection failures, up to four
  attempts, 1 s, 2 s, 4 s with jitter, a server's `Retry-After` honoured up to 30 s. A spent quota or balance (`insufficient_quota`,
  "credit", "balance") is not retried. Each wait is announced as a notice ("DeepSeek: rate limited, retrying in 2 s (attempt 2 of 4)")
  in the terminal and as a line from the companion in the desktop app. Cancelling the turn aborts the call, waits included.
- **Stall guard:** a hosted model that sends nothing for 120 s, a local one for 300 s, ends the turn with "no data for N s".
- **Messages that say what to do:** 401 "the API key was not accepted. Check it in Settings, Providers", 402 "no credit left", 404
  "the model or the address was not found", 429 "too many requests", 529 "overloaded", a context overflow "the conversation is too long
  for this model's context window", Ollama not running "Start the Ollama app". The provider's own message is appended, cut to 300
  characters. A URL is never shown (it can carry a key).
- **Cut-off tool calls:** the adapter keeps arguments that are not valid JSON as raw text; the model is told "the text ends in
  the middle: it was probably cut off by the length limit" and to send smaller calls. One stray closing brace after a complete
  object (seen from DeepSeek) is forgiven.
- **Loops:** the same call three times in a row failing, or six times in a row with the same arguments even when it succeeds, stops
  the turn.

## Tokens and cost

Every model call reports `input` (full price), `cache_read`, `cache_write`, `output` and `reasoning` tokens. A turn ends with
one line, in the terminal (`[36480 in (99% cached), 59 out, $0.0002]`) and in the desktop toolbar.

Built-in prices (US dollars per million tokens, from the providers' price pages, 2026-11-05): DeepSeek `deepseek-flash`,
`deepseek-v4-pro` (half price off-peak: 01:00 to 04:00 and 06:00 to 10:00 UTC on weekdays are peak; Chinese holidays count as peak,
so the estimate errs high), Claude Fable 5.1, Opus 5.5, Opus 5, Sonnet 5.5, Sonnet 5, Haiku 5.5 and the 4.x names. Any other
model has no price and shows tokens only: add one in `config.json`:

```json
{ "prices": { "openai:gpt-5": { "input": 1.25, "output": 10.0, "cache_read": 0.125 } } }
```

Local models cost `$0`. Team runs are priced per model, each from its own calls.

`workcraft-cli run tools.stats ["{\"text\":\"...\"}"]` prints what every request carries before the conversation:
system prompt tokens, tool count and tokens per group, and with `text` the size after routing for that question.

## Keeping the request small

**Tool router** (`craft/router.rs`, setting `tool_routing`, default on). Seventeen apps are 200+ tools; the four document apps
alone were 91 tools in every request, 36,000 prompt tokens measured on DeepSeek before the user had said anything, which does not
fit a local model's window and is paid for on every round of a tool loop. A request now carries the built-in tools, the
`craft_*` verbs (which reach every installed app) and the tools of the apps the chat is about: named in the latest message
(`@photocraft`, "a spreadsheet", "blur in.png", "export STEP"), already used in this chat, or asked for with `find_tools`. Apps are
connected on demand, headless. The set only grows within a chat and is computed from the history, so it is the same from round to
round and the provider's prompt cache stays valid. `find_tools {query}` lists matching tools of apps that are not loaded and loads
them for the next step.

| Question | Tools sent | Estimated tokens | Real prompt tokens (DeepSeek) |
| :--- | -: | -: | -: |
| "hello", no router (alpha.2) | 92 | 15,600 | **36,480** |
| "hello" | 32 | 5,800 | **6,578** |
| "@vectorcraft hello" | 45 | | 12,516 |
| "make a spreadsheet with a sum formula" | 48 | 9,500 | |

The estimate (characters divided by 3.4) is about half the real count for this JSON-heavy tool list; the real numbers are what DeepSeek
reported. A whole GridCraft task (five model calls) averaged 15,600 prompt tokens per call, against about 38,000 before.

**Context fit** (`context.rs`). The window is `num_ctx` for Ollama (16k by default), `context_window` from the settings, else a
table by model name, capped at 200k for hosted models. Before each call the history is copied and, when the whole request would not
fit, shortened in steps: old tool results cut and old screenshots dropped (always, for results older than the last eight
messages), then much harder cuts, then whole old turns dropped oldest first behind a note. The newest turn stays whole, every tool
call keeps its result, and the stored chat is never changed. The first time a chat needs more than the routine cut, a notice says so.
The size estimate is characters divided by 3.4; real counts vary by tokenizer.

**Skills** are 20 to 25 KB each. `read_skill` returns the first 7 KB and the list of sections; `read_skill name section="..."`
returns one section, `full=true` everything.

**Team mode** passes at most 4,000 characters of a result to another agent or to the leader (was 6,000 and 8,000).

## Team mode

Specialists work in parallel with scoped tools and one writer per app at a time. Two things save model calls:

- **Straight to one specialist:** a short request (under 400 characters, no "and then", "also", "compare") whose words name apps of one
  specialist goes to that specialist alone: no plan, no review, no summary, no plan approval. Setting `direct_route` in the team.
- **Smart review:** the leader reviews a result only when the automatic checks doubt it: nothing came back, the run stopped early,
  the worker called no tool, or a tool call failed. A worker that called tools without an error is accepted. Setting `smart_review`
  in the team; turn it off to review every result.

Not verified live: a Team run with real models after these two changes (the scripted engine tests pass).

## MCP

- **Transports:** stdio, Streamable HTTP, and the older HTTP+SSE transport. A URL is tried as Streamable HTTP first; HTTP 400, 404,
  405 or 406 on the first request falls back to SSE, and a URL ending in `/sse` goes straight to it.
- **Streamable HTTP:** a busy or unreachable server is asked again (twice, 0.6 s and 1.2 s); a session the server forgot (HTTP 404 with a session id)
  is started anew once; 401 and 403 say to add credentials under `headers` in `mcp.json`.
- **Many servers:** servers are started with a 30 s limit and listed in parallel with a 15 s limit each, so one slow server no longer
  holds up the next model step. A server that cannot be restarted is left alone for 5 s, doubling to 5 minutes, instead of being
  restarted (and waited for) on every step; its last error and the seconds until the next try are in the tool error.
- **Reconnect:** a stdio server that dies is restarted once and the call repeated; `tools/list` is cached and refreshed on
  `notifications/tools/list_changed`.
- **Errors for the model:** calling a tool an app does not have lists the app's real tools; `null` optional arguments are dropped;
  `skill` is accepted for `name` in `read_skill`.

## Pairing with IDEs and agent apps

```sh
workcraft-cli mcp clients                # which apps are found, and their settings files
workcraft-cli mcp setup --dry-run        # say what would change
workcraft-cli mcp setup                  # add a "workcraft" server to every installed app
workcraft-cli mcp setup --client cursor,claude-code
workcraft-cli mcp snippet trae           # the entry to paste by hand
```

Supported: Claude Code (`~/.claude.json`), Claude Desktop, Cursor (`~/.cursor/mcp.json`), Windsurf
(`~/.codeium/windsurf/mcp_config.json`), Antigravity (`~/.gemini/config/mcp_config.json`), Gemini CLI, OpenCode
(`~/.config/opencode/opencode.json`), VS Code (user `mcp.json`); Trae has no known settings file, so it gets the snippet. An app counts as
installed when its settings folder exists. Only the `workcraft` entry changes: other servers keep their order, the file is copied to
`<name>.bak` once, a file that is not valid JSON is left alone, an `opencode.jsonc` with comments is left to you, and an entry you changed
(extra arguments, another program) is never overwritten. The entry starts `workcraft-cli mcp`, which exposes WorkCraft's command
bus (`chat_send`, `team_run`, `approval_respond`, ...) over stdio.

Verified: the file edits, on temporary folders (new file, existing file, order, backup, repeat run, moved program, edited entry, broken
file). `mcp setup --dry-run` was run on this computer and found Claude Code, Claude Desktop, Antigravity, OpenCode and VS Code.
**Not verified:** a real `mcp setup` on this computer (it would change your real IDE settings), and an IDE actually starting the server.
There is no HTTP endpoint for WorkCraft's own MCP server yet (stdio and the control channel only).

## Settings that matter

| Key in `config.json` | Default | What |
| :--- | :--- | :--- |
| `num_ctx` | none (16,384 for Ollama) | context window for Ollama |
| `context_window` | by model name | window for hosted and OpenAI-compatible models |
| `tool_routing` | `true` | send only the tools of the apps the chat is about |
| `prices` | built-in list | dollars per million tokens by model |
| `think` | model default | reasoning on or off where the model supports it |
| team `smart_review`, `direct_route` | `true` | see Team mode |
| `fallback_model` | none | `provider:model` to continue with once when the chosen model cannot be reached, is overloaded, rate limited, out of credit or silent |

## Where this goes beyond the workspace

The workspace's `economy.ts` routes tools by words and compacts history once per question. Here, in addition: retries and a stall
guard with readable errors in the adapter layer, a window-aware history fit with provider-specific reasoning handling, per-model cost
with cache and off-peak rates, tool routing computed from the history so it is cache-stable, on-demand connection of the apps the
router picks, a circuit breaker and parallel listing for many MCP servers, the older SSE transport, session recovery, `mcp setup`,
and the Ollama `num_ctx` default that the workspace sets by probing the GPU and this chat sets to a fixed 16k (no GPU probing:
a larger window needs `num_ctx`).
