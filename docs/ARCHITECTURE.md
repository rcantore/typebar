# typebar: Documento de Arquitectura v0.1

> Editor de Markdown WYSIWYG para terminal. CLI: `tb`. Licencia dual MIT + Apache-2.0.

## Vision

typebar es un editor de Markdown WYSIWYG para terminal, con keybindings
configurables por presets (default `standard` modeless, `vim` modal opt-in),
theming configurable para ricing, y diseno modular para contribuciones open
source. Primera version: soporte exclusivo de Markdown con rendering inline
"soft WYSIWYG" (Nivel 1) y un Nivel 2 que oculta los delimitadores inline
fuera de la linea activa.

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
- Undo/redo lineal con stack de snapshots (implementado, ver mas abajo)

**Cursor Model:**
```rust
struct CursorState {
    position: Position,           // linea + columna en el buffer
    anchor: Option<Position>,     // inicio de seleccion
    mode: CursorMode,             // Normal, Insert, Visual, Command
    preferred_column: usize,      // para movimiento vertical
}
```

En Nivel 1 el mapeo de cursor es 1:1 (no ocultamos markers), por lo que `position` siempre coincide con la posicion visual. En Nivel 2 el mapeo sigue siendo 1:1 sobre la linea activa, porque esa linea se renderiza como Nivel 1; los markers solo se contraen en las lineas inactivas, donde el cursor no esta.

### 2. Renderer Engine (`src/renderer/`)

Transforma el AST de Markdown en widgets estilizados de ratatui.

**Pipeline:** Buffer (texto crudo) -> tree-sitter parse (AST) -> Style Mapper (AST node a estilo terminal) -> ratatui Spans (texto + estilos) -> Terminal draw.

**Style mapping (Nivel 1, Soft WYSIWYG):**

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

**Nivel 2 (soft WYSIWYG con linea activa visible):** transformaciones aplicadas SOLO en las lineas que NO tienen el cursor (la linea activa sigue renderizandose como Nivel 1, asi que el mapeo cursor->columna no cambia):

| Elemento | Linea activa (Nivel 1) | Linea inactiva (Nivel 2) |
|---|---|---|
| `**bold**` | `**bold**` | `bold` |
| `*italic*` | `*italic*` | `italic` |
| `` `code` `` | `` `code` `` | `code` |
| `# Heading`, `## H2`, ... | `# Heading` | `Heading` |
| `- item`, `* item`, `+ item` | `- item` | `• item` |
| `1. item` | `1. item` | `1. item` (sin cambios) |
| `> cita` | `> cita` | `│ cita` |
| `[texto](url)` | `[texto](url)` | `texto` |
| `![alt](src)` | `![alt](src)` | `alt` |

Si hay seleccion no vacia o coincidencias de busqueda activas, se vuelve a Nivel 1 global mientras dure el estado, para que los highlights caigan sobre celdas reales y no sobre bytes ocultos. Se configura con `[ui].wysiwyg_level = 1 | 2` en el TOML (default `2`).

### 3. Keybinding System (`src/keybinding.rs`)

Sistema configurable con presets intercambiables. Desacopla la *tecla fisica*
de la *accion semantica*: cada preset implementa el trait `Keymap` y traduce una
*secuencia* de `KeyEvent` (segun el modo actual) a una `Action`; el loop
principal aplica la `Action` sobre el `Document` sin saber que preset esta
activo.

**Presets:**

| Preset | Modal | Comportamiento | Estado |
|---|---|---|---|
| `standard` (DEFAULT) | No (modeless) | Siempre inserta texto, flechas para moverse, `Ctrl-s` guarda, `Ctrl-q` sale, `Ctrl-b` negrita, chord `Ctrl-P` formato | Implementado |
| `vim` | Si (Normal/Insert) | Replica el Vim minimo: `hjkl`, `i`/`a`/`o`, `x`, `q`, `Esc`, chord `Ctrl-P` formato (en cualquier modo) | Implementado |
| `wordstar` | No (modeless + chords) | Homenaje al editor clasico: diamante `Ctrl-E/X/S/D` + chords `Ctrl-K`/`Ctrl-Q`/`Ctrl-P` | Implementado |

El default es modeless (`standard`): no hay modos Normal/Insert, lo que se
tipea se inserta y las flechas mueven. El modo en la status bar solo se muestra
cuando el preset es modal.

Seleccion via flag `--keys <nombre>` (sin config TOML por ahora).

**Soporte de chords (secuencias multi-tecla):**

