# Contribuir a typebar

*Read this in [English](CONTRIBUTING.md).*

¡Gracias por tu interés en typebar! Es un proyecto chico y en etapa temprana,
así que toda contribución es bienvenida: reportes de bugs, fixes, docs, ideas.

## Código de conducta

Al participar te comprometés a mantener un trato respetuoso y constructivo.
Sé amable, asumí buena fe.

## Primeros pasos

typebar está escrito en Rust y usa [edition 2024], así que necesitás un
toolchain estable reciente (Rust **1.85+**). Instalalo con
[rustup](https://rustup.rs/).

```bash
git clone https://github.com/rcantore/typebar
cd typebar
cargo build
cargo run -- examples/sample.md      # probalo
```

Para entender cómo está estructurado el editor, leé
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Antes de abrir un pull request

Corré los mismos checks que corre CI, los cuatro tienen que pasar:

```bash
cargo fmt --check                          # formato
cargo clippy --all-targets -- -D warnings  # lints (los warnings son errores)
cargo build --verbose                      # compila
cargo test --verbose                       # tests en verde
```

Un `cargo fmt && cargo clippy --all-targets -- -D warnings && cargo test` antes
de pushear te ahorra una vuelta de CI.

### Checklist

- [ ] `cargo fmt --check` está limpio.
- [ ] `cargo clippy --all-targets -- -D warnings` pasa.
- [ ] `cargo test` pasa.
- [ ] El comportamiento nuevo está cubierto por tests.
- [ ] **Si tu cambio afecta el comportamiento, agregá una línea a la sección
      `[Unreleased]` de [`CHANGELOG.md`](CHANGELOG.md).**
- [ ] Los cambios visibles para el usuario se reflejan en `README.md` y
      `README.es.md`.

## Mensajes de commit

Usá mensajes cortos, con prefijo, en imperativo, el estilo que ya usa el repo:

```
feat: add soft-wrap toggle
fix: correct cursor position after blockquote
docs: clarify keybinding precedence
ci: build on macOS and Windows
style: apply cargo fmt
```

Prefijos comunes: `feat`, `fix`, `docs`, `style`, `refactor`, `test`, `ci`,
`chore`, `i18n`.

## Reportar bugs

Abrí un issue con:

- Qué hiciste (idealmente un fragmento mínimo de Markdown o las teclas).
- Qué esperabas vs. qué pasó.
- Tu SO, emulador de terminal y `typebar --version` (o el hash del commit).
- El preset de keybindings en uso (`standard`, `vim` o `wordstar`).

## Proponer features

Para algo no trivial, abrí un issue para discutir antes de escribir código;
ahorra tiempo si se acuerda el enfoque primero. Los PRs chicos y enfocados son
mucho más fáciles de revisar que los grandes.

## Licencia

Al contribuir, aceptás que tus contribuciones se licencian bajo los mismos
términos que el proyecto: **MIT OR Apache-2.0** (ver
[`LICENSE-MIT`](LICENSE-MIT) y [`LICENSE-APACHE`](LICENSE-APACHE)).

[edition 2024]: https://doc.rust-lang.org/edition-guide/rust-2024/index.html
