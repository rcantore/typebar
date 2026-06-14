# typebar — Documento de Arquitectura v0.1

> Editor de Markdown WYSIWYG para terminal. CLI: `tb`. Licencia dual MIT + Apache-2.0.

## Vision

typebar es un editor de Markdown WYSIWYG para terminal, con keybindings
configurables por presets (default `standard` modeless, `vim` modal opt-in),
theming configurable para ricing, y diseno modular para contribuciones open
source. Primera version: soporte exclusivo de Markdown con rendering inline
"soft WYSIWYG" (Nivel 1).

El nombre viene de la pieza mecanica de la maquina de escribir: la barra que
lleva el tipo grabado y lo imprime contra el papel a traves de la cinta. Igual
que el editor, que toma el input del teclado y lo imprime ya estilizado.

---

## Stack Tecnologico

| Componente | Tecnologia | Justificacion |
|---|---|---|
| Lenguaje | Rust (edition 2024) | Performance, type safety, ecosistema TUI maduro |
| Framework TUI | ratatui + crossterm | Estandar de facto, widget system extensible |
| Parsing Markdown | tree-sitter-markdown | Parsing incremental, AST actualizado por keystroke |
| Text buffer | ropey | Rope data structure, eficiente para edicion de texto |
| Syntax highlight | syntect | Highlighting de code blocks, configurable |
| Configuracion | TOML (via toml crate) | Estandar en herramientas TUI modernas |
| Serializacion | serde | De/serializacion de config, themes, keybindings |
| File watching | notify | Deteccion de cambios externos en archivos |

---

## Modulos Principales

### 1. Document Model (`src/document/`)

El corazon del editor. Mantiene el texto y su representacion estructural.

```
Document
- buffer: Rope              // texto crudo (ropey)
- tree: Tree                // AST Markdown (tree-sitter)
- cursor: CursorState       // posicion, selecciones
- history: UndoStack        // undo/redo lineal (stack)
- metadata: DocMeta         // path, encoding, dirty flag
```

**Responsabilidades:**
- Almacenar el texto en un Rope para edicion eficiente en documentos grandes
- Mantener el AST sincronizado via tree-sitter incremental parsing: cada edit genera un `InputEdit` que actualiza el arbol sin re-parsear todo
- Exponer una API de modificacion (`insert`, `delete`, `replace`) que actualiza buffer + arbol atomicamente
- Undo/redo lineal con stack de operaciones

**Cursor Model:**
```rust
struct CursorState {
    position: Position,           // linea + columna en el buffer
    anchor: Option<Position>,     // inicio de seleccion
    mode: CursorMode,             // Normal, Insert, Visual, Command
    preferred_column: usize,      // para movimiento vertical
}
```

En Nivel 1 el mapeo de cursor es 1:1 (no ocultamos markers), por lo que `position` siempre coincide con la posicion visual.

### 2. Renderer Engine (`src/renderer/`)

Transforma el AST de Markdown en widgets estilizados de ratatui.

**Pipeline:** Buffer (texto crudo) -> tree-sitter parse (AST) -> Style Mapper (AST node a estilo terminal) -> ratatui Spans (texto + estilos) -> Terminal draw.

**Style mapping (Nivel 1 — Soft WYSIWYG):**

| Markdown element | Rendering terminal |
|---|---|
| `**bold**` | Texto en bold, `**` dimmeados (fg gris) |
| `*italic*` | Texto en italic, `*` dimmeados |
| `# Heading 1` | Bold + color primario + `#` dimmeado |
| `## Heading 2` | Bold + color secundario + `##` dimmeado |
| inline code | Background sutil + fg mono, backticks dimmeados |
| `[link](url)` | Texto underline + color link, markers dimmeados |
| `> blockquote` | Barra vertical izquierda estilizada + color quote |
| `- list item` | Bullet reemplazado por punto o configurable |
| `---` | Linea horizontal con box-drawing chars |
| Code blocks | Background sutil + syntax highlighting (syntect) |
| Tables | Box-drawing borders + alignment |

**Principio clave:** los markers de sintaxis NUNCA desaparecen en Nivel 1. Solo se dimmean. Esto elimina el problema de cursor mapping y mantiene la edicion predecible. El usuario siempre sabe que esta editando.

### 3. Keybinding System (`src/keybinding.rs`)

Sistema configurable con presets intercambiables. Desacopla la *tecla fisica*
de la *accion semantica*: cada preset implementa el trait `Keymap` y traduce un
`KeyEvent` (segun el modo actual) a una `Action`; el loop principal aplica la
`Action` sobre el `Document` sin saber que preset esta activo.

**Presets:**

