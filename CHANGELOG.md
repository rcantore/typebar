# Changelog

All notable changes to typebar are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

<!--
How to maintain this file:
- Add every user-facing change under [Unreleased], in the right category
  (Added / Changed / Deprecated / Removed / Fixed / Security), ideally in the
  same PR as the change.
- On release, rename [Unreleased] to the new version with a date, e.g.
  `## [0.1.0] - 2026-06-23`, and start a fresh empty [Unreleased] above it.
-->

## [Unreleased]

### Added

- WYSIWYG inline rendering as you type: bold, italic, headings, bullet/ordered
  lists, blockquotes, links, and images.
- Three swappable keybinding presets: `standard` (modeless, default), `vim`
  (modal), and `wordstar` (classic chords), with per-key custom overrides.
- TOML configuration file for selecting the preset and layering key overrides.
- `--keys <preset>` command-line flag (precedence: flag → config → built-in
  default).
- `PageUp` / `PageDown` navigation.
- System clipboard integration.
- Bilingual README (English / Español).

[Unreleased]: https://github.com/rcantore/typebar/commits/main