El trait `resolve` recibe el *buffer* de teclas pendientes y devuelve un
`Resolve`: `Action(a)` (la secuencia completa una accion), `Pending` (la
secuencia es prefijo de un chord, hay que esperar mas teclas) o `None` (no
bindeada). El loop principal mantiene un `Vec<KeyEvent>` de teclas pendientes:
en cada `Press` agrega la tecla y resuelve; ante `Action` aplica y limpia, ante
`Pending` espera, ante `None` limpia (lo que cancela gratis un chord invalido o
un `Esc` tras un prefijo). La status bar muestra un indicador `^K`/`^Q`/`^P`
cuando hay un chord en curso (el indicador es generico para cualquier prefijo
con Ctrl). No hay timeout: el prefijo queda pendiente hasta la proxima tecla.

**Chord de formato `Ctrl-P` (uniforme en los tres presets):**

El prefijo `Ctrl-P` seguido de una letra togglea un estilo inline sobre la
palabra bajo el cursor: `Ctrl-P B` negrita, `Ctrl-P I` italica, `Ctrl-P C`
codigo (segunda tecla case-insensitive). En `standard` ademas `Ctrl-B` togglea
negrita directo (memoria muscular); NO se bindea `Ctrl-I` para italica porque en
la terminal es indistinguible de `Tab`, por eso la via canonica es el prefijo
`Ctrl-P`. En `vim` el chord funciona en CUALQUIER modo (el formato es agnostico
al modo). El helper `resolve_format_second` se comparte entre los tres presets
para mantener el mapeo unico.

**Preset `wordstar` (homenaje):**

- **Diamante de navegacion** (Ctrl): `Ctrl-E` arriba, `Ctrl-X` abajo, `Ctrl-S`
  IZQUIERDA, `Ctrl-D` derecha. (Si: en WordStar `Ctrl-S` es izquierda, no
  guardar; guardar es un chord. Se respeta esa autenticidad.)
- **Chord `Ctrl-K`** (bloque/archivo): `Ctrl-K S` guarda, `Ctrl-K D` y
  `Ctrl-K X` guardan y salen, `Ctrl-K Q` sale, `Ctrl-K C` copia bloque (yank),
  `Ctrl-K V` pega/mueve bloque (paste).
- **Chord `Ctrl-Q`** (movimiento rapido): `Ctrl-Q S` inicio de linea, `Ctrl-Q D`
  fin de linea, `Ctrl-Q R` inicio del documento, `Ctrl-Q C` fin del documento.
- La segunda tecla del chord se acepta case-insensitive. Las flechas y los
  chars sin Ctrl funcionan normal (insercion modeless).

**Diseno actual (simplificado del objetivo de largo plazo):**

```rust
pub enum Action {
    CursorLeft, CursorRight, CursorUp, CursorDown,
    InsertChar(char), InsertNewline, Backspace, DeleteChar,
    EnterInsert, EnterNormal, InsertAfter, OpenLineBelow,
    LineStart, LineEnd, DocStart, DocEnd,
    Save, SaveAndQuit, Quit,
    ToggleBold, ToggleItalic, ToggleCode,
    EnterVisual, SelectLeft, SelectRight, SelectUp, SelectDown, DeleteSelection,
    Undo, Redo,
    Yank, Paste,
}

pub enum Resolve {
    Action(Action),  // la secuencia completa una accion
    Pending,         // prefijo de un chord: esperar mas teclas
    None,            // no bindeada
}

pub trait Keymap {
    fn resolve(&self, mode: Mode, keys: &[KeyEvent]) -> Resolve;
    fn is_modal(&self) -> bool;
    fn initial_mode(&self) -> Mode;
    fn name(&self) -> &'static str;
}
```

**Edicion de estilos (AST-based, implementado):**

El chord `Ctrl-P B/I/C` togglea negrita/italica/codigo sobre la palabra bajo el
cursor. La deteccion del estilo existente NO es textual sino **AST-based**: el
modulo `src/markdown.rs` parsea el documento con tree-sitter-md y la funcion
`enclosing(text, byte_offset, kind)` busca el nodo inline mas interno
(`strong_emphasis`/`emphasis`/`code_span`) que contiene el offset del cursor,
devolviendo los rangos en bytes de sus marcadores de apertura y cierre. Con eso
`Document::toggle_inline` decide:

- **Cursor dentro de un enfasis del tipo** (`enclosing` devuelve `Some`):
  destogglea quitando ambos marcadores (borra primero el cierre para no
  invalidar indices) y reubica el cursor sobre el mismo char de contenido.
