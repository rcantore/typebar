# typebar

Un editor de Markdown **WYSIWYG** para la terminal, escrito en *Rust*.

## Spike técnico

Validamos el pipeline `tree-sitter` → `ratatui`. Esto es un párrafo
normal con una palabra en **negrita**, otra en *itálica*, y código
inline como `let x = 42;`.

### Detalles

El render mantiene los marcadores visibles (`**`, `*`, backticks)
pero **dimmeados**, siguiendo el Nivel 1 de la arquitectura.
