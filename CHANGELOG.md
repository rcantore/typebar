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

- Soft wrap: lines longer than the viewport now wrap visually at the available
  width (word-aware, grapheme-safe, CJK/emoji never split) in every mode, with
  the cursor, scroll, selection and search highlights following the wrapped
  rows. This fixes text typed past the whitepaper column (or in a narrow
  terminal) becoming invisible: typebar previously had neither wrap nor
  horizontal scroll. Table grids clip instead of wrapping, and the file on disk
  is untouched (purely visual, no hard line breaks inserted).

## [0.4.0] - 2026-07-08

### Added

- Cycle keybinding presets at runtime: the *Cycle keybindings* command
  (`Ctrl-O K`, or `z k` in vim; also in the command palette) rotates between
  standard, vim, and wordstar without restarting, re-applies your per-key
  overrides, and persists the choice to the config file.
- PDF export: the *Export PDF* command (command palette) writes a print-ready
  HTML (print CSS plus an automatic `window.print()`) to the system temp dir
  and opens it in the default browser; save it as PDF from the print dialog.
  New `export::to_html_print` in typebar-core. No new dependencies.
- Runtime theme picker (`Ctrl-O T`, or `z t` in vim) with fuzzy search and
  live preview; the chosen theme persists to the config file.
- Five new built-in themes: Dracula, Tokyo Night, Nord, Gruvbox, and Solarized.

### Changed

- Overlay polish (command palette and switcher): rounded borders, accent-colored
  border from the theme, hint footers, and a dimmed backdrop that keeps the
  document visible instead of clearing to black.

## [0.3.2] - 2026-07-06

### Fixed

- The `typebar` and `typebar-core` crates now ship a README on crates.io. After
  the move to a Cargo workspace, the README lived at the repository root, outside
  each crate's directory, so the published packages had no README. Both crates
  now point at a README via the `readme` field (the app README for `typebar`, a
  dedicated one for `typebar-core`).

## [0.3.1] - 2026-07-03

### Changed

- Internal restructure into a Cargo workspace, with the editor's core (document
  model, buffers, search, fuzzy, files, markdown, export, i18n, text) extracted
  into its own `typebar-core` crate. No behavior or installation changes; the
  published package is still `typebar`.

## [0.3.0] - 2026-07-01

### Added

- Close the active buffer with `Ctrl-W` (all presets; remappable via
  `close-buffer`). If it has unsaved changes, a prompt asks first — `[s]` save &
  close, `[d]` discard, `[c]` cancel. Closing the only buffer replaces it with a
  fresh empty one.
- Table rendering. Markdown pipe tables render as an aligned grid with
  box-drawing borders: column widths from content, a header separator from the
  `|---|` row, and a full frame whose top/bottom borders reuse the blank lines
  around the table (no extra lines, so the cursor still maps 1:1). Alignment
  (`:--`, `--:`, `:-:`) is respected and the header is bold. The row under the
  cursor stays raw Markdown for editing, like the rest of the WYSIWYG view.

### Changed

- File switcher moved off `Ctrl-G` (collides with zellij's lock toggle) to
  `Ctrl-E` in the standard and vim presets; in wordstar, where `Ctrl-E` is
  cursor-up, it's the `Ctrl-K F` chord ("find file"). `Ctrl-G` still works in
  standard/vim as a legacy alias.
- Fuzzy switcher lists open buffers first (most relevant to reach), then the
  project files, deduplicated. Candidates that aren't safely on disk (unsaved
  changes, or a never-saved buffer) are marked with `[+]`.
- The unsaved marker `[+]` now means "not safely on disk" — unsaved changes OR a
  never-saved buffer — consistently in the status bar and the switcher.
- Minimal chrome: the editor no longer draws a frame around itself; a subtle rule
  separates the writing area from the toolbar/status. Toolbar, themes and status
  bar are unchanged.

### Fixed

- Code blocks now render as a box. Fenced (` ``` `) and indented code blocks fill
  with the code background as a rectangle sized to the widest line, instead of
  looking like plain prose. In level 2 the opening/closing fences collapse into
  the box (like inline backticks do) on inactive lines, and reappear when the
  cursor is on them. Syntax highlighting is still pending.

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

### Fixed

- Top margin so the text no longer hugs the top edge, in both normal and zen
  modes (a row of air, mirroring the existing left margin). Whitepaper keeps its
  larger sheet margin.

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

[Unreleased]: https://github.com/rcantore/typebar/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/rcantore/typebar/compare/v0.3.2...v0.4.0
[0.3.2]: https://github.com/rcantore/typebar/compare/v0.3.1...v0.3.2
[0.3.1]: https://github.com/rcantore/typebar/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/rcantore/typebar/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/rcantore/typebar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/rcantore/typebar/releases/tag/v0.1.0
