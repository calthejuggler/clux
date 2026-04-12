# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.6](https://github.com/calthejuggler/clux/compare/v0.1.5...v0.1.6) - 2026-04-12

### Added

- add clap CLI, anyhow errors, and crates.io publishing

### Other

- use PAT for release-plz PRs to trigger CI checks
- add workflow_dispatch trigger to release-plz
- add contributing guide, PR template, and issue templates

## [0.1.5](https://github.com/calthejuggler/clux/compare/v0.1.4...v0.1.5) - 2026-04-12

### Added

- separate lib/bin functions

### Fixed

- re-stage files after cargo fmt in pre-commit hook

### Other

- fix release-plz config
- add even more tests
- add more tests
- add test suite
- add fail-fast false to release matrix

## [0.1.4](https://github.com/calthejuggler/clux/releases/tag/v0.1.4) - 2026-04-12

### Added

- move select and pick logic into the Rust binary
- add fzf override
- sort sessions by message timestamp
- use last message, instead of first
- show mode, tasks and sub-agents in claude-view
- add claude-first fuzzy finder
- add session filtering
- add customisable formatting
- add customisable bind key
- implement clux mvp

### Fixed

- detect active state for idle sessions with child processes
- recursive walk and UTF-8 support on MacOS
- align table columns
- read state from last message
- rename shadowed variable
- always run install script on plugin load to pick up new versions
- disable crates.io publishing in release-plz config
- use colon-suffixed session targets for tmux set-option

### Other

- bump version
- add screenshots to README
- fix release-plz workflow
- update README with new features
- deduplicate pane queries and build tree once
- release v0.1.2 ([#1](https://github.com/calthejuggler/clux/pull/1))
- add pre-commit hooks
- debump version to v0.1.1
- add release-plz workflow
- bump version to v0.2.0
- update github action versions
- improve rustisms
- add readme and license
- initial commit

## [0.1.2](https://github.com/calthejuggler/clux/compare/v0.1.1...v0.1.2) - 2026-04-07

### Fixed

- use colon-suffixed session targets for tmux set-option

### Other

- add pre-commit hooks
