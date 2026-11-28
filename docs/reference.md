# Reference: how the checks decide

The [README](../README.md) covers installing and configuring bleep. This page holds the detailed behavior of the checks, moved here from the README's former Usage section. The design principle behind the `gh` grammar is in [adr/0003-constructive-grammar.md](adr/0003-constructive-grammar.md).

## The `--cwd` option

A global option, placed before the subcommand name, that
tells `scan-push` (both forms) to treat `DIR` as the target repository's
location instead of the process's own cwd. `bleep-hook` still passes it for all
three hosts from the PreToolUse payload's `cwd` field (Claude, Codex, and
Copilot all expose one; the field path is in the `bleep-hook` host adapter), but
`scan-bash-command` no longer uses it: a PreToolUse hook runs as a separate
process **before** the shell command executes, so a `cd /other/repo && …` can't
be resolved from the hook's own cwd (#10, #14) — and bleep now doesn't try. A
`gh` post must name its destination (`-R`) and its body file by absolute path,
so no directory is needed; a `git push` is judged by the pre-push hook, which
runs in the right place by construction.

## How `scan-bash-command` classifies a compound command

`CMD` is split
into segments on `;`, `&&`, `||`, `|`, and newlines (quote-aware — `&&`
inside a quoted string is not treated as a separator). Each segment is
tokenized and the command word is found **by position** — assignments
(`X=1 gh …`), prefixes (`env`, `sudo`, `timeout`, `nohup`, …), the subshell
opener, and an absolute path to the binary are skipped — and the contents of
`sh|bash|zsh -c '…'`, `eval '…'`, `$(…)` and backticks are classified the same
way, recursively (depth-limited). Which strings count as **code** (looked at)
and which as **data** (not looked at) is decided by the shell's own rules
([docs/adr/0005-code-position-closure.md](adr/0005-code-position-closure.md)):
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
([docs/adr/0003-constructive-grammar.md](adr/0003-constructive-grammar.md)).
The commands in the grammar are the ones that carry free text to a public
surface: `pr create|edit|comment|review|close|merge|reopen`,
`issue create|edit|comment|close|reopen`, `release create|edit`, `repo edit`,
`gist create`, and `api` with a write method (`-X`/`--method` `POST`, `PUT` or
`PATCH`, or `-f`/`-F`/`--input` without a method). A `gh` command that is not in
that table — a read (`pr view`, `run watch`, a `gh api` GET), or an operation
with no free text (`pr ready`) — is not scanned, as before. So is a `gh api graphql`
whose `-f`/`-F query=` document is a literal read: it starts with `query` or `{`
and has no `mutation` (GraphQL carries reads and writes in the same POST, so the
method cannot tell them apart; `query="$Q"`, `query=@file` and `--input` stay
posts). So is a call that
only asks for usage (`--help`, or `-h` outside `gh repo edit`, where `-h` is
`--homepage`): it prints help and posts nothing. A `--help` sitting where a flag
value goes (`--title --help`) is consumed as that value, so it stays a post. The
`&` of `2>&1`, `<&0` and `&>file` is part of the redirection, not a command
separator, so a post followed by `2>&1` is still a standalone command. A post is
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
  ([docs/adr/0004-body-file-post-stands-alone.md](adr/0004-body-file-post-stands-alone.md))

Anything else is **denied** (reason id `gh-noncanonical`, with one of the
closed codes `no-repo`, `bad-repo`, `inline-body`, `inline-comment`,
`body-stdin`, `body-path`, `dynamic-value`, `unparsable`, `not-alone` in
the ledger's `detail`), and the reason text shows the canonical form: write the body to a
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
core.hooksPath=…` global option (reason id `push-hook-bypass`). Two forms whose
content cannot be read at PreToolUse are denied with their own reason id,
`opaque-exec` (#81; #115 split it from `gh-noncanonical` because it can fire without
any `gh` in the command, and its reason text carries no `gh` post rules): a command
name that is a variable (`f=gh; $f issue …`, `"$cmd" …`, `bash -c "$x"`), and text
piped into a shell (`echo '…' | bash`, `… | sh -s`; `cat <<'EOF' | bash` and
`… | bash script.sh` are not, since the body is inspected as code and a file is
run, respectively). Both have no everyday use and can be rewritten with a literal
`gh` ([docs/adr/0005-code-position-closure.md](adr/0005-code-position-closure.md)).
The forms the recursion does not reach (a deeper nesting,
`python -c 'subprocess.run(["gh", …])'`, a script on disk, an alias, a variable
after a prefix such as `env $f …`, a `|` inside an unquoted `$(…)`, a binary other
than `gh`) remain an approximation, within the scope this
tool already states below: a guardrail against accidents, not a boundary.

## What `scan-push` checks

`bleep scan-push --pre-push <remote-name>
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
