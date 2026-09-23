# publish-guard

A deny/ask gate that prevents AI coding agents from leaking company/private repository names onto public GitHub surfaces (git push, PR/Issue creation, MCP tool calls).

## Background

Born as a Claude Code PreToolUse hook (originally
`config/claude/hooks/public-publish-guard.sh` in
[tarotene/dotfiles](https://github.com/tarotene/dotfiles)), this repository
extracted the decision engine into an agent-agnostic CLI (`publish-guard`,
Bash) and a small Rust binary (`publish-guard-hook`) that translates each
host's PreToolUse payload into calls against that CLI. A thin Bash shim
(`hooks/pg-hook.sh`) is what each host actually registers — it locates
`publish-guard-hook` and execs into it, falling back to an `ask` verdict if
the binary isn't installed (see [Install](#install)).

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

**Marking a private repo as "prospectively public"** (#15): a personal side
project that's private only because it isn't polished yet — not because its
name or existence is sensitive — can be exempted from the hard-deny tier
with nothing more than `allow-stopwords.txt`; there is no separate
"prospective public" file or config tier, because this exemption is fully
covered by the stopword mechanism that already exists for false positives
in general:

```
$ cat >> ~/.config/publish-guard/allow-stopwords.txt <<'EOF'
# prospectively public — revert to repos.txt if this changes
my-side-project
EOF
```

What this does and does not cover:

- `allow-stopwords.txt` filters **both** `WORD_HARD` (hard-deny, bare repo
  names) and `WORD_WARN` (ask, bare org names) — so a stopword removes a
  live-swept private repo name from the hard-deny tier just as it would a
  `repos.txt` entry.
- Your **own personal repos** (the org derived from your authenticated `gh`
  user, not an entry in `orgs.txt`) never appear in `PLAIN_PATTERNS`
  (`org/repo`-shaped literal matches) in the first place — only
  `orgs.txt`/`repos.txt`-derived entries do. So for a personal repo, a bare
  stopword is enough; both the bare name and the `owner/repo` form pass.
- This does **not** apply to repos under an org listed in `orgs.txt` — those
  still hard-deny on the `org/repo` form via `PLAIN_PATTERNS`, which
  stopwords don't filter (intentional: `orgs.txt` encodes "definitely keep
  this secret," and a stopword shouldn't be able to fully undo that for a
  company/org repo).
- Precede the entry with a whole-line `#` comment recording *why* it's there
  and what to do if the decision reverses — a bare word in this file
  doesn't otherwise distinguish "known false-positive trigger" from "opted
  out of hard-deny on purpose." Only whole-line comments are stripped
  (`read_lines` skips lines that *start* with `#`); there's no inline
  trailing-comment syntax, so a comment needs its own line, as in the
  example above. If you commit to keeping the repo private, move the entry
  back out of `allow-stopwords.txt` and into `repos.txt` instead.

### `publish-guard-hook` binary (required for every host)

Every host's hook registration points at `hooks/pg-hook.sh`, a shim that
execs into the `publish-guard-hook` binary. Install it once per machine:

```
$ cargo install publish-guard-hook
```

The shim looks for it via `$PUBLISH_GUARD_HOOK_BIN`, then `PATH`, then
`~/.cargo/bin/publish-guard-hook` (in that order). **If none of those
resolve, the shim does not silently let the tool call through** — it prints
an `ask` verdict and exits 0, so an agent is stopped for confirmation rather
than the guard quietly doing nothing. This is a different failure mode from
the hook-timeout caveat below: a missing binary is something this repository
*can* detect and fail loud on, unlike a timeout, which is out of its hands
entirely.

### Claude Code

```
/plugin marketplace add tarotene/publish-guard
/plugin install publish-guard@publish-guard
```

`.claude-plugin/plugin.json` reads `hooks/hooks.json`, which registers
`hooks/pg-hook.sh --host=claude` on `PreToolUse` with a single compound
matcher `Bash|mcp__.*` (self-resolved via `${CLAUDE_PLUGIN_ROOT}`).

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
          {"type": "command", "command": "/path/to/publish-guard/hooks/pg-hook.sh --host=codex", "timeout": 20}
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
      {"type": "command", "bash": "/path/to/publish-guard/hooks/pg-hook.sh --host=copilot", "timeoutSec": 20}
    ]
  }
}
```

Copilot's `preToolUse` **has no matcher** (verified against a real
instance — it fires unconditionally on every tool call). Filtering happens
inside `publish-guard-hook` itself (checking whether `toolName == "bash"`).

## Usage

**`--cwd DIR`**: a global option, placed before the subcommand name, that
tells `resolve_repo_nwo`/`compute_push_diff_text`/`resolve_default_branch` to
treat `DIR` as the target repository's location instead of the hook
process's own cwd. `publish-guard-hook` passes this automatically for all
three hosts from the PreToolUse payload's `cwd` field (Claude, Codex, and
Copilot all expose one — Copilot's has been verified against a real
instance; see `src/host.rs` for the field paths). This exists because a
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

**Before making a private repo public, run `audit --since`**: the default
`audit` only looks at the **current working tree** (`git ls-files`), so it
can't see a denylisted string that was added in one commit and removed in a
later one — that content is still fully present in the repo's history, and
the moment you flip the repo's visibility to public, GitHub exposes the
whole history, not just the current tree (#11's motivating concern, closed
as already-implemented for the *added-lines* case, but this is the
already-known gap for the *history* case). `--since <rev>` additionally
scans every added line (plus new/renamed file paths and commit messages)
between `<rev>` and `HEAD`, reusing the same added-lines-only extraction
`scan-push` uses (#7 — removed lines still carry no leak-prevention value
here either):

```
$ publish-guard audit --since "$(git rev-list --max-parents=0 HEAD)"
```

**This intentionally isn't wired into CI.** Running `audit` in CI would mean
putting `orgs.txt`/`repos.txt` into an Actions secret so the workflow can
reconstruct the denylist — but that puts denylist data in a place this
project has decided it categorically shouldn't be (`## Install`'s "this
repository never commits denylist data," CONTRIBUTING's sanitization
rules). Run `audit --since` locally, by hand, right before you flip
visibility.

## Scope

This repository owns the decision-engine CLI (`scan`/`scan-push`/
`scan-bash-command`/`audit`), the `publish-guard-hook` binary that translates
each host's PreToolUse payload into calls against that CLI, the `pg-hook.sh`
shim each host actually registers, and the denylist/allowlist config-file
format.
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

- `cargo build` (builds `target/debug/publish-guard-hook`, which the Bash
  selftests below shell out to for command lexing — set
  `PUBLISH_GUARD_LEX_BIN=target/debug/publish-guard-hook` before running
  them if the binary isn't on `PATH`)
- `./publish-guard selftest` / `./hooks/pg-hook.sh --selftest`
- `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`
- `shellcheck -S error publish-guard hooks/*.sh`
- Pre-commit sanitization rules: [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT — [LICENSE](LICENSE)