- **Sin enfasis, con palabra bajo el cursor**: la envuelve con el marcador,
  dejando el cursor sobre el mismo char.
- **Sin palabra** (cursor en espacio/vacio): inserta el par de marcadores vacio
  y deja el cursor entre ambos para tipear adentro.

`markdown.rs` aisla todo el uso de tree-sitter para consultas semanticas, para
que `document.rs` no dependa de la gramatica directamente.

#### Modelo de seleccion (`src/document/select.rs`)

La seleccion se guarda en `Document` como un *ancla* opcional
(`selection_anchor: Option<usize>`), un char-index ABSOLUTO en el buffer. El
rango seleccionado va del min al max entre el ancla y el cursor (character-wise,
ordenado); `None` o un rango vacio = sin seleccion. `selection_range()` devuelve
el rango en chars y `selection_byte_range()` el mismo en bytes (para el render),
sin exponer el buffer.

- **Movimiento**: cada movimiento tiene un nucleo `*_core` que solo mueve el
  cursor. Los `move_*` publicos colapsan la seleccion antes de moverse; los
  `extend_*` fijan el ancla antes de moverse. Asi la logica grapheme-based del
  movimiento no se duplica.
- **Vim**: la tecla `v` entra a `Mode::Visual`; en Visual `h/j/k/l`/flechas
  extienden, `x`/`d` borran la seleccion, `Esc` vuelve a Normal. El chord de
  formato `Ctrl-P B/I/C` opera sobre la seleccion y consume el modo (vuelve a
  Normal).
- **Presets modeless** (standard/wordstar): `Shift`+flechas extienden la
  seleccion (las flechas sin shift colapsan). No usan `Mode::Visual`: la
  seleccion vive solo en el ancla, y la status bar muestra `SEL` si esta activa.
- **Operaciones**: `toggle_inline` con seleccion envuelve EL RANGO con el
  marcador (sin detectar enfasis existente: siempre envuelve, nunca destogglea,
  limitacion conocida). `delete_selection` borra el rango y reubica el cursor en
  el inicio; `backspace`/`delete_char` borran la seleccion si hay una activa.
- **Resaltado**: `render(source, selection)` recibe el rango en BYTES y, tras
  armar el mapa de estilo por byte, pisa el `bg` de los bytes seleccionados
  (`SELECTION_BG`) preservando fg/modifiers del texto.

#### Undo/Redo (`src/document/history.rs`, implementado)

Undo/redo lineal por **snapshots**, no por diffs. Cada snapshot guarda una copia
del estado restaurable: `{ buffer: Rope, line, col, selection_anchor }`. Clonar
el `Rope` es barato porque ropey es una estructura persistente con sharing de
nodos, asi que no hace falta modelar operaciones inversas.

- **Pilas**: `Document` tiene `undo_stack` (estados previos) y `redo_stack`
  (estados deshechos), privadas. Un helper `snapshot()` empuja el estado ACTUAL
  al `undo_stack` ANTES de mutar y vacia el `redo_stack` (cualquier edicion nueva
  descarta lo deshecho). El `undo_stack` se capa a 500 entradas descartando la
  mas vieja, para acotar memoria.
- **Coalescing del tipeo**: para que `undo` no sea letra por letra, los
  `insert_char` consecutivos comparten un solo snapshot. El flag
  `last_was_insert` arranca una corrida en el primer insert y solo ahi
  snapshotea; los inserts siguientes no. Todas las demas mutaciones
  (`insert_newline`, `backspace`, `delete_char`, `open_line_below`,
  `toggle_inline`, `delete_selection`) snapshotean siempre y apagan el flag. Los
  **movimientos** (move_*, move_to_*, extend_*) y el propio `undo`/`redo` apagan
  el flag sin snapshotear: asi un movimiento corta la corrida y el proximo insert
  arranca un grupo de undo nuevo.
- **`undo`/`redo`**: `undo` empuja el estado actual a `redo_stack`, saca el tope
  de `undo_stack` y lo restaura (buffer/cursor/seleccion + `sync_preferred` +
  `dirty`). `redo` es simetrico. Sin historia, no hacen nada.
- **Bindings por preset**: vim `u` deshace y `Ctrl-R` rehace (canonico, solo en
  Normal); standard y wordstar usan `Ctrl-Z` deshacer y `Ctrl-Y` rehacer
  (convencion moderna; el raw mode captura `Ctrl-Z` asi que no suspende).

