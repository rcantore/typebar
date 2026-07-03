# Contributing to typebar

*Read this in [Español](CONTRIBUTING.es.md).*

Thanks for your interest in typebar! This is a small, early-stage project, so
contributions of any size (bug reports, fixes, docs, ideas) are welcome.

## Code of conduct

By participating you agree to keep interactions respectful and constructive.
Be kind, assume good faith.

## Getting started

typebar is written in Rust and uses [edition 2024], so you need a recent stable
toolchain (Rust **1.85+**). Install it via [rustup](https://rustup.rs/).

```bash
git clone https://github.com/rcantore/typebar
cd typebar
cargo build
cargo run -- crates/typebar-tui/examples/sample.md      # try it out
```

typebar is a Cargo workspace with two crates: `typebar-core` (the UI-agnostic
document model, buffers, search, markdown, export, i18n) and `typebar-tui`
(the terminal interface, packaged as `typebar`). For a high-level tour of how
the editor is structured, read
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Before you open a pull request

Run the same checks CI runs, all four must pass:

```bash
cargo fmt --check                          # formatting
cargo clippy --all-targets -- -D warnings  # lints (warnings are errors)
cargo build --verbose                      # it compiles
cargo test --verbose                       # tests are green
```

A quick `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test`
before pushing will save you a CI round-trip.

### Checklist

- [ ] `cargo fmt --check` is clean.
- [ ] `cargo clippy --all-targets -- -D warnings` passes.
- [ ] `cargo test` passes.
- [ ] New behavior is covered by tests.
- [ ] **If your change affects behavior, add a line to the `[Unreleased]`
      section of [`CHANGELOG.md`](CHANGELOG.md).**
- [ ] User-facing changes are reflected in both `README.md` and `README.es.md`.

## Commit messages

Use short, prefixed, imperative messages, the style already used in the repo:

```
feat: add soft-wrap toggle
fix: correct cursor position after blockquote
docs: clarify keybinding precedence
ci: build on macOS and Windows
style: apply cargo fmt
```

Common prefixes: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `ci`,
`chore`, `i18n`.

## Reporting bugs

Open an issue with:

- What you did (ideally a minimal Markdown snippet or keystrokes).
- What you expected vs. what happened.
- Your OS, terminal emulator, and `typebar --version` (or commit hash).
- The keybinding preset in use (`standard`, `vim`, or `wordstar`).

## Proposing features

For anything non-trivial, open an issue to discuss before writing code; it
saves everyone time if the approach can be agreed on first. Small, focused PRs
are much easier to review than large ones.

## License

By contributing, you agree that your contributions are licensed under the same
terms as the project: **MIT OR Apache-2.0** (see
[`LICENSE-MIT`](LICENSE-MIT) and [`LICENSE-APACHE`](LICENSE-APACHE)).

[edition 2024]: https://doc.rust-lang.org/edition-guide/rust-2024/index.html
