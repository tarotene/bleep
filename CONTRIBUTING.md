# Contributing

See [README](README.md) for what this repository is and how to set up a
development environment.

## Issues

Judging question: Does this change improve detection/judging of "accidental
name leaks" on the PreToolUse surfaces this tool already mediates?

Accepted:
- fix(scan-push): refspec parsing misses a force-push form
- feat(adapter): follow Copilot CLI's MCP tool naming in the matcher

Rejected:
- feat: also monitor browser-based posting
- feat(audit): add API token leak detection

## Pull requests

Fork this repository, create a topic branch, and open a pull request against
`main`. Before submitting, run the checks listed in the README's
[Development](README.md#development) section (`selftest` and `shellcheck -S
error`) and make sure both pass.

**Sanitization rules (canonical for this public repository)**. This rule
duplicates, verbatim, §3 of `config/claude/skills/skill-gardening/SKILL.md`
in [tarotene/dotfiles](https://github.com/tarotene/dotfiles) for this
repository's use. The rule itself is an instance of this tool's own design
principle — "never commit denylist data" — and applies to every
contribution.

This repository is public, so it must never carry any company-internal
information. Code, README, Issues/PRs, and commit messages must all satisfy:

- **Zero proper nouns**: no company names, product names, machine/project
  names, real repository names, personal names, or Issue/PR numbers. Tests
  and examples may only use obviously fictional org/repo names like `acme`
  or `secret-project`.
- **Zero internal URLs**: no links to internal resources (publicly
  documented references, like official docs, are the exception).
- **No real artifacts**: no screenshots, copies of real files, or real data.
- **Never commit denylist/allowlist config files**: `orgs.txt`, `repos.txt`,
  and `allow-*.txt` are files this tool reads at runtime, and are also in
  this repository's `.gitignore`. These, and anything derived from real
  `gh`/`git` state, stay confined to a `mktemp -d` inside selftest.
  Static mock payloads under `tests/fixtures/` (PreToolUse JSON, the stub
  `gh` script) are fine to commit as long as they only use the fictional
  names above — they aren't denylist/allowlist config, they're inputs the
  selftest feeds to it.
- **Final check before committing**: re-read the diff specifically looking
  for proper nouns and URLs. When in doubt, leave it out.

**Before changing this tool**. `bleep` is designed around the
README's "This is not a security boundary" premise (Lampson 1973, Saltzer &
Schroeder 1975, CWE-184). When adding new detection logic:

- Never let an indeterminate verdict silently pass (fail-loud). Look at the
  existing `PUSH_DIFF_FAIL_REASON` and the read-failure handling in
  `cmd_scan`/`cmd_scan_push` for the pattern.
- Never name the bypass mechanism (`BLEEP_ALLOW=1`) in a deny
  reason's text.
- Both `./bleep selftest` and `shellcheck -S error bleep`
  must pass.
