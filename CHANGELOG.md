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

- Word count in the status bar (Unicode-aware), showing the document total and
  the selected count while a selection is active.
- Zen / focus mode: hides all chrome (border, toolbar, status bar) to leave only
  the text. Toggled from the new "view" submenu — `Ctrl-O Z` (standard and
  wordstar, echoing WordStar's onscreen-format prefix) or `z z` (vim, its view
  command prefix). In the modeless presets `Esc` also exits. Remappable via the
  `toggle-zen` action.
- Catppuccin Latte light theme, selectable with `theme = "latte"` in the config
  (joins the dark `frappe` and `mocha` palettes), and toggleable at runtime from
  the *view* submenu — `Ctrl-O L` (standard / wordstar) or `z l` (vim) flips
  between the configured dark theme and Latte. Remappable via the `toggle-light`
  action.
- HTML export: `typebar <file> --export-html` converts the Markdown to a
  standalone HTML document (CommonMark via pulldown-cmark, with tables,
  strikethrough, footnotes and task lists) and exits without opening the editor.
- Multiple open files with a fuzzy file switcher: `Ctrl-G` opens a centered
  finder over the project files (current directory, recursive, skipping
  `.git`/`target`/`node_modules`/hidden) and the open buffers; type to filter,
  arrows or `Ctrl-N`/`Ctrl-P` to move, Enter to open or switch, Esc to cancel.
  Bound in all three presets; remappable via the `open-switcher` action.
- New file: `Ctrl-N` opens a fresh empty buffer (`untitled.md`), focused, over the
  multi-buffer workspace. Remappable via the `new-buffer` action.
- Command palette: `Ctrl-A` opens a fuzzy finder over the editor's commands,
  showing each command's name and its current shortcut, and runs the chosen one
  on Enter (also a way to discover the keybindings). Remappable via the
  `open-palette` action.

## [0.1.0] - 2026-06-24

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

[Unreleased]: https://github.com/rcantore/typebar/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/rcantore/typebar/releases/tag/v0.1.0