| Preset | Modal | Comportamiento | Estado |
|---|---|---|---|
| `standard` (DEFAULT) | No (modeless) | Siempre inserta texto, flechas para moverse, `Ctrl-s` guarda, `Ctrl-q` sale | Implementado |
| `vim` | Si (Normal/Insert) | Replica el Vim minimo: `hjkl`, `i`/`a`/`o`, `x`, `q`, `Esc` | Implementado |
| `wordstar` | No (modeless + chords) | Homenaje al editor clasico, basado en chords tipo `Ctrl-K S` | Proximo paso |

El default es modeless (`standard`): no hay modos Normal/Insert, lo que se
tipea se inserta y las flechas mueven. El modo en la status bar solo se muestra
cuando el preset es modal.

Seleccion via flag `--keys <nombre>` (sin config TOML por ahora). El preset
`wordstar` requiere soporte de **chords** (secuencias multi-tecla tipo
`Ctrl-K S`) que todavia no existe en el motor de teclado; queda como proximo
milestone junto con la config TOML de keybindings.

**Diseno actual (simplificado del objetivo de largo plazo):**

```rust
pub enum Action {
    CursorLeft, CursorRight, CursorUp, CursorDown,
    InsertChar(char), InsertNewline, Backspace, DeleteChar,
    EnterInsert, EnterNormal, InsertAfter, OpenLineBelow,
    Save, Quit,
}

pub trait Keymap {
    fn resolve(&self, mode: Mode, key: KeyEvent) -> Option<Action>;
    fn is_modal(&self) -> bool;
    fn initial_mode(&self) -> Mode;
    fn name(&self) -> &'static str;
}
```

**Objetivo de largo plazo (config TOML + acciones markdown):**

```rust
struct Keybinding {
    mode: Mode,              // Normal, Insert, Visual, Command
    keys: Vec<KeyEvent>,     // secuencia de teclas
    action: Action,          // accion a ejecutar
}

enum Action {
    // Movimiento
    CursorUp, CursorDown, CursorLeft, CursorRight,
    WordForward, WordBackward, LineStart, LineEnd,
    PageUp, PageDown, GoToLine(usize),
    // Edicion
    Insert(char), Delete, Backspace, NewLine, JoinLines,
    // Markdown-specific
    ToggleBold, ToggleItalic, ToggleCode, ToggleHeading,
    InsertLink, InsertTable, ToggleCheckbox,
    // Modos
    EnterInsertMode, EnterNormalMode, EnterVisualMode, EnterCommandMode,
    // Archivo
    Save, SaveAs, Open, Quit, ForceQuit,
    // UI
    ToggleZenMode, ToggleFileManager, ToggleWordCount,
    // Custom (plugins futuros)
    Custom(String),
}
```

**Config TOML (ejemplo):**
```toml
preset = "vim"

[normal]
"Ctrl-s" = "Save"
"Ctrl-b" = "ToggleBold"
"Ctrl-i" = "ToggleItalic"
"Ctrl-k" = "ToggleCode"
"Ctrl-p" = "ToggleFileManager"
"Ctrl-z" = "ToggleZenMode"

[insert]
"Ctrl-s" = "Save"
"Escape" = "EnterNormalMode"
```

### 4. Theme Engine (`src/theme/`)

Sistema de theming completo para la comunidad ricer.

```toml
# themes/catppuccin-frappe.toml
[metadata]
name = "Catppuccin Frappe"
variant = "dark"

[palette]
background = "#303446"
foreground = "#c6d0f5"
primary = "#ca9ee6"
secondary = "#99d1db"
accent = "#e78284"
dimmed = "#626880"

[markdown]
heading1 = { fg = "#ca9ee6", bold = true }
heading2 = { fg = "#99d1db", bold = true }
bold = { bold = true }
italic = { italic = true }
code_inline = { fg = "#e78284", bg = "#414559" }
link = { fg = "#8caaee", underline = true }
marker_dimmed = { fg = "#626880" }
```

**Soporte:** TOML nativo, importacion de paletas Catppuccin built-in, themes contribuidos por la comunidad (directorio `themes/`).

### 5. File Manager (`src/file_manager/`)

Panel lateral minimalista para navegacion de archivos.

**Features MVP:**
- Arbol de directorio colapsable
- Filtro por extension (`.md` por default)
- Crear / renombrar / eliminar archivos
- Indicador de archivo modificado (dirty)
- Toggle con shortcut (oculto por default en zen mode)
- Fuzzy finder integrado (tipo Telescope)

**Implementacion:** `std::fs` para filesystem, `notify` crate para watch de cambios externos, widget custom de ratatui para el arbol.

### 6. Layout Manager (`src/layout/`)

