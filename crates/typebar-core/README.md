# typebar-core

[![Crates.io](https://img.shields.io/crates/v/typebar-core.svg)](https://crates.io/crates/typebar-core)

The UI-agnostic core of [typebar](https://crates.io/crates/typebar), a WYSIWYG
Markdown editor for the terminal.

This crate holds everything that does not depend on how the screen is painted or
how keys are read: the document and its editing model, the undo/redo history,
motions and selection, Markdown analysis, search, the fuzzy matcher, file
discovery, HTML export, Unicode text geometry, and the i18n strings.

It is consumed by the TUI front end (`typebar`) and, in the future, by a GUI
front end (`typebar-gui`): both share this same core and only add the
presentation layer on top.

## Modules

- `document` — the document model and editing operations
- `blocks` / `text` — block structure and Unicode text geometry
- `markdown` — Markdown parsing and analysis
- `search` / `fuzzy` — in-document search and fuzzy matching
- `files` / `buffers` — file discovery and open buffers
- `export` — HTML export
- `i18n` — localized strings

## Status

`typebar-core` is published mainly to support the `typebar` binary. Its API is
still evolving and is not yet meant as a stable, general-purpose library. If you
just want the editor, install [`typebar`](https://crates.io/crates/typebar).

## License

Licensed under either of MIT or Apache-2.0, at your option.
