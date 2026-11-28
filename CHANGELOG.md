# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.1](https://github.com/tarotene/bleep/compare/v0.2.0...v0.2.1) — 2026-11-28

### Added

- **bleep**: Name the matched file and line in the scan-push reason ([#130](https://github.com/tarotene/bleep/pull/130))
- **bleep**: Name the ordinary rewrites in the repo-ref next step (@me for an assignee, a scratch place without the project name) ([#127](https://github.com/tarotene/bleep/pull/127))

### Fixed

- **changelog**: Fix the three breakages in the CHANGELOG template ([#133](https://github.com/tarotene/bleep/pull/133))
- **lex**: Do not split on && and || inside a [[ ]] expression ([#131](https://github.com/tarotene/bleep/pull/131))
- **bleep**: Do not deny the bare name of a repo named like your own login ([#129](https://github.com/tarotene/bleep/pull/129))

### Other

- **readme**: Reorder the README by what a new user does ([#135](https://github.com/tarotene/bleep/pull/135))
- **crates**: Make bleep-hook publishable to crates.io ([#134](https://github.com/tarotene/bleep/pull/134))
