# Changelog

All notable changes to Forge are documented in this file.

Forge follows [Semantic Versioning](https://semver.org/) using `MAJOR.MINOR.PATCH` package versions and `v`-prefixed release tags.

## [Unreleased]

### Added

- Versioning policy based on Semantic Versioning.
- `ponytail` plugin install: real upstream `DietrichGebert/ponytail@4.7.0`
  ruleset shipped as a Forge skill (`ext/tests/fixtures/skills/ponytail.md`)
  + plugin manifest (`ext/tests/fixtures/plugins/ponytail/forge-plugin.toml`).
  `ext::skills::SkillDiscovery` and `ext::plugins::{PluginLoader,
  PluginRegistry}` now have a live install to round-trip against. Mirrored
  to `~/.forge/skills/ponytail.md` and `~/.forge/plugins/ponytail/` for
  end-user discovery.
- `ext/tests/ponytail_install.rs` — 6 integration tests that fail loud
  if upstream ever drops YAGNI / stdlib-first / lazy-senior persona, or if
  the Forge skill or plugin loader regresses on the install.

### Changed

- Standardized the current MVP version to `v0.100.0` across package metadata and project documentation.

## [v0.100.0] - 2026-06-14

### Added

- Initial MVP CLI loop with tool-observe-act flow.
- Anthropic provider implementation for the MVP provider path.
- File read, diff-edit, and command execution tools.
- Network-off sandbox baseline.
- AGENTS.md context loading.

### Changed

- Established `v0.100.0` as the canonical current MVP release, replacing the older conflicting MVP label.

[Unreleased]: https://github.com/forge-org/forge/compare/v0.100.0...HEAD
[v0.100.0]: https://github.com/forge-org/forge/releases/tag/v0.100.0