Maneja la disposicion de paneles y modos de visualizacion.

- **Normal:** file manager (opcional) a la izquierda + editor + status bar abajo.
- **Zen mode:** solo el editor centrado con ancho maximo configurable, todo lo demas oculto.
- **Split (futuro):** dos editores lado a lado, post-MVP.

### 7. i18n (`src/i18n/`)

Sistema de internacionalizacion ligero basado en archivos TOML.

**Implementacion:**
- Un archivo TOML por idioma en `locales/` (ej: `en.toml`, `es.toml`)
- Macro `t!("key")` que resuelve la string del locale activo
- Fallback a `en` si la clave no existe en el idioma seleccionado
- Idioma configurable via `config.toml` -> `[locale] language = "es"`
- ~50-80 strings totales (status bar, prompts, errores, file manager)

```toml
# locales/es.toml
save_success = "Archivo guardado"
unsaved_changes = "Hay cambios sin guardar. Salir igual? (y/n)"
word_count = "palabras"
line_count = "lineas"
search_placeholder = "Buscar archivo..."
no_results = "Sin resultados"
```

**Regla fundamental:** ninguna string de UI hardcodeada en el codigo fuente.

---

## Estructura del Proyecto

```
typebar/
- Cargo.toml
- LICENSE-MIT
- LICENSE-APACHE
- README.md
- CONTRIBUTING.md
- src/
  - main.rs               # CLI entry, clap args
  - app.rs                # App shell, event loop
  - document/             # buffer, tree, cursor, history
  - renderer/             # style_map, spans, viewport
  - keybindings/          # parser, vim preset, actions
  - theme/                # loader, default
  - i18n/                 # macro t!() + locale loader
  - file_manager/         # tree, fuzzy
  - layout/               # zen mode
  - config/               # defaults
- themes/                 # default-dark, default-light, catppuccin-frappe
- locales/                # en.toml, es.toml
- config/                 # default.toml
- docs/                   # ARCHITECTURE, KEYBINDINGS, THEMING
```

---

## Configuracion Global

```toml
# config.toml
[editor]
tab_size = 4
word_wrap = true
wrap_width = 80
line_numbers = true
auto_save = false

[markdown]
default_extension = ".md"
smart_lists = true
checkbox_toggle = true

[ui]
theme = "default-dark"
file_manager = true
zen_wrap_width = 72

[keybindings]
preset = "vim"

[locale]
language = "es"
```

---

## Plan de Implementacion — MVP

**Fase 1: Foundation (semanas 1-2)** — scaffold Cargo, event loop crossterm, document model (Rope + cursor), render de texto plano, modos Normal/Insert con Vim minimo, abrir/editar/guardar archivo.

**Fase 2: Markdown Rendering (semanas 3-4)** — integracion tree-sitter-markdown, style mapper (headings, bold, italic, code), markers dimmeados, syntax highlighting en code blocks, status bar.

**Fase 3: UX Polish (semanas 5-6)** — theme engine + 2-3 themes, zen mode, file manager, fuzzy finder, word count, keybindings configurables via TOML.

**Fase 4: Release Prep (semana 7)** — README con GIFs, CONTRIBUTING.md, CI (build + test + clippy + fmt), release binaries Linux/macOS/Windows, publicar en crates.io, post de anuncio (r/rust, r/commandline, r/unixporn).

---

## Decisiones Resueltas

| Decision | Resolucion | Notas |
|---|---|---|
| Nombre | typebar (CLI: `tb`) | Limpio en crates.io y GitHub al jun 2026 |
| Licencia | Dual MIT + Apache-2.0 | Proteccion de patentes + permisividad |
| Plugin system | Diferido post-MVP | Evaluar trait-based / WASM / Lua |
| Multi-buffer | Tabs (sin splits en v1) | Splits diferido post-MVP |
| i18n | Custom ligero (TOML) | Helper t!() de ~30 lineas |
| Undo/Redo | Lineal clasico (stack) | Sin undo tree UI |
| Keybindings | Default `standard` (flechas, modeless); `vim` (modal) y `wordstar` (homenaje) opt-in | `wordstar` requiere chords (`Ctrl-K S`), proximo paso |

---

## Evolucion Post-MVP

- **Nivel 2 WYSIWYG:** markers ocultos fuera de la linea activa
- **Split panels:** multi-buffer con splits
- **Sistema de plugins:** Lua / WASM / trait-based
- **Undo tree:** reemplazar stack lineal con visualizacion
- **LSP integration:** autocomplete de links, referencias
- **Git integration:** indicadores de diff en el gutter
- **Lenguajes adicionales:** RST, AsciiDoc, Org-mode
- **Export:** HTML, PDF desde el editor