#### Clipboard interno (`src/document/clipboard.rs`, implementado)

Portapapeles INTERNO del editor (no el del SO), guardado en `Document` como
`clipboard: Option<String>` (privado, no persiste entre sesiones).

- **`yank`**: si hay seleccion, copia ese rango del buffer al `clipboard` y
  limpia la seleccion. Sin seleccion no hace nada. NO es una mutacion del buffer:
  no toma snapshot ni toca `dirty`.
- **`paste`**: si el `clipboard` tiene texto, lo inserta en la posicion del
  cursor y deja el cursor al final del texto pegado (avanza por CHARS, no bytes).
  Es una MUTACION: toma snapshot al tope (asi se integra con undo/redo y es
  undoable) y marca `dirty`. Con el clipboard vacio no hace nada.
- **Seleccion activa al pegar**: por simplicidad NO se reemplaza; `paste` solo
  inserta en el cursor (queda fuera del scope reemplazar la seleccion).
- **Bindings por preset**: vim `y` (en Visual) copia y `p` (en Normal) pega;
  standard `Ctrl-C` copia y `Ctrl-V` pega (el raw mode captura `Ctrl-C` asi que
  no interrumpe); wordstar `Ctrl-K C` copia bloque y `Ctrl-K V` pega/mueve
  bloque (autentico de WordStar). En Vim, `yank` consume el modo Visual (vuelve a
  Normal).

**Proximos pasos** (fuera del scope actual):

- **Clipboard del SISTEMA**: hoy el portapapeles es solo interno; el siguiente
  paso es integrarlo con el del SO via un crate tipo `arboard`/`copypasta`, y
  exponer la eleccion (interno vs sistema) por config TOML.
- Seleccion por palabra/linea (`vw`, `V` de Vim): hoy es solo character-wise.
- Movimiento por palabra (`Ctrl-A`/`Ctrl-F` de WordStar, requiere acciones de
  word-motion).
- Config TOML de keybindings.

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

## Plan de Implementacion: MVP

**Fase 1: Foundation (semanas 1-2).** Scaffold Cargo, event loop crossterm, document model (Rope + cursor), render de texto plano, modos Normal/Insert con Vim minimo, abrir/editar/guardar archivo.

**Fase 2: Markdown Rendering (semanas 3-4).** Integracion tree-sitter-markdown, style mapper (headings, bold, italic, code), markers dimmeados, syntax highlighting en code blocks, status bar.

**Fase 3: UX Polish (semanas 5-6).** Theme engine + 2-3 themes, zen mode, file manager, fuzzy finder, word count, keybindings configurables via TOML.

**Fase 4: Release Prep (semana 7).** README con GIFs, CONTRIBUTING.md, CI (build + test + clippy + fmt), release binaries Linux/macOS/Windows, publicar en crates.io, post de anuncio (r/rust, r/commandline, r/unixporn).

---

## Decisiones Resueltas

| Decision | Resolucion | Notas |
|---|---|---|
| Nombre | typebar (CLI: `tb`) | Limpio en crates.io y GitHub al jun 2026 |
| Licencia | Dual MIT + Apache-2.0 | Proteccion de patentes + permisividad |
| Plugin system | Diferido post-MVP | Evaluar trait-based / WASM / Lua |
| Multi-buffer | Tabs (sin splits en v1) | Splits diferido post-MVP |
| i18n | Custom ligero (TOML) | Helper t!() de ~30 lineas |
| Undo/Redo | Lineal clasico (stack de snapshots, implementado) | Coalescing del tipeo; aprovecha el clone barato de ropey. Sin undo tree UI |
| Keybindings | Default `standard` (flechas, modeless); `vim` (modal) y `wordstar` (homenaje, con chords) opt-in | Chords implementados via `Resolve` + buffer de teclas pendientes |

---

## Evolucion Post-MVP

- **Nivel 2 WYSIWYG:** implementado completo (inline `**`/`*`/`` ` ``, headings, listas no ordenadas, blockquote, links, imagenes).
- **Split panels:** multi-buffer con splits
- **Sistema de plugins:** Lua / WASM / trait-based
- **Undo tree:** reemplazar stack lineal con visualizacion
- **LSP integration:** autocomplete de links, referencias
- **Git integration:** indicadores de diff en el gutter
- **Lenguajes adicionales:** RST, AsciiDoc, Org-mode
- **Export:** HTML, PDF desde el editor
