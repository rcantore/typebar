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

### Fixed

- Zen mode now has a top margin so the text no longer hugs the top edge,
  mirroring the extra left margin zen already had (both compensate for the
  border that zen hides). Whitepaper keeps its larger sheet margin.

## [0.2.0] - 2026-06-28

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
- Buffer tab bar across the top (shown with 2+ open buffers), with the active
  buffer highlighted. Switch buffers with `Ctrl-PageDown`/`Ctrl-PageUp` (next /
  previous, wrapping); remappable via `next-buffer`/`prev-buffer`. Optional mouse
  support (`[ui] mouse = true` in the config, off by default) makes the tabs
  clickable.
- Command palette: `Ctrl-A` opens a fuzzy finder over the editor's commands,
  showing each command's name and its current shortcut, and runs the chosen one
  on Enter (also a way to discover the keybindings). Remappable via the
  `open-palette` action.
- Whitepaper mode: a typewriter-like "sheet of paper" that orchestrates zen, a
  monochrome ink-on-paper theme (hierarchy from weight, not color), and a
  centered fixed-width column. Toggle from the *view* submenu — `Ctrl-O W`
  (standard / wordstar) or `z w` (vim); `Esc` exits in the modeless presets.
  Remappable via the `toggle-whitepaper` action. Draws a synthetic cursor so it
  stays visible on the light background.
- In-editor HTML export: the *Export HTML* command (command palette) exports the
  current buffer to `<file>.html` without leaving the editor, reporting the
  result in the status bar. Remappable via the `export-html` action.
- `-h` / `--help` command-line flag: prints usage and the available options, then
  exits. Previously unknown flags were silently ignored.

### Changed

- Leaner toolbar: trimmed to the essentials plus the two submenus (Save,
  Commands…, Search, Format…, View…, Quit) so it no longer overflows and clips
  the Quit hint; the rest of the commands are discoverable via the command
  palette (`Ctrl-A`) and their keys.

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

[Unreleased]: https://github.com/rcantore/typebar/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/rcantore/typebar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rcantore/typebar/releases/tag/v0.1.0
