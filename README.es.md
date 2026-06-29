# typebar

*Read this in [English](README.md).*

Un editor de Markdown WYSIWYG para la terminal, escrito en Rust. typebar
renderiza el Markdown en vivo mientras escribís (la negrita se ve en negrita,
los títulos parecen títulos) manteniendo la edición predecible a través de
presets de teclado configurables (`standard`, `vim`, `wordstar`).

El nombre viene de la pieza mecánica de la máquina de escribir: la barra que
lleva el tipo grabado y lo imprime contra el papel a través de la cinta.

## Por qué typebar

Paso casi todo el día en la terminal y nunca encontré un editor de Markdown que
viviera ahí cómodo. Existe Obsidian, y hay un montón de editores gráficos
lindos, pero yo quería algo más parecido a Typora:
limpio, renderizado en vivo, un WYSIWYG que no te estorba, pero open source y
corriendo en una terminal. WordStar cumplía un poco con eso, al menos en mi
nostalgia, así que me pareció un buen camino para arrancar.

Y es deliberadamente simple. Si usás algo como Typora, normalmente es porque
disfrutás *escribir*, no porque quieras un asistente que lo haga por vos.
typebar no tiene IA, ni autocompletado, ni nube. Sólo vos, el teclado, y tu
texto renderizado mientras escribís.

> **Estado:** desarrollo temprano (`v0.2.0`). Sólo Markdown.

## Características

