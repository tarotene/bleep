# publish-guard

A deny/ask gate that prevents AI coding agents from leaking company/private repository names onto public GitHub surfaces (git push, PR/Issue creation, MCP tool calls).

## Background

Born as a Claude Code PreToolUse hook (originally
`config/claude/hooks/public-publish-guard.sh` in
[tarotene/dotfiles](https://github.com/tarotene/dotfiles)), this repository
extracted the decision engine into an agent-agnostic CLI and added thin
adapters for the Claude Code plugin, Codex CLI, and Copilot CLI.

## Install

This repository never commits denylist data (org names, repo names). You
place your own under `$XDG_CONFIG_HOME/publish-guard/` (defaults to
`~/.config/publish-guard/`). **The only thing you realistically have to
write by hand is one line in `orgs.txt` with your org name** — your own
private/internal repository names are live-derived from the org name you
wrote there plus the owner of your authenticated `gh` user, using your own
`gh` credentials. Nobody carries around a literal list of repo names.

```
$ mkdir -p ~/.config/publish-guard
$ echo acme > ~/.config/publish-guard/orgs.txt   # write only your company's org name
```

Optional config files you can add (all optional, one entry per line, `#`
comments and blank lines ignored):

| File | Purpose |
|---|---|
| `orgs.txt` | Company/other-party org names (this is realistically the only one you write by hand) |
| `repos.txt` | Explicit `org/repo` references (optional) |
| `allow-stopwords.txt` | Words excluded from the denylist (`.github` is a built-in default) |
| `allow-regexes.txt` | If this regex matches the scanned text, it disables **only the ask** verdict (it does not loosen deny) |
| `allow-paths.txt` | Regex of file paths excluded from `audit` |

### Claude Code

```
/plugin marketplace add tarotene/publish-guard
/plugin install publish-guard@publish-guard
```

`.claude-plugin/plugin.json` reads `hooks/hooks.json`, which registers
`hooks/claude-adapter.sh` on `PreToolUse` with a single compound matcher
`Bash|mcp__.*` (self-resolved via `${CLAUDE_PLUGIN_ROOT}`).

**Why the matchers aren't split**: you might be tempted to write two
separate hook entries, one for Bash and one for MCP — don't. Combined with a
registration mechanism that can only detect existing entries by exact
command-string match (e.g. home-manager's `registerHooks`), registering the
same command under two matchers means the second one short-circuits on
"already registered," leaving one of the two paths unchecked. Keeping it to
a single compound matcher gives the same behavior under any registration
mechanism.

**Rolling this out org-wide**: Claude Code's managed settings can restrict
which marketplaces an organization may use via `strictKnownMarketplaces`,
and pre-install for every user via `enabledPlugins`.
— Anthropic, "Plugin marketplaces",
<https://code.claude.com/docs/en/plugin-marketplaces.md> (accessed 2026-09-10).

```json
{
  "strictKnownMarketplaces": [
    {"source": "github", "repo": "tarotene/publish-guard"}
  ],
  "enabledPlugins": {
    "publish-guard@publish-guard": true
  }
}
```

Denylist data (`orgs.txt`, etc.) does not travel through this distribution
path — each user still has to write their org name into their own
`~/.config/publish-guard/orgs.txt` (this is intentional; see "This is not a
security boundary" under [Scope](#scope) for details).

### Codex CLI

Add this to `~/.codex/hooks.json` by hand (or write your own idempotent
merger, similar to dotfiles' `register-codex-hooks`):

```json
{
  "hooks": {
    "PreToolUse": [
      {
        "matcher": "Bash|mcp__.*",
        "hooks": [
          {"type": "command", "command": "/path/to/publish-guard/adapters/codex-adapter.sh", "timeout": 20}
        ]
      }
    ]
  }
}
```

The `Bash` matcher has been verified against a real Codex CLI
(`codex exec --dangerously-bypass-hook-trust`). **Codex's MCP tool naming
convention (whether it follows the `mcp__server__tool` shape) has not been
verified** — if you use an MCP server, check the actual `tool_name` and
adjust the matcher accordingly.

### Copilot CLI

Add this under the `"hooks"` key in `~/.copilot/settings.json`:

```json
{
  "hooks": {
    "preToolUse": [
      {"type": "command", "bash": "/path/to/publish-guard/adapters/copilot-adapter.sh", "timeoutSec": 20}
    ]
  }
}
```

Copilot's `preToolUse` **has no matcher** (verified against a real
instance — it fires unconditionally on every tool call). Filtering happens
inside the adapter itself (checking whether `toolName == "bash"`).

## Usage

**`--cwd DIR`**: a global option, placed before the subcommand name, that
tells `resolve_repo_nwo`/`compute_push_diff_text`/`resolve_default_branch` to
treat `DIR` as the target repository's location instead of the hook
process's own cwd. All three official adapters pass this automatically from
the PreToolUse payload's `cwd` field (Claude, Codex, and Copilot all expose
one — Copilot's has been verified against a real instance; see `hooks/claude-
adapter.sh` / `adapters/*.sh` for the field paths). This exists because a
PreToolUse hook runs as a separate process **before** the actual shell
command executes, so a command like `cd /other/repo && gh pr create ...`
can't be resolved by looking at the hook process's own cwd — it's still
sitting wherever the agent's session started (#10, #14).

**How `scan-bash-command` classifies a compound command**: `CMD` is split
into segments on `;`, `&&`, `||`, `|`, and newlines (quote-aware — `&&`
inside a quoted string is not treated as a separator). Each segment is
tokenized and walked by **position**, skipping recognized global options
(`git`'s `-C`/`-c`/`--git-dir`/etc., `gh`'s `--repo`/`-R`), before checking
whether the next token is the actual subcommand. This means `git -C <dir>
push` and `gh --repo owner/repo pr create` are correctly recognized as
push/publish actions — a plain adjacency regex (the previous implementation)
missed both (#9, #14). The recognized `gh` publish surface is
`pr|issue create|edit|comment`, `release create|edit`, `repo edit`,
`gist create`, and `api` with a write method (`-X`/`--method` set to `POST`,
`PUT`, or `PATCH` — a plain `gh api <endpoint>` read defaults to GET and is
not scanned). `gh api` doesn't take `--repo`, so its target-repo resolution
falls back to the same `cd`-tracking/`git remote` lookup as everything else.
If any of these segments is found, the denylist match runs against `CMD`
with any segment that is
**exactly** `cd <single token>` removed — not against the whole command
string — so a `cd`'s path argument merely containing a private repo name no
longer triggers a false-positive hard-deny (#9); everything else (including
any oddly-split fragment) stays in the scanned text, so this narrowing never
creates a new blind spot. If a `git push` segment is found, its effective
target directory is resolved from that segment's own `-C` override, or
failing that, from the cumulative effect of every preceding bare `cd <dir>`
segment (not just the first one) starting at `--cwd`/the hook's own cwd —
this is what makes `cd /other/repo && git push` and `cd /other/repo && gh pr
create ...` correctly resolve visibility/diffs against `/other/repo` instead
of the hook process's own cwd (#10, #14). Nested `$(...)` command
substitutions are not tracked — this is an approximation within the "not a
security boundary" scope already stated below.

`publish-guard scan`/`scan-push`/`scan-bash-command` all share the same exit
code contract (for anyone scripting against this themselves):

| exit code | meaning | stdout |
|---|---|---|
| 0 | pass (no denylist match) | none |
| 1 | ask (a bare org-name match — could collide with a legitimate use) | one-line reason |
| 2 | deny (an explicit org/repo reference or a specific repo-name match — almost certainly an unintended leak) | one-line reason |

**What `scan-push` checks**: `publish-guard scan-push` (and `git push`
detection via `scan-bash-command`) inspects the **added lines** (plus
new/renamed file paths) and the full commit message of each commit in the
range being pushed. **Removed lines are not scanned** (#7 — this addresses
the problem where a fix that removes a line containing a denylisted name
would itself get denied). Deleted content is either already on the base
(already public) or was already scanned as an added line in an earlier
commit within the same push range, so scanning removed lines has no
leak-prevention value. Because this looks at commits (`git log -p`), not the
net diff, adding a secret in one commit and removing it in a later commit
within the same branch still gets denied once you push, since the
intermediate commit's content becomes public regardless.

**A caveat about `audit --remote`**: `publish-guard audit --remote` fetches
the **full title/body of every open Issue/PR** in the target repository via
`gh issue list`/`gh pr list` and loads it into the local process. Running it
against a company private/internal repository temporarily leaves that
internal Issue/PR content in this process's memory (and shell history,
etc.). Choose your targets deliberately.

## Scope

This repository owns the decision-engine CLI (`scan`/`scan-push`/
`scan-bash-command`/`audit`), thin adapters for the Claude Code plugin,
Codex CLI, and Copilot CLI, and the denylist/allowlist config-file format.
Building a security boundary against malicious evasion, secret detection
(gitleaks and similar tools' job), and per-host hook wiring (dotfiles' job)
are treated as concerns outside this repository.

**This is not a security boundary.** This tool is a **guardrail against
accidents and agent slip-ups**, not a mechanism that prevents malicious
evasion. There are three reasons for this.

1. **There is no way to mediate the path where the constrained party (the
   agent, or a human typing directly) types straight into github.com in a
   browser.** Rewording, splitting, base64-encoding, or manual typing can
   all route around it. This is a limitation of the local-PreToolUse-hook
   design itself, not something an implementation can fix.
   — Lampson, B. W., "A Note on the Confinement Problem", *Communications
   of the ACM* 16(10), 1973, pp. 613–615,
   <https://dl.acm.org/doi/10.1145/362375.362389> (accessed 2026-09-10).
2. **It does not satisfy complete mediation.** This tool only mediates
   `PreToolUse` events for the `Bash` tool and `mcp__*` tools; it sees
   nothing on any other path (an agent hitting an API directly, a user
   working in a separate terminal, etc.).
   — Saltzer, J. H. & Schroeder, M. D., "The Protection of Information in
   Computer Systems", 1975,
   <https://www.cs.virginia.edu/~evans/cs551/saltzer/> (accessed 2026-09-10).
3. **Denylist-based detection is only useful for "detecting suspicious
   activity."** The `audit` subcommand is designed around this premise (for
   after-the-fact checks of existing files/Issues/PRs).
   — MITRE CWE-184, "Incomplete List of Disallowed Inputs",
   <https://cwe.mitre.org/data/definitions/184.html> (accessed 2026-09-10).

A high false-positive rate normalizes bypassing for both agents and humans,
which makes the tool functionally equivalent to not existing at all. If you
find a false positive, reach for the allowlist before tightening the
denylist's granularity.
— Rahman, A., Imtiaz, F., Storey, M.-A., Williams, L., "Why secret
detection tools are not enough: It's not just about false positives — An
industrial case study", *Empirical Software Engineering*, 2022,
<https://doi.org/10.1007/s10664-021-10109-y> (accessed 2026-09-10).

**A hook timeout is not something this tool can structurally prevent.**
Claude Code's official documentation states that "a hook that times out does
not block the tool call." In other words, if the hook process itself doesn't
finish in time, the operation goes through regardless of the verdict. This
is a host-CLI-side specification that publish-guard cannot address.
— Anthropic, "Hooks reference", <https://code.claude.com/docs/en/hooks>
(accessed 2026-09-10).

**Bypass: `PUBLISH_GUARD_ALLOW=1`**. `scan`/`scan-push`/`scan-bash-command`
pass immediately if the `PUBLISH_GUARD_ALLOW=1` environment variable is set.
This is an escape hatch for when you deliberately want to skip the check.
**The deny/ask reason text never names this env var** — because the
constrained party (the agent) could otherwise read the deny reason and
re-run the bypass itself. The existence of this bypass is meant to be known
only to the human who configures this tool.

## Development

- `./publish-guard selftest` / `./hooks/claude-adapter.sh --selftest` /
  `./adapters/codex-adapter.sh --selftest` / `./adapters/copilot-adapter.sh --selftest`
- `shellcheck -S error publish-guard hooks/*.sh adapters/*.sh`
- Pre-commit sanitization rules: [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT — [LICENSE](LICENSE)
