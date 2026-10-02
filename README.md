# bleep

<img src="docs/img/bleep.svg" alt="A black censor bar over the word BLEEP" width="360">

A deny/ask gate that prevents AI coding agents from leaking company/private repository names onto public GitHub surfaces (git push, PR/Issue creation, MCP tool calls).

Where it hooks in: `git push` is checked by git's own **pre-push hook**
(`hooks/pre-push`), which receives the exact refs and SHAs being pushed; `gh`
and MCP tool calls are checked by the agent host's **PreToolUse hook**
(`hooks/bleep.sh`). The design principle — shrink what is accepted to a small
grammar instead of inspecting arbitrary generated commands — is in
[docs/adr/0003-constructive-grammar.md](docs/adr/0003-constructive-grammar.md).

Formerly `publish-guard` — see [docs/adr/0001-name-bleep.md](docs/adr/0001-name-bleep.md) for why it was renamed.

## Background

Born as a Claude Code PreToolUse hook (originally
`config/claude/hooks/public-publish-guard.sh` in
[tarotene/dotfiles](https://github.com/tarotene/dotfiles), from this
repository's time as `publish-guard`), this repository
extracted the decision engine into an agent-agnostic CLI (`bleep`,
Bash) and a small Rust binary (`bleep-hook`) that translates each
host's PreToolUse payload into calls against that CLI. A thin Bash shim
(`hooks/bleep.sh`) is what each host actually registers — it locates
`bleep-hook` and execs into it, falling back to an `ask` verdict if
the binary isn't installed (see [Install](#install)).

## Install

**Upgrading from `publish-guard`**: this tool was renamed from
`publish-guard`/`publish-guard-hook` to `bleep`/`bleep-hook` (see
[docs/adr/0001-name-bleep.md](docs/adr/0001-name-bleep.md)). There is no
compatibility shim — move your config and state directories by hand, once:

```
$ mv ~/.config/publish-guard ~/.config/bleep
$ mv ~/.local/state/publish-guard ~/.local/state/bleep   # if it exists
```

Then reinstall the plugin/hooks under the new name (see below) and remove the
old `publish-guard@publish-guard` plugin install.

If you skip the move, `bleep` does not silently run without your org list.
While `orgs.txt` is missing and the old `~/.config/publish-guard/` directory
still exists, every publish check returns **ask** and says the migration is
incomplete.

This repository never commits denylist data (org names, repo names). You
place your own under `$XDG_CONFIG_HOME/bleep/` (defaults to
`~/.config/bleep/`). **The only thing you realistically have to
write by hand is one line in `orgs.txt` with your org name** — your own
private/internal repository names are live-derived from the org name you
wrote there plus the owner of your authenticated `gh` user, using your own
`gh` credentials. Nobody carries around a literal list of repo names.

```
$ mkdir -p ~/.config/bleep
$ echo acme > ~/.config/bleep/orgs.txt   # write only your company's org name
```

`orgs.txt` itself is **required**. When it does not exist, `scan`,
`scan-push`, and the publish paths of `scan-bash-command` return **ask**
instead of passing. The rest of the denylist is never empty on its own (your
own private repos are always derived), so "no org list" would otherwise go
unnoticed. If you have no org to protect, create an empty `orgs.txt` to say
so explicitly:

```
$ touch ~/.config/bleep/orgs.txt   # explicit opt-out: no company org
```

If your `orgs.txt`/`repos.txt` are generated declaratively (e.g. a
home-manager module, as opposed to hand-placed), a missing file more often
means the generation didn't run on this host than an intentional opt-out —
re-apply whatever generates them instead of writing one by hand. Either way,
deciding what org name to register (or that there is none) is a human
decision: an agent that hits this `ask` should surface it and wait, not
recreate the file itself from a backup or a guess.

Other config files you can add (all optional, one entry per line, `#`
comments and blank lines ignored):

| File | Purpose |
|---|---|
| `orgs.txt` | Company/other-party org names (this is realistically the only one you write by hand). The file must exist; an empty file is an explicit opt-out |
| `repos.txt` | Explicit `org/repo` references (optional) |
| `allow-stopwords.txt` | Words excluded from the denylist (`.github` is a built-in default) |
| `allow-regexes.txt` | If this regex matches the scanned text, it disables **only the ask** verdict (it does not loosen deny) |
| `allow-paths.txt` | Regex of file paths excluded from `audit` |

**Short repo names are downgraded to `ask` by default** (`BLEEP_SOFT_MAXLEN`,
default `4`, #44): a live-swept private repo name that's `BLEEP_SOFT_MAXLEN`
characters or shorter — the kind of short, generic name that's more likely to
collide with ordinary prose in an unrelated public repo (`gizmo.json`,
`app-gizmo`, a short code name reused across many repos) — is checked as
`ask` instead of hard-`deny`. This is not a dictionary lookup (no bundled or
system word list is consulted, so the check is the same on every host
regardless of what's installed); it's purely a length threshold, applied
**after** `allow-stopwords.txt` filtering, so a stopworded short name is
still fully exempt (see below). To keep recall from dropping, the downgraded
name's `owner/repo` form is still hard-`deny`ed (a word-boundary match, not
`PLAIN_PATTERNS`' substring match) — so an explicit `owner/repo` reference to
the same repo is unaffected by the downgrade; only the bare, ambiguous name
is softened. Set `BLEEP_SOFT_MAXLEN=0` to disable the downgrade and
hard-deny every live-swept name regardless of length (the pre-#44 behavior).
The cache this reads from (`private-repos-cache.txt`) now stores `owner/name`
per line instead of a bare name, so this downgrade has an owner to fall back
to; an old-format cache from before #44 is detected and refreshed
automatically the next time it's read.

**Marking a private repo as "prospectively public"** (#15): a personal side
project that's private only because it isn't polished yet — not because its
name or existence is sensitive — can be exempted from the hard-deny tier
with nothing more than `allow-stopwords.txt`; there is no separate
"prospective public" file or config tier, because this exemption is fully
covered by the stopword mechanism that already exists for false positives
in general:

```
$ cat >> ~/.config/bleep/allow-stopwords.txt <<'EOF'
# prospectively public — revert to repos.txt if this changes
my-side-project
EOF
```

What this does and does not cover:

- `allow-stopwords.txt` filters **both** `WORD_HARD` (hard-deny, bare repo
  names) and `WORD_WARN` (ask, bare org names) — so a stopword removes a
  live-swept private repo name from the hard-deny tier just as it would a
  `repos.txt` entry. It's applied **before** the `BLEEP_SOFT_MAXLEN` length
  downgrade (#44), so a stopworded short name never reaches `WORD_SOFT`
  either — both the bare form and its `owner/repo` form (`NWO_HARD`) stay
  fully exempt, not merely downgraded to `ask`.
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
- `WORD_HARD`/`WORD_WARN` word-boundary matching treats a file extension
  (a `.` immediately followed by an alphanumeric run) and a hyphen as
  **non**-boundary characters, unlike a plain `grep -w` (#35). Without this,
  a private repo name that happens to be a single common word (e.g.
  `gizmo`) false-positives on any unrelated public repo's
  `gizmo.json`/`app-gizmo`/`gizmo-app`. A `.` that ends a sentence
  (not followed by an extension-like run) still counts as a boundary, so
  `see gizmo.` still matches.
- Precede the entry with a whole-line `#` comment recording *why* it's there
  and what to do if the decision reverses — a bare word in this file
  doesn't otherwise distinguish "known false-positive trigger" from "opted
  out of hard-deny on purpose." Only whole-line comments are stripped
  (`read_lines` skips lines that *start* with `#`); there's no inline
  trailing-comment syntax, so a comment needs its own line, as in the
  example above. If you commit to keeping the repo private, move the entry
  back out of `allow-stopwords.txt` and into `repos.txt` instead.

### `bleep-hook` binary (required for every host)

Every host's hook registration points at `hooks/bleep.sh`, a shim that
execs into the `bleep-hook` binary. Install it once per machine:

```
$ cargo install bleep-hook
```

The shim looks for it via `$BLEEP_HOOK_BIN`, then `PATH`, then
`~/.cargo/bin/bleep-hook` (in that order). **If none of those
resolve, the shim does not silently let the tool call through** — it prints
an `ask` verdict and exits 0, so an agent is stopped for confirmation rather
than the guard quietly doing nothing. This is a different failure mode from
the hook-timeout caveat below: a missing binary is something this repository
*can* detect and fail loud on, unlike a timeout, which is out of its hands
entirely.

### git pre-push hook (required for push checks)

`git push` is **not** inspected by the PreToolUse hook (see
[docs/adr/0003-constructive-grammar.md](docs/adr/0003-constructive-grammar.md)):
guessing the pushed range from the command string failed on new branches, other
worktrees and shallow clones (#67/#69/#73). The check lives in git's pre-push
hook instead, which gets the exact `<local-ref> <local-sha> <remote-ref>
<remote-sha>` lines on stdin whatever the command looked like. Place
`hooks/pre-push` where git looks for hooks — the directory `core.hooksPath`
points to, or `.git/hooks/` — as a copy or a symlink:

```
$ ln -s /path/to/bleep/hooks/pre-push "$(git config core.hooksPath || echo .git/hooks)/pre-push"
```

If you already have a pre-push hook, call `bleep scan-push --pre-push "$1" "$2"`
from it before anything else reads stdin (git's stdin is inherited by the
child), and block the push on any non-zero exit — a pre-push hook has no way to
ask, so `ask` (exit 1) blocks too.
**Without this hook, pushes are not checked at all**; `bleep doctor` reports it.
PreToolUse only denies the forms that switch the hook off: `git push
--no-verify` and `git -c core.hooksPath=… push`.

### Claude Code

```
/plugin marketplace add tarotene/bleep
/plugin install bleep@bleep
```

`.claude-plugin/plugin.json` reads `hooks/hooks.json`, which registers
`hooks/bleep.sh --host=claude` on `PreToolUse` with a single compound
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
    {"source": "github", "repo": "tarotene/bleep"}
  ],
  "enabledPlugins": {
    "bleep@bleep": true
  }
}
```

Denylist data (`orgs.txt`, etc.) does not travel through this distribution
path — each user still has to write their org name into their own
`~/.config/bleep/orgs.txt` (this is intentional; see "This is not a
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
          {"type": "command", "command": "/path/to/bleep/hooks/bleep.sh --host=codex", "timeout": 20}
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
      {"type": "command", "bash": "/path/to/bleep/hooks/bleep.sh --host=copilot", "timeoutSec": 20}
    ]
  }
}
```

Copilot's `preToolUse` **has no matcher** (verified against a real
instance — it fires unconditionally on every tool call). Filtering happens
inside `bleep-hook` itself (checking whether `toolName == "bash"`).

## Usage

**`--cwd DIR`**: a global option, placed before the subcommand name, that
tells `scan-push` (both forms) to treat `DIR` as the target repository's
location instead of the process's own cwd. `bleep-hook` still passes it for all
three hosts from the PreToolUse payload's `cwd` field (Claude, Codex, and
Copilot all expose one; see `src/host.rs` for the field paths), but
`scan-bash-command` no longer uses it: a PreToolUse hook runs as a separate
process **before** the shell command executes, so a `cd /other/repo && …` can't
be resolved from the hook's own cwd (#10, #14) — and bleep now doesn't try. A
`gh` post must name its destination (`-R`) and its body file by absolute path,
so no directory is needed; a `git push` is judged by the pre-push hook, which
runs in the right place by construction.

**How `scan-bash-command` classifies a compound command**: `CMD` is split
into segments on `;`, `&&`, `||`, `|`, and newlines (quote-aware — `&&`
inside a quoted string is not treated as a separator). Each segment is
tokenized and the command word is found **by position** — assignments
(`X=1 gh …`), prefixes (`env`, `sudo`, `timeout`, `nohup`, …), the subshell
opener, and an absolute path (`/usr/bin/gh`) are skipped — and the contents of
`sh|bash|zsh -c '…'`, `eval '…'`, `$(…)` and backticks are classified the same
way, recursively (depth-limited). Which strings count as **code** (looked at)
and which as **data** (not looked at) is decided by the shell's own rules
([docs/adr/0005-code-position-closure.md](docs/adr/0005-code-position-closure.md)):
a heredoc body belongs to the command it is written on instead of being split
into commands; with a quoted delimiter (`<<'EOF'`) it is data, with an unquoted
one only its `$(…)`/backtick substitutions are code (POSIX 2.7.4), and it is
code as a whole when a shell reads it from stdin (`bash <<'EOF'`,
`cat <<'EOF' | bash`, `eval "$(cat <<'EOF' …)"`); single-quoted text, `$'…'`,
escaped `\$(` and `#` comments are data. A segment that cannot be tokenized (an
unclosed quote) but looks like a `gh` post is denied as `unparsable` instead of
passed through. `cargo test` checks the recognizer against what a real `bash`
executes (a stub `gh` on `PATH`): every executed `gh` must be recognized.

Instead of reconstructing what an arbitrary `gh` command will publish, bleep
accepts a **small grammar** and denies everything outside it
([docs/adr/0003-constructive-grammar.md](docs/adr/0003-constructive-grammar.md)).
The commands in the grammar are the ones that carry free text to a public
surface: `pr create|edit|comment|review|close|merge|reopen`,
`issue create|edit|comment|close|reopen`, `release create|edit`, `repo edit`,
`gist create`, and `api` with a write method (`-X`/`--method` `POST`, `PUT` or
`PATCH`, or `-f`/`-F`/`--input` without a method). A `gh` command that is not in
that table — a read (`pr view`, `run watch`, a `gh api` GET), or an operation
with no free text (`pr ready`) — is not scanned, as before. A post is
**canonical** when:

- the destination is a literal `-R|--repo OWNER/REPO` (position-independent,
  last one wins); a PR/Issue URL positional also works for `pr|issue`, a
  positional `OWNER/REPO` for `repo edit`; `gist` has none; for `gh api` it is
  the literal `repos/<owner>/<repo>/…` path (`{owner}` placeholders resolve
  through the cwd's origin, so they are outside the grammar)
- the body comes only from a file given as an **absolute, literal path**
  (`--body-file`, `-F`; `--notes-file` for `release`; the positional files of
  `gist create`; `-F key=@path` / `--input` for `gh api`) — never from
  `--body`/`-b`/`--notes`/`-n`, a `close|reopen --comment`, or stdin (`-`)
- every other value is literal: no `$(…)`, backticks, or `$VAR`
- a post that reads a body file **stands alone**: it is the whole command (an
  `env`/`timeout` prefix is fine), not chained with `&&`, `;`, `|` or a newline,
  and not inside `sh -c`, `eval` or `$(…)`. A preceding command could rewrite the
  file between the scan and the post
  ([docs/adr/0004-body-file-post-stands-alone.md](docs/adr/0004-body-file-post-stands-alone.md))

Anything else is **denied** (reason id `gh-noncanonical`, with one of the
closed codes `no-repo`, `bad-repo`, `inline-body`, `inline-comment`,
`body-stdin`, `body-path`, `dynamic-value`, `unparsable`, `not-alone` in the ledger's
`detail`), and the reason text shows the canonical form: write the body to a
file first, then `gh issue comment 5 -R OWNER/REPO --body-file /abs/path.md`
(to close with a comment, comment first and then `gh issue close`). bleep does
not guess a destination from the cwd, track `cd`, or resolve variables any more
— a form that would need that is simply outside the grammar. A post whose
destination is a literal PRIVATE/INTERNAL repo is not checked at all (the
grammar included); with several posts in one command, every post is looked at
and the denylist match runs on the whole command text plus the body files of the
posts that go to a public (or unresolvable, e.g. `gh api graphql`) destination.
The scanned text is `CMD` with any segment that is **exactly** `cd <single
token>` removed — so a `cd`'s path argument merely containing a private repo
name no longer triggers a false-positive hard-deny (#9) — and, within a `gh`
segment with an explicit `--repo`/`-R`, that flag and its value removed too
(#43: the destination a command names is not itself a leak). Everything the
destination doesn't cover (titles, other values, and any oddly-split fragment)
stays in the scanned text. A `git push` segment is **not** inspected for its
range or target: the range is decided by the pre-push hook from git's own input
(see "What `scan-push` checks" below). The only push forms that are denied here
are the ones that switch that hook off — `--no-verify`, or a `-c
core.hooksPath=…` global option (reason id `push-hook-bypass`). The forms the
recursion does not reach (a deeper nesting, `python -c 'subprocess.run(["gh", …])'`,
a command name held in a variable, text piped into a shell
(`echo '…' | bash`), a script on disk, a binary other than `gh`) remain an
approximation, within the scope this
tool already states below: a guardrail against accidents, not a boundary.

`bleep scan`/`scan-push`/`scan-bash-command` all share the same exit
code contract (for anyone scripting against this themselves):

| exit code | meaning | stdout |
|---|---|---|
| 0 | pass (no denylist match) | none |
| 1 | ask (a bare org-name match, or a bare repo-name match short enough to be downgraded — could collide with a legitimate use, see `BLEEP_SOFT_MAXLEN` above) | one-line reason |
| 2 | deny (an explicit org/repo reference or a specific repo-name match — almost certainly an unintended leak) | one-line reason |

**What `scan-push` checks**: `bleep scan-push --pre-push <remote-name>
<remote-url>` (called by `hooks/pre-push`, with git's stdin) inspects the
**added lines** (plus new/renamed file paths) and the full commit message of
each commit in the range being pushed. The range comes from the pre-push input,
not from guessing: `remote-sha..local-sha` when the remote's current tip exists
locally (an update of an existing branch); otherwise (a new branch, a shallow
clone whose remote tip isn't present, a remote that is ahead) `local-sha --not
--remotes=<remote>`, i.e. everything the remote's tracking refs don't already
have, or the whole history if there are none. Deleting a branch pushes no
content and is skipped. Visibility (PRIVATE/INTERNAL skips the check) is read
from the remote's URL. When a deny/ask comes out, the reason names the short
SHA of the first commit that matched (never the matched word), so a false
positive can be told apart from a real one (`git show <sha>`). Plain `bleep
scan-push` (no arguments) is the older compatibility path — it checks
`merge-base(HEAD, origin/<default>)..HEAD` of the current directory and asks
when that can't be computed; switch your hook to `--pre-push`. **Removed lines are not scanned** (#7 — this addresses
the problem where a fix that removes a line containing a denylisted name
would itself get denied). Deleted content is either already on the base
(already public) or was already scanned as an added line in an earlier
commit within the same push range, so scanning removed lines has no
leak-prevention value. Because this looks at commits (`git log -p`), not the
net diff, adding a secret in one commit and removing it in a later commit
within the same branch still gets denied once you push, since the
intermediate commit's content becomes public regardless.

**A caveat about `audit --remote`**: `bleep audit --remote` fetches
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
$ bleep audit --since "$(git rev-list --max-parents=0 HEAD)"
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
`scan-bash-command`/`audit`), the `bleep-hook` binary that translates
each host's PreToolUse payload into calls against that CLI, the `bleep.sh`
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
   `PreToolUse` events for the `Bash` tool and `mcp__*` tools, plus git's
   pre-push hook; it sees nothing on any other path (an agent hitting an API
   directly, a user working in a separate terminal, a push whose hook was
   disabled through the environment, etc.).
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
is a host-CLI-side specification that bleep cannot address.
— Anthropic, "Hooks reference", <https://code.claude.com/docs/en/hooks>
(accessed 2026-09-10).

**Bypass: `BLEEP_ALLOW=1`**. `scan`/`scan-push`/`scan-bash-command`
pass immediately if the `BLEEP_ALLOW=1` environment variable is set.
This is an escape hatch for when you deliberately want to skip the check.
**The deny/ask reason text never names this env var** — because the
constrained party (the agent) could otherwise read the deny reason and
re-run the bypass itself. The existence of this bypass is meant to be known
only to the human who configures this tool.

## Verdict ledger (local only, never sent anywhere)

Every `deny`/`ask` verdict from `scan`/`scan-push`/`scan-bash-command`
(including the fail-loud `ask`s such as a missing `orgs.txt`) is appended,
one JSON object per line, to
`$XDG_STATE_HOME/agent-verdicts/bleep.jsonl` (default
`~/.local/state/agent-verdicts/bleep.jsonl`). Nothing is transmitted
anywhere — this exists so an agent that keeps getting blocked by the same
judgment can have that fact surfaced locally and, if you choose to, turned
into a bug report against this repository, instead of silently working
around the denial. See
[tarotene/dotfiles docs/adr/](https://github.com/tarotene/dotfiles/tree/main/docs/adr)
(search for "agent-verdict-ledger") for the consuming side and the
canonical [JSON Schema](https://github.com/tarotene/dotfiles/blob/main/docs/schemas/agent-verdict.schema.json)
for the record shape.

- **What is written**: a closed-vocabulary `reason_id` and `match_class`,
  the host name, an optional session id, the invoked tool name, and an
  HMAC-SHA256 hash (keyed by a random, machine-local key at
  `$XDG_STATE_HOME/agent-verdicts/hmac-key`, generated on first use) of
  whichever denylist term matched.
- **What is never written**: the command text, `cwd`, the plaintext
  matched term, or the deny/ask reason text itself. Writing any of these
  would defeat this tool's own purpose.
- **Opt out**: set `DO_NOT_TRACK` (any non-empty value) or
  `BLEEP_NO_LEDGER=1`. A write failure never affects the verdict itself
  (fail-open).

**Diagnosing a repeated denial: `bleep explain TERM_HASH`** (#44). Because
the ledger only ever stores an HMAC of the matched term, an agent that keeps
getting denied by the same judgment has no way to identify — even by
brute-forcing every word in its own diff — which denylist entry actually
matched; the false-positive reports filed against this tool as a result
could never name the offending word. Run this **on your own terminal**,
never inside an agent session, since it prints denylist entries (org/repo
names) in plaintext:

```
$ jq -r 'select(.match_class != "none") | .term_hash' \
    ~/.local/state/agent-verdicts/bleep.jsonl | tail -1
4fe91aa1f4f5089de7f37664256681d75908f013dde5bec9cb43cc0e67b0837f
$ bleep explain 4fe91aa1f4f5089de7f37664256681d75908f013dde5bec9cb43cc0e67b0837f
word-hard	gizmo
```

This rehashes every current denylist entry with the same machine-local key
and prints whichever one matches, alongside its `match_class`. It does not
touch `match_verdict` and writes nothing to the ledger — it is purely a
read of the current denylist, not a re-run of any verdict. **The deny/ask
reason text never mentions this subcommand**, for the same reason it never
mentions `BLEEP_ALLOW` (see above): the constrained party should not be
able to read its own way out of a false positive.

## Checking your install

`bleep doctor` prints `ok:` / `NG:` lines and exits 1 on any `NG`. It checks that
`bleep-hook` runs, that its lex output version matches the `bleep` script (a
mismatched pair would otherwise misread each other's output; `scan-bash-command`
also asks in that case, with reason id `lex-protocol-mismatch`), that the
effective pre-push hook (`core.hooksPath`, else `.git/hooks/`; `BLEEP_PRE_PUSH_HOOK`
points it at another file) calls `scan-push --pre-push` — without it, pushes are
not checked — and that `hooks/hooks.json` matches both `Bash` and `mcp__` tools
(`BLEEP_HOOKS_JSON` points it at another file). Design notes:
[docs/adr/0002-gh-intent-and-layers.md](docs/adr/0002-gh-intent-and-layers.md).

## Development

- `cargo build` (builds `target/debug/bleep-hook`, which the Bash
  selftests below shell out to for command lexing — set
  `BLEEP_LEX_BIN=target/debug/bleep-hook` before running
  them if the binary isn't on `PATH`)
- `./bleep selftest` / `./hooks/bleep.sh --selftest`
- `cargo test` / `cargo clippy --all-targets -- -D warnings` / `cargo fmt --check`
- `shellcheck -S error bleep hooks/*.sh`
- Pre-commit sanitization rules: [CONTRIBUTING.md](CONTRIBUTING.md)

## License

MIT — [LICENSE](LICENSE)