- **Renderizado soft WYSIWYG** con [tree-sitter](https://tree-sitter.github.io/),
  en dos niveles:
  - **Nivel 1**: los marcadores de sintaxis nunca se ocultan, sólo se atenúan.
    El mapeo cursor→columna queda 1:1 en todas las líneas, así la edición es
    siempre predecible.
  - **Nivel 2** *(default)*: los delimitadores inline se contraen en las
    líneas **inactivas**: `**negrita**` → **negrita**, `# Título` pierde el `#`,
    `- item` → `• item`, `> cita` → `│ cita`, y `[texto](url)` muestra sólo el
    texto. La línea bajo el cursor siempre se renderiza como Nivel 1, así que el
    mapeo del cursor no se corre. Con una selección o búsqueda activa, toda la
    vista vuelve a Nivel 1 para que los resaltados caigan sobre celdas reales.
- **Tres presets de teclado**, intercambiables al iniciar o por config:
  - `standard`: modeless, navegación con flechas (default).
  - `vim`: modal (Normal / Insert / Visual).
  - `wordstar`: modeless con chords clásicos (`Ctrl-K S`, `Ctrl-Q S`, …).
  - Además, **overrides por tecla** que se aplican encima de cualquier preset.
- **Edición esencial**: undo/redo, selección visual, copiar/pegar/cortar contra
  el portapapeles del sistema, buscar y reemplazar, toggles de negrita/itálica/
  código, y movimientos completos (inicio/fin de línea y de documento, Page
  Up/Down, Home/End).
- **Multi-archivo con switcher fuzzy y tabs**: `Ctrl-G` abre un fuzzy finder sobre
  los archivos del proyecto y los buffers abiertos (tipeás para filtrar, Enter
  para abrir/cambiar); `Ctrl-N` arranca un archivo nuevo; los buffers abiertos se
  ven como tabs que cambiás con `Ctrl-PageDown`/`Ctrl-PageUp` (o con click, si
  ponés `[ui] mouse = true`).
- **Paleta de comandos**: `Ctrl-A` fuzzy-filtra cualquier comando por nombre
  (mostrando su atajo actual) y lo ejecuta — también sirve para aprender los
  keybindings.
- **Contador de palabras en vivo** en la status bar (límites de palabra Unicode),
  con conteo de las palabras seleccionadas mientras hay selección.
- **Modo zen / focus**: oculta todo el chrome (borde, toolbar, status bar) para
  escribir sin distracciones. Se togglea desde el submenú *view* — `Ctrl-O Z`
  (standard / wordstar, homenaje al prefijo de onscreen-format de WordStar) o
  `z z` (vim); en los presets modeless `Esc` también sale.
- **Modo whitepaper**: una *hoja de papel* estilo máquina de escribir — zen más
  un theme monocromo de tinta sobre papel (la jerarquía la da el peso, no el
  color) y una columna de ancho fijo centrada. Se togglea desde el submenú
  *view* — `Ctrl-O W` (standard / wordstar) o `z w` (vim); en los presets
  modeless `Esc` también sale.
- **Consciente de Unicode**: movimiento del cursor por clusters de grafemas y
  ancho de display correcto para CJK, emoji y caracteres combinantes.
- **Export a HTML**: `typebar notas.md --export-html` escribe un `notas.html`
  standalone (CommonMark vía pulldown-cmark) sin abrir el editor — o exportá el
  buffer actual desde adentro del editor con el comando *Export HTML* (paleta de
  comandos), que muestra el resultado en la status bar.
- **Themeable** para ricing: paletas Catppuccin `frappe` (default), `mocha` y la
  clara `latte`, con un toggle claro/oscuro en runtime (`Ctrl-O L`, o `z l` en
  vim).
- **UI internacionalizada**: inglés por defecto, español autodetectado desde
  `$LANG`, ambos configurables.

## Instalación y ejecución

### Instalar un binario precompilado (recomendado)

Un comando, sin prompt de Gatekeeper/SmartScreen (los instaladores bajados con
`curl`/PowerShell no quedan en cuarentena). El binario va a `~/.cargo/bin`
(o `~/.local/bin`) y se agrega a tu `PATH`.

**macOS / Linux:**

```bash
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/rcantore/typebar/releases/latest/download/typebar-installer.sh | sh
```

**Windows (PowerShell):**

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://github.com/rcantore/typebar/releases/latest/download/typebar-installer.ps1 | iex"
```

Después corré `typebar notas.md`.

#### O descargar el archivo a mano

Desde el [último release](https://github.com/rcantore/typebar/releases/latest):

- **macOS** (Apple Silicon): `typebar-aarch64-apple-darwin.tar.xz`
- **Linux** (x86_64): `typebar-x86_64-unknown-linux-gnu.tar.xz`
- **Windows** (x86_64): `typebar-x86_64-pc-windows-msvc.zip`

Cada archivo trae el binario `typebar` junto al README y las licencias, más un
checksum `.sha256`:

```bash
tar xf typebar-aarch64-apple-darwin.tar.xz
./typebar-aarch64-apple-darwin/typebar notas.md
```

Como el navegador marca las descargas manuales, el binario **sin firmar** avisa
en el primer arranque (los instaladores de arriba evitan esto):

- **macOS**: hacé click derecho sobre el binario y elegí *Abrir* la primera vez
  (o limpiá la marca de cuarentena con `xattr -d com.apple.quarantine ./typebar`);
  después corre normal.
- **Windows**: si aparece SmartScreen, elegí *Más información -> Ejecutar de
  todas formas*.

### Compilar desde el código

Requiere **Rust 1.85+** (edición 2024).

```bash
git clone https://github.com/rcantore/typebar.git
cd typebar
cargo run --release -- notas.md
```

O compilá el binario:

```bash
cargo build --release
./target/release/typebar notas.md
```

### Uso por línea de comandos

```
typebar [PATH] [--keys <preset>]
```

- `PATH`: archivo a abrir (default `scratch.md` si se omite).
- `--keys <preset>`: preset de teclado (`standard`, `vim` o `wordstar`).
  Tiene prioridad sobre el archivo de config.
- `--export-html`: convierte `PATH` a un archivo `.html` standalone y sale, sin
  abrir el editor.
- `-h`, `--help`: imprime la ayuda y sale.

```bash
typebar README.md --keys vim
typebar              # abre scratch.md con el preset standard
typebar notas.md --export-html   # escribe notas.html y sale
typebar --help                   # imprime la ayuda y sale
```

## Configuración

typebar lee un archivo TOML opcional en `~/.config/typebar/config.toml`
(respeta `XDG_CONFIG_HOME`). Todo es opcional; sin el archivo, se usan los
defaults. Hay un punto de partida en
[`examples/config.toml`](examples/config.toml).

```toml
[keybindings]
# "standard" (default) | "vim" | "wordstar".
# El flag --keys de la CLI tiene prioridad sobre esto.
preset = "standard"

# Overrides por tecla, aplicados encima del preset. `mode` es opcional
# ("normal" | "insert" | "visual"); si se omite, el binding aplica en cualquier modo.
[[keybindings.bind]]
keys = "ctrl-s"
action = "save"

[ui]
# "frappe" (default) | "mocha" | "latte" (claro). Nombres desconocidos caen a frappe.
theme = "frappe"

# Idioma de la UI: "en" | "es". Default inglés, o tu $LANG si es español.
locale = "es"

# Nivel WYSIWYG: 1 (markers siempre visibles) o 2 (ocultos fuera de la línea
# activa). Default 2; valores inválidos caen a 2.
wysiwyg_level = 2

# Captura del mouse: habilita el click en las tabs de buffers. Off por default
# (deja intacta la selección nativa del terminal).
mouse = false
```

**Precedencia de presets:** flag `--keys` → `preset` del config → default
built-in (`standard`).

Las acciones bindeables incluyen `cursor-{left,right,up,down}`,
`line-{start,end}`, `doc-{start,end}`, `page-{up,down}`,
`enter-{insert,normal,visual}`, `insert-after`, `open-line-below`,
`select-{left,right,up,down}`, `delete-selection`, `delete-char`, `backspace`,
`insert-newline`, `toggle-{bold,italic,code}`, `undo`, `redo`, `yank`, `paste`,
`search`, `replace`, `save`, `save-and-quit` y `quit`.

## Desarrollo

```bash
cargo build              # compilar
cargo test               # correr los tests
cargo fmt --check        # formato (exigido en CI)
cargo clippy --all-targets -- -D warnings   # lints (exigidos en CI)
```

La arquitectura y el pipeline de renderizado están documentados en
[`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md).

## Contribuir

¡Las contribuciones son bienvenidas! Leé [`CONTRIBUTING.es.md`](CONTRIBUTING.es.md)
para saber cómo compilar, testear y enviar cambios, y tené en cuenta el
[Código de Conducta](CODE_OF_CONDUCT.md). Los cambios relevantes se registran en
el [changelog](CHANGELOG.md).

## Licencia

Licenciado bajo [MIT](LICENSE-MIT) o [Apache-2.0](LICENSE-APACHE), a tu
elección.
