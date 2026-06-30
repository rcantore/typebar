# typebar

[![Crates.io](https://img.shields.io/crates/v/typebar.svg)](https://crates.io/crates/typebar)

*Read this in [Español](README.es.md).*

A WYSIWYG Markdown editor for the terminal, written in Rust. typebar renders
Markdown inline as you type (bold looks bold, headings look like headings)
while keeping editing predictable through configurable keybinding presets
(`standard`, `vim`, `wordstar`).

The name comes from the mechanical typewriter part: the bar that carries the
type and strikes it against the paper through the ribbon.

## Why typebar

I spend most of my day in the terminal, and I could never find a Markdown
editor that felt at home there. There's Obsidian, and there are plenty of
beautiful GUI editors, but I wanted something closer to
Typora: clean, live-rendered, get-out-of-your-way
WYSIWYG, except open source and running in a terminal. WordStar scratched a bit
of that itch, at least in my nostalgia, so it felt like a good direction to
build from.

And it stays deliberately simple. If you reach for something like Typora, it's
usually because you enjoy *writing*, not because you want an assistant doing it
for you. typebar has no AI, no autocomplete, no cloud. Just you, the keyboard,
and your text rendered as you type.

> **Status:** early development (`v0.2.0`). Markdown-only.

## Features

- **Soft WYSIWYG rendering** powered by [tree-sitter](https://tree-sitter.github.io/),
  in two levels:
  - **Level 1**: syntax markers are never hidden, only dimmed. Cursor-to-column
    mapping stays 1:1 on every line, so editing is always predictable.
  - **Level 2** *(default)*: inline delimiters are collapsed on **inactive**
    lines: `**bold**` → **bold**, `# Heading` drops the `#`, `- item` → `• item`,
    `> quote` → `│ quote`, and `[text](url)` shows just the text. The line under
    the cursor always renders as Level 1, so the cursor mapping never shifts.
    During an active selection or search, the whole view falls back to Level 1
    so highlights land on real cells.
- **Three keybinding presets**, swappable at launch or via config:
  - `standard`: modeless, arrow-key navigation (default).
  - `vim`: modal (Normal / Insert / Visual).
  - `wordstar`: modeless with classic chords (`Ctrl-K S`, `Ctrl-Q S`, …).
  - Plus per-key **custom overrides** layered on top of any preset.
- **Editing essentials**: undo/redo, visual selection, system clipboard
  copy/paste/cut, find & replace, bold/italic/code toggles, and full motions
  (line/doc start & end, Page Up/Down, Home/End).
- **Multiple files with a fuzzy switcher and tabs**: `Ctrl-E` opens a fuzzy finder
  over the project files and open buffers (type to filter, Enter to open/switch);
  `Ctrl-N` starts a new file; open buffers show as tabs you can cycle with
  `Ctrl-PageDown`/`Ctrl-PageUp` (or click, with `[ui] mouse = true`).
- **Command palette**: `Ctrl-A` fuzzy-finds any command by name (with its current
  shortcut shown) and runs it — also a handy way to learn the keybindings.
- **Live word count** in the status bar (Unicode word boundaries), with a
  selected-words count while text is selected.
- **Zen / focus mode**: hide all chrome (border, toolbar, status bar) to write
  distraction-free. Toggle from the *view* submenu — `Ctrl-O Z` (standard /
  wordstar, echoing WordStar's onscreen-format prefix) or `z z` (vim); in the
  modeless presets `Esc` also exits.
- **Whitepaper mode**: a typewriter-like *sheet of paper* — zen plus a monochrome
  ink-on-paper theme (hierarchy comes from weight, not color) and a centered
  fixed-width column. Toggle from the *view* submenu — `Ctrl-O W` (standard /
  wordstar) or `z w` (vim); `Esc` exits in the modeless presets.
- **Unicode-aware**: grapheme-cluster cursor movement and correct display width
  for CJK, emoji, and combining characters.
- **HTML export**: `typebar notes.md --export-html` writes a standalone
  `notes.html` (CommonMark via pulldown-cmark) without opening the editor — or
  export the current buffer from inside the editor with the *Export HTML* command
  (command palette), which shows the result in the status bar.
- **Themeable** for ricing: `frappe` (default), `mocha`, and the light `latte`
  Catppuccin palettes, with a runtime light/dark toggle (`Ctrl-O L`, or `z l` in
  vim).
- **Internationalized UI**: English by default, Spanish auto-detected from
  `$LANG`, both overridable in config.

## Install & run

### Install a prebuilt binary (recommended)

One command, no Gatekeeper/SmartScreen prompt (installers downloaded with
`curl`/PowerShell are not quarantined). The binary lands in `~/.cargo/bin`
(or `~/.local/bin`) and is added to your `PATH`.

**macOS / Linux:**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rcantore/typebar/releases/latest/download/typebar-installer.sh | sh
```

**Windows (PowerShell):**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rcantore/typebar/releases/latest/download/typebar-installer.ps1 | iex"
```

Then run `typebar notes.md`.

#### Or download the archive manually

From the [latest release](https://github.com/rcantore/typebar/releases/latest):

- **macOS** (Apple Silicon): `typebar-aarch64-apple-darwin.tar.xz`
- **Linux** (x86_64): `typebar-x86_64-unknown-linux-gnu.tar.xz`
- **Windows** (x86_64): `typebar-x86_64-pc-windows-msvc.zip`

Each archive bundles the `typebar` binary with the README and licenses, plus a
matching `.sha256` checksum:

```bash
tar xf typebar-aarch64-apple-darwin.tar.xz
./typebar-aarch64-apple-darwin/typebar notes.md
```

Because the browser flags manual downloads, the **unsigned** binary will warn on
first launch (the installers above avoid this):

- **macOS**: right-click the binary and choose *Open* the first time (or clear
  the quarantine flag with `xattr -d com.apple.quarantine ./typebar`);
  afterwards it runs normally.
- **Windows**: if SmartScreen appears, choose *More info -> Run anyway*.

### Install with cargo

If you already have a Rust toolchain, install the latest release from
crates.io. It compiles locally, so there's no Gatekeeper/SmartScreen prompt:

```bash
cargo install typebar
```

Then run `typebar notes.md`.

### Build from source

Requires **Rust 1.85+** (edition 2024).

```bash
git clone https://github.com/rcantore/typebar.git
cd typebar
cargo run --release -- notes.md
```

Or build a binary:

```bash
cargo build --release
./target/release/typebar notes.md
```

### Command-line usage

```
typebar [PATH] [--keys <preset>]
```

- `PATH`: file to open (defaults to `scratch.md` if omitted).
- `--keys <preset>`: keybinding preset (`standard`, `vim`, or `wordstar`).
  Takes precedence over the config file.
- `--export-html`: convert `PATH` to a standalone `.html` file and exit, without
  opening the editor.
- `-h`, `--help`: print usage and exit.

```bash
typebar README.md --keys vim
typebar              # opens scratch.md with the standard preset
typebar notes.md --export-html   # writes notes.html and exits
typebar --help                   # prints usage and exits
```

## Configuration

typebar reads an optional TOML file at `~/.config/typebar/config.toml`
(honoring `XDG_CONFIG_HOME`). Everything is optional; without the file, the
built-in defaults apply. A starting point lives in
[`examples/config.toml`](examples/config.toml).

```toml
[keybindings]
# "standard" (default) | "vim" | "wordstar".
# The --keys CLI flag overrides this.
preset = "standard"

# Per-key overrides, layered on top of the preset. `mode` is optional
# ("normal" | "insert" | "visual"); when omitted, the binding applies in any mode.
[[keybindings.bind]]
keys = "ctrl-s"
action = "save"

[ui]
# "frappe" (default) | "mocha" | "latte" (light). Unknown names fall back to frappe.
theme = "frappe"

# UI language: "en" | "es". Defaults to English, or to your $LANG if Spanish.
locale = "en"

# WYSIWYG level: 1 (markers always visible) or 2 (hidden off the active line).
# Defaults to 2; invalid values fall back to 2.
wysiwyg_level = 2

# Mouse capture: enables clicking the buffer tabs. Off by default (keeps the
# terminal's native text selection).
mouse = false
```

**Preset resolution precedence:** `--keys` flag → config `preset` → built-in
default (`standard`).

Bindable actions include `cursor-{left,right,up,down}`, `line-{start,end}`,
`doc-{start,end}`, `page-{up,down}`, `enter-{insert,normal,visual}`,
`insert-after`, `open-line-below`, `select-{left,right,up,down}`,
`delete-selection`, `delete-char`, `backspace`, `insert-newline`,
`toggle-{bold,italic,code}`, `undo`, `redo`, `yank`, `paste`, `search`,
`replace`, `save`, `save-and-quit`, and `quit`.

## Development

```bash
cargo build              # build
cargo test               # run the test suite
cargo fmt --check        # formatting (enforced in CI)
cargo clippy --all-targets -- -D warnings   # lints (enforced in CI)
```

The architecture and rendering pipeline are documented in
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Contributing

Contributions are welcome! Please read [`CONTRIBUTING.md`](CONTRIBUTING.md)
for how to build, test, and submit changes, and note the
[Code of Conduct](CODE_OF_CONDUCT.md). Notable changes are tracked in the
[changelog](CHANGELOG.md).

## License

Licensed under either of [MIT](LICENSE-MIT) or
[Apache-2.0](LICENSE-APACHE) at your option.
