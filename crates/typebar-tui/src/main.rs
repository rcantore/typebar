//! typebar: editor de Markdown WYSIWYG para terminal (milestone editable minimo).
//!
//! Render "soft WYSIWYG": los marcadores (`**`, `*`, backticks, `#`) siempre
//! quedan visibles y dimmeados. El mapeo cursor->pantalla YA NO es 1:1 linea a
//! linea: `draw` corre `render::render` (una `Line` por linea del documento,
//! ver `render.rs`) a traves de la capa de soft wrap (`crate::wrap`), que la
//! parte en filas visuales segun el ancho del viewport. Scroll y cursor
//! razonan en esas filas visuales via `WrapLayout::row_and_x`.
//!
//! El teclado se maneja por presets intercambiables (ver `keybinding`): por
//! default `standard` (modeless, flechas), con `vim` (modal) y `wordstar`
//! (modeless con chords tipo `Ctrl-K S`) opt-in via el flag `--keys`. El loop
//! acumula teclas en un buffer `pending` para resolver secuencias multi-tecla.

mod config;
mod keybinding;
mod overlay;
mod palette;
mod render;
mod switcher;
mod tabs;
mod theme;
mod theme_picker;
mod wrap;

use typebar_core::document::{Document, Mode};
use typebar_core::markdown::InlineKind;
use typebar_core::{buffers, export, files, i18n};

use keybinding::{
    Action, Binding, CustomKeymap, Keymap, Resolve, keymap_from_name, next_preset, parse_binding,
};
use overlay::Overlay;
use palette::{Palette, PaletteOutcome};
use switcher::{Switcher, SwitcherOutcome};
use theme::Theme;
use theme_picker::{ThemeOutcome, ThemePicker};

use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
};
use ratatui::layout::{Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Padding, Paragraph};

const DEFAULT_PATH: &str = "scratch.md";
const DEFAULT_PRESET: &str = "standard";
/// Ancho (en columnas, incluido el margen izquierdo) de la "hoja" del modo
/// whitepaper: el texto se centra en una columna de a lo sumo este ancho para la
/// sensacion typewriter/papel. Si la terminal es mas angosta, se usa todo el ancho.
const WHITEPAPER_WIDTH: u16 = 72;

/// Args parseados de la linea de comandos. `preset` es `None` cuando el usuario
/// no paso `--keys`: distinguir "no especificado" de un valor concreto es lo que
/// permite que el config file tenga la chance de aplicar su preset.
struct Args {
    path: String,
    preset: Option<String>,
    /// Si esta en `true` (flag `--export-html`), el programa convierte el
    /// archivo a HTML standalone y sale sin abrir la TUI.
    export_html: bool,
    /// Si esta en `true` (flag `--help`/`-h`), el programa imprime la ayuda por
    /// stdout y sale sin abrir la TUI. Tiene prioridad sobre el resto.
    help: bool,
}

/// Parsea los argumentos a mano (sin clap). Soporta `--keys <nombre>` y
/// `--keys=<nombre>` en cualquier posicion; el primer posicional (no-flag) es
/// el path del archivo. Default de path: `scratch.md`. El preset queda en
/// `None` si no se paso `--keys` (lo resuelve luego `resolve_preset`).
fn parse_args(raw: impl Iterator<Item = String>) -> Args {
    let mut path: Option<String> = None;
    let mut preset: Option<String> = None;
    let mut export_html = false;
    let mut help = false;
    let mut args = raw.peekable();

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--keys=") {
            preset = Some(value.to_string());
        } else if arg == "--keys" {
            // Tomar el siguiente token como valor (si lo hay).
            preset = args.next();
        } else if arg == "--export-html" {
            // Flag booleano (sin valor): exportar a HTML y salir.
            export_html = true;
        } else if arg == "--help" || arg == "-h" {
            // Imprimir la ayuda y salir (lo resuelve `main`, antes que todo).
            help = true;
        } else if !arg.starts_with('-') && path.is_none() {
            path = Some(arg);
        }
        // Cualquier otro flag desconocido se ignora silenciosamente.
    }

    Args {
        path: path.unwrap_or_else(|| DEFAULT_PATH.to_string()),
        preset,
        export_html,
        help,
    }
}

/// Imprime la ayuda de la linea de comandos por stdout: uso, el argumento
/// posicional (archivo) y los flags soportados. En ingles (convencion CLI); el
/// resto de la UI interactiva si esta i18n-izada.
fn print_help() {
    println!(
        "typebar - a WYSIWYG Markdown editor for the terminal\n\
\n\
USAGE:\n    \
    typebar [OPTIONS] [FILE]\n\
\n\
ARGS:\n    \
    [FILE]    Markdown file to open (default: {DEFAULT_PATH})\n\
\n\
OPTIONS:\n    \
    --keys <PRESET>    Keybinding preset: standard | vim | wordstar\n    \
    --export-html      Export FILE to standalone HTML and exit\n    \
    -h, --help         Print this help and exit"
    );
}

/// Resuelve el preset final aplicando la precedencia: flag CLI `--keys` > preset
/// del config file > default built-in (`standard`). El preset del config se
/// valida en el borde: si trae un nombre desconocido se avisa por stderr y se
/// ignora (cae al default). El flag CLI NO se valida aca: `keymap_from_name` ya
/// cae a `standard` ante un nombre raro, manteniendo el comportamiento previo.
fn resolve_preset(cli_preset: Option<String>, config: &config::Config) -> String {
    if let Some(name) = cli_preset {
        return name;
    }
    match config.keybindings.preset.as_deref() {
        Some(name) if config::is_known_preset(name) => name.to_string(),
        Some(name) => {
            eprintln!("{}", i18n::error_unknown_preset(name, DEFAULT_PRESET));
            DEFAULT_PRESET.to_string()
        }
        None => DEFAULT_PRESET.to_string(),
    }
}

/// Aplica los overrides de teclas del usuario encima del preset `base`. Cada
/// entrada se parsea en el borde: las invalidas se reportan por stderr y se
/// descartan (igual que el resto de la config, el editor arranca igual). Si no
/// queda ningun override valido, devuelve el preset base sin envolver.
fn apply_overrides(base: Box<dyn Keymap>, entries: &[config::BindEntry]) -> Box<dyn Keymap> {
    let mut bindings: Vec<Binding> = Vec::new();
    for entry in entries {
        match parse_binding(&entry.keys, &entry.action, entry.mode.as_deref()) {
            Ok(binding) => bindings.push(binding),
            Err(err) => eprintln!(
                "{}",
                i18n::error_invalid_keybinding(&entry.keys, &entry.action, err)
            ),
        }
    }
    if bindings.is_empty() {
        base
    } else {
        Box::new(CustomKeymap::new(base, bindings))
    }
}

/// Calcula el path de salida del HTML a partir del path del archivo de entrada:
/// reemplaza la extension por `.html` (ej `notes.md` -> `notes.html`); si no
/// tiene extension, le agrega `.html` (ej `notes` -> `notes.html`).
fn html_output_path(input: &str) -> std::path::PathBuf {
    std::path::Path::new(input).with_extension("html")
}

/// Exporta el archivo Markdown en `path` a un HTML standalone junto a el (misma
/// ruta, extension `.html`) y avisa por stderr. Si el archivo no existe, se
/// trata su contenido como vacio (genera un HTML valido pero sin cuerpo). El
/// resto de los errores de IO (lectura/escritura) se propagan.
fn export_to_html(path: &str) -> std::io::Result<()> {
    // Un archivo inexistente se trata como contenido vacio; el resto de los
    // errores de lectura (permisos, etc.) se propagan.
    let content = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    let html = export::to_html(&content, path);
    let out = html_output_path(path);
    std::fs::write(&out, html)?;
    eprintln!("{}", i18n::exported_to(&out));
    Ok(())
}

/// Exporta el buffer activo (su texto en memoria, con cambios sin guardar) a un
/// HTML standalone junto al archivo (misma ruta, extension `.html`) y devuelve el
/// path de salida. A diferencia de `export_to_html`, NO lee del disco: usa el
/// contenido actual del documento, asi exporta lo que se ve aunque no se haya
/// guardado todavia. El titulo de la pagina es el path del archivo.
fn export_doc_to_html(doc: &Document) -> std::io::Result<std::path::PathBuf> {
    let title = doc.path.to_string_lossy();
    let html = export::to_html(&doc.text(), &title);
    let out = doc.path.with_extension("html");
    std::fs::write(&out, html)?;
    Ok(out)
}

/// Calcula el path del HTML print-ready para "exportar a PDF via el
/// navegador": el stem del archivo del doc (ej `notes.md` -> `notes`) mas el
/// sufijo `.print.html`, bajo el directorio temporal del sistema (para no
/// ensuciar el directorio del usuario, a diferencia del HTML de
/// `export_doc_to_html` que va al lado del archivo). Si el doc no tiene stem
/// (ej path vacio), cae a `untitled`.
fn pdf_temp_path(doc_path: &std::path::Path) -> std::path::PathBuf {
    let stem = doc_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("untitled");
    std::env::temp_dir().join(format!("{stem}.print.html"))
}

/// Exporta el buffer activo (su texto en memoria, con cambios sin guardar) a un
/// HTML print-ready para "exportar a PDF via el navegador": igual que
/// `export_doc_to_html`, pero con el script de auto-print
/// (`export::to_html_print`) y escrito bajo el directorio temporal (ver
/// `pdf_temp_path`), no al lado del archivo. Devuelve el path escrito; el
/// caller lo abre en el navegador.
fn export_doc_to_pdf(doc: &Document) -> std::io::Result<std::path::PathBuf> {
    let title = doc.path.to_string_lossy();
    let html = export::to_html_print(&doc.text(), &title);
    let out = pdf_temp_path(&doc.path);
    std::fs::write(&out, html)?;
    Ok(out)
}

/// Abre `path` en el navegador default del sistema sin esperar a que el
/// proceso termine (`spawn`, no `status`/`output`): el editor sigue andando
/// mientras el usuario ve/imprime la pagina. Un comando distinto por
/// plataforma, todos ya presentes en el sistema (sin dependencias nuevas).
#[cfg(target_os = "macos")]
fn open_in_browser(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("open").arg(path).spawn()?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn open_in_browser(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("xdg-open").arg(path).spawn()?;
    Ok(())
}

#[cfg(target_os = "windows")]
fn open_in_browser(path: &std::path::Path) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(path)
        .spawn()?;
    Ok(())
}

fn main() -> std::io::Result<()> {
    // Saltar argv[0] (nombre del binario).
    let args = parse_args(std::env::args().skip(1));

    // Ayuda: imprimir y salir, antes que cualquier otra cosa (no abre la TUI ni
    // toca el config).
    if args.help {
        print_help();
        return Ok(());
    }

    // Export a HTML: convertir y salir ANTES de inicializar la terminal.
    if args.export_html {
        return export_to_html(&args.path);
    }

    // Cargar config primero; el override del CLI se aplica encima en
    // `resolve_preset`. Sin config file valido, esto cae a defaults en silencio.
    let config = match config::config_path() {
        Some(path) => config::load_from_path(&path),
        None => config::Config::default(),
    };

    // Resolver y fijar el idioma de la UI ANTES de leer cualquier label
    // (los presets traducen sus hints en `keymap.hints`, los mensajes de error
    // que vienen abajo tambien). Precedencia: config > $LANG/$LC_ALL > default.
    let locale = config
        .ui
        .locale
        .as_deref()
        .and_then(i18n::Locale::from_str)
        .unwrap_or_else(i18n::Locale::from_env);
    i18n::init(locale);

    let preset = resolve_preset(args.preset, &config);
    // Overrides del usuario: se sacan de `config` aca (no solo se piden prestados)
    // porque `run` los necesita enteros, via `KeymapState`, para poder reconstruir
    // el keymap si el usuario cicla de preset en runtime (`CycleKeymapPreset`).
    let binds = config.keybindings.bind;
    // Preset base + overrides del usuario encima (si los hay).
    let keymap = apply_overrides(keymap_from_name(&preset), &binds);

    // Themes para el toggle `^O L` (claro <-> oscuro en runtime). El claro es
    // siempre Latte; el oscuro es el configurado, salvo que el config YA sea Latte
    // (ahi el oscuro cae a frappe, para que el toggle tenga a donde ir). El editor
    // arranca mostrando el theme que el usuario eligio: si configuro Latte, arranca
    // en claro. `by_name` cae a `frappe` ante un nombre desconocido. El modo
    // whitepaper usa aparte un theme propio (`Theme::paper()`, monocromo).
    let configured_is_light = config.ui.theme.eq_ignore_ascii_case("latte");
    let light_theme = Theme::latte();
    let dark_theme = if configured_is_light {
        Theme::frappe()
    } else {
        Theme::by_name(&config.ui.theme)
    };
    // Id del theme BASE (oscuro) con el que arranca el theme picker: si el config
    // ya es Latte, la base oscura cae a frappe (para que el toggle claro/oscuro
    // tenga a donde ir), asi que el id base es "frappe".
    let base_theme_id = if configured_is_light {
        theme::DEFAULT_THEME.to_string()
    } else {
        config.ui.theme.clone()
    };
    let wysiwyg_level = config.ui.resolved_wysiwyg_level();

    let mut document = Document::open(&args.path)?;
    document.mode = keymap.initial_mode();

    let mut terminal = ratatui::init();
    // Captura del mouse (opt-in por config): habilita el click en las tabs. Por
    // default off, para no robarle al terminal su seleccion nativa. Si falla, se
    // ignora (el editor anda igual, solo sin mouse).
    if config.ui.mouse {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::EnableMouseCapture
        );
    }
    let result = run(
        &mut terminal,
        document,
        KeymapState {
            keymap,
            preset_id: preset,
            binds,
        },
        Themes {
            dark: dark_theme,
            light: light_theme,
            paper: Theme::paper(),
        },
        configured_is_light,
        base_theme_id,
        wysiwyg_level,
    );
    if config.ui.mouse {
        let _ = ratatui::crossterm::execute!(
            std::io::stdout(),
            ratatui::crossterm::event::DisableMouseCapture
        );
    }
    ratatui::restore();
    result
}

/// Prompt de confirmacion modal: mientras vive, `run` le entrega las teclas al
/// prompt (no al documento) hasta que el usuario resuelve. Por ahora la unica
/// variante es cerrar un buffer con cambios sin guardar; es un enum para que el
/// Quit (que hoy sale sin preguntar) pueda reusar el mismo patron mas adelante.
enum Confirm {
    /// Cerrar el buffer activo, que esta dirty: `[s]` guarda y cierra, `[d]`
    /// descarta y cierra, `[c]`/Esc cancela.
    CloseBuffer,
}

/// Estado de VISTA del run loop: todo lo que vive entre frames y que `draw` lee y
/// las acciones tocan, pero que NO es ni el documento, ni el keymap, ni los themes,
/// ni el nivel de wysiwyg (ese contexto es aparte, fijo durante el run). Agrupar
/// estos campos en un struct evita threadear 8-9 params sueltos por `draw` y
/// `dispatch_action`. Lo posee `run`.
struct AppState {
    /// Offset vertical de scroll: primera FILA VISUAL visible (post soft wrap,
    /// ver `crate::wrap`), no linea del documento. `draw` lo ajusta para
    /// mantener el cursor dentro del viewport, y lo clampea contra el total de
    /// filas visuales del layout (una edicion puede haber achicado el doc).
    scroll: usize,
    /// Alto del area de edicion (en lineas) tras el ultimo draw. Lo escribe
    /// `draw`; lo leen las acciones que dependen del viewport (PageUp/PageDown).
    /// Antes del primer draw queda en 1, fallback razonable para no entregar 0 a
    /// un calculo de pagina.
    viewport_height: usize,
    /// Zen/focus mode: oculta el chrome (borde, toolbar, status) para dejar solo
    /// el texto. Estado de la vista, no del documento. Se togglea con el submenu
    /// "view" y, en presets modeless, sale tambien con Esc.
    zen: bool,
    /// Modo whitepaper: orquesta zen (chrome oculto) + theme monocromo de papel
    /// (tinta sobre papel, `Theme::paper()`) + una columna de ancho fijo centrada,
    /// para la sensacion "hoja de papel". Estado de la vista; cuando esta activo,
    /// `run` fuerza el theme de papel y `draw` centra el editor en una columna de
    /// `WHITEPAPER_WIDTH` y dibuja un cursor sintetico visible sobre el fondo claro.
    whitepaper: bool,
    /// Toggle del theme claro (Latte) en runtime (`^O L`). Estado de la vista; el
    /// theme activo se calcula en `run` a partir de este flag.
    light_on: bool,
    /// Buffer de teclas de un chord en curso (vacio si no hay nada pendiente).
    pending: Vec<KeyEvent>,
    /// Overlay de busqueda/reemplazo activo (None = edicion normal).
    overlay: Option<Overlay>,
    /// Switcher de archivos (fuzzy finder) activo (None = edicion normal). Opera a
    /// nivel workspace: al aceptar, abre/cambia de buffer. Tapa el editor.
    switcher: Option<Switcher>,
    /// Paleta de comandos (fuzzy sobre los Action) activa (None = edicion normal).
    /// Al aceptar, despacha el Action por el mismo camino que el keymap.
    palette: Option<Palette>,
    /// Theme picker activo (None = edicion normal). Mientras vive, `run` dibuja
    /// cada frame con el theme resaltado (preview en vivo); al aceptar, fija el
    /// elegido como base. Tapa el editor como los otros pickers.
    theme_picker: Option<ThemePicker>,
    /// Id del theme BASE (oscuro) activo, para marcar el "actual" al abrir el theme
    /// picker y para saber que persistir. Lo actualiza el picker al aceptar; los
    /// toggles de vista (claro/papel) no lo tocan (siguen siendo capas encima).
    theme_id: String,
    /// Mensaje transitorio en la status bar (ej "save failed: ..."): se muestra en
    /// el proximo frame y se limpia al apretar la siguiente tecla. Evita que un
    /// error de guardado tumbe el editor (writing-first: nunca perder el buffer).
    flash: Option<String>,
    /// Prompt de confirmacion modal activo (None = edicion normal). Mientras vive,
    /// `run` le entrega las teclas y dibuja su linea en lugar de la status bar.
    confirm: Option<Confirm>,
}

impl AppState {
    /// Estado inicial del run loop: sin scroll, viewport en 1 (fallback antes del
    /// primer draw), sin chord pendiente ni overlays/pickers. `light_on` arranca
    /// segun el theme configurado (claro o no).
    fn new(light_on: bool) -> Self {
        AppState {
            scroll: 0,
            viewport_height: 1,
            zen: false,
            whitepaper: false,
            light_on,
            pending: Vec::new(),
            overlay: None,
            switcher: None,
            palette: None,
            theme_picker: None,
            // Se sobreescribe en `run` con el id realmente configurado; "frappe" es
            // el default coherente con `DEFAULT_THEME`.
            theme_id: theme::DEFAULT_THEME.to_string(),
            flash: None,
            confirm: None,
        }
    }
}

/// Los tres themes disponibles en runtime: el oscuro (el configurado), el claro
/// (Latte, toggle `^O L`) y el de papel (modo whitepaper, `^O W`). Se arman una
/// vez en `main`; `run` elige cual usar cada frame segun el estado de la vista.
struct Themes {
    dark: Theme,
    light: Theme,
    paper: Theme,
}

/// El keymap activo junto con lo necesario para reconstruirlo en runtime
/// (`CycleKeymapPreset`): el id del preset actual y los overrides del usuario.
/// Agrupar estos tres campos evita threadearlos como params sueltos por `run` y
/// `dispatch_action`.
struct KeymapState {
    keymap: Box<dyn Keymap>,
    /// Id del preset activo (standard/vim/wordstar). NO se lee de `keymap.name()`:
    /// un `CustomKeymap` delega el nombre al preset base y podria no calzar con
    /// este id.
    preset_id: String,
    /// Overrides del usuario (config), para reaplicarlos encima del preset nuevo
    /// al ciclar (`CycleKeymapPreset`).
    binds: Vec<config::BindEntry>,
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    doc: Document,
    mut keys: KeymapState,
    mut themes: Themes,
    light_on: bool,
    theme_id: String,
    wysiwyg_level: u8,
) -> std::io::Result<()> {
    // Los buffers abiertos. El editor siempre opera sobre el activo
    // (`workspace.active*`); el multi-archivo es transparente para draw/acciones/
    // overlays. Arranca con el documento que abrio `main`.
    let mut workspace = buffers::Workspace::new(doc);
    // Estado de vista del loop (scroll, zen, overlay, pickers, etc.) agrupado.
    let mut state = AppState::new(light_on);
    // El theme base configurado (para el marcador "actual" del theme picker y para
    // la persistencia del theme elegido).
    state.theme_id = theme_id;

    loop {
        // Theme activo (owned; `Theme` es `Copy`, asi que copiarlo es barato). Base:
        // el modo whitepaper usa su theme monocromo (tinta sobre papel) y gana sobre
        // todo; si no, el claro (Latte) cuando el toggle `^O L` esta on; si no, el
        // base (oscuro). Con el theme picker abierto, el theme RESALTADO pisa la base
        // (preview en vivo: el editor entero se ve con el theme que estas mirando).
        // Se recalcula cada frame.
        let base = if state.whitepaper {
            themes.paper
        } else if state.light_on {
            themes.light
        } else {
            themes.dark
        };
        let theme = state
            .theme_picker
            .as_ref()
            .and_then(|tp| tp.highlighted_theme())
            .unwrap_or(base);
        // Barra de tabs de los buffers abiertos (solo con >=2 y con el chrome
        // visible, es decir fuera de zen y de whitepaper). `tab_line` es lo que
        // dibuja `draw`; `tab_hits` mapea columna->buffer para el click del mouse.
        let (tab_line, tab_hits) = if !state.zen && !state.whitepaper && workspace.count() >= 2 {
            let titles: Vec<String> = workspace
                .paths()
                .map(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| p.to_string_lossy().into_owned())
                })
                .collect();
            let (line, hits) = tabs::build(&titles, workspace.active_index(), &theme);
            (Some(line), hits)
        } else {
            (None, Vec::new())
        };
        terminal.draw(|frame| {
            draw(
                frame,
                workspace.active(),
                keys.keymap.as_ref(),
                &theme,
                wysiwyg_level,
                &mut state,
                tab_line.clone(),
            );
            // Paperwhite: si el theme activo es claro, pinta fondo/texto sobre el
            // frame ya dibujado (editor, chrome y pickers de una). No-op en oscuros.
            apply_theme_fill(frame, &theme);
        })?;

        let ev = event::read()?;
        // Click izquierdo en la fila de tabs (y=0): cambia de buffer. Si la captura
        // del mouse esta off (el default), estos eventos no llegan nunca.
        if let Event::Mouse(me) = ev {
            if me.kind == MouseEventKind::Down(MouseButton::Left)
                && me.row == 0
                && let Some(hit) = tab_hits.iter().find(|t| t.cols.contains(&me.column))
            {
                workspace.switch_to(hit.index);
                state.scroll = 0;
            }
            continue;
        }
        let Event::Key(key) = ev else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        // Cualquier tecla limpia un mensaje flash previo (ej un error de guardado).
        state.flash = None;

        // Con un prompt de confirmacion abierto, las teclas las consume el prompt
        // (es modal): `s` guarda y cierra, `d` descarta y cierra, `c`/Esc cancela.
        // Cualquier otra tecla se ignora (el prompt sigue vivo). Tras cerrar, el
        // buffer recien enfocado arranca arriba, asi que se resetea el scroll.
        if state.confirm.is_some() {
            match key.code {
                KeyCode::Char('s') => {
                    state.confirm = None;
                    // Guardar y, solo si sale bien, cerrar. Si el guardado falla, NO
                    // cerramos (no perder el buffer): se muestra el error y el buffer
                    // queda abierto, intacto.
                    match workspace.active_mut().save() {
                        Ok(()) => {
                            workspace.close_active(keys.keymap.initial_mode());
                            state.scroll = 0;
                        }
                        Err(e) => state.flash = Some(format!("save failed: {e}")),
                    }
                }
                KeyCode::Char('d') => {
                    state.confirm = None;
                    workspace.close_active(keys.keymap.initial_mode());
                    state.scroll = 0;
                }
                KeyCode::Char('c') | KeyCode::Esc => state.confirm = None,
                _ => {}
            }
            continue;
        }

        // Con la paleta abierta, las teclas las consume la paleta (tipear filtra,
        // flechas/Ctrl-N/P navegan, Enter ejecuta el comando, Esc cancela). Al
        // aceptar, despachamos el Action por el MISMO camino que un action del
        // keymap (ver `dispatch_action`), asi no se duplica logica.
        if let Some(pal) = state.palette.as_mut() {
            match pal.handle_key(key) {
                PaletteOutcome::Stay => {}
                PaletteOutcome::Cancel => state.palette = None,
                PaletteOutcome::Accept(action) => {
                    state.palette = None;
                    let before = workspace.active_index();
                    match dispatch_action(action, &mut workspace, &mut keys, &mut state) {
                        Ok(true) => return Ok(()),
                        Ok(false) => {}
                        // Un error (de guardado) NO tumba el editor: se muestra y sigue.
                        Err(e) => state.flash = Some(format!("save failed: {e}")),
                    }
                    if workspace.active_index() != before {
                        state.scroll = 0; // el buffer recien enfocado arranca arriba
                    }
                }
            }
            continue;
        }

        // Con el theme picker abierto, las teclas las consume el picker (tipear
        // filtra, flechas/Ctrl-N/P navegan y PREVIEWAN en vivo, Enter fija, Esc
        // cancela volviendo al theme que estaba). Al aceptar, el elegido pasa a ser
        // la base oscura y se apagan los toggles claro/papel para que se vea tal
        // cual lo elegiste.
        if let Some(tp) = state.theme_picker.as_mut() {
            match tp.handle_key(key) {
                ThemeOutcome::Stay => {}
                ThemeOutcome::Cancel => state.theme_picker = None,
                ThemeOutcome::Accept(id) => {
                    state.theme_picker = None;
                    themes.dark = Theme::by_name(id);
                    state.theme_id = id.to_string();
                    state.light_on = false;
                    state.whitepaper = false;
                    // Persistir la eleccion en el config para que sobreviva al
                    // reinicio. Best-effort: si falla (sin dir de config, permisos)
                    // se avisa en el flash, pero el theme ya cambio en vivo igual.
                    if let Err(e) = config::persist_theme(id) {
                        state.flash = Some(format!("theme not saved: {e}"));
                    }
                }
            }
            continue;
        }

        // Con el switcher abierto, las teclas las consume el switcher (tipear
        // filtra, flechas/Ctrl-N/P navegan, Enter abre el elegido, Esc cancela).
        if let Some(sw) = state.switcher.as_mut() {
            match sw.handle_key(key) {
                SwitcherOutcome::Stay => {}
                SwitcherOutcome::Cancel => state.switcher = None,
                SwitcherOutcome::Accept(path) => {
                    state.switcher = None;
                    // Abrir o cambiar al buffer. Si el archivo no se puede abrir,
                    // lo ignoramos y seguimos en el buffer actual.
                    if workspace
                        .open_or_switch(&path, keys.keymap.initial_mode())
                        .is_ok()
                    {
                        state.scroll = 0; // el buffer recien enfocado arranca arriba
                    }
                }
            }
            continue;
        }

        // Con un overlay abierto, las teclas las consume el overlay (escribir el
        // termino, navegar, confirmar o cancelar), no el documento.
        if let Some(ov) = state.overlay.as_mut() {
            if ov.handle_key(workspace.active_mut(), key) {
                state.overlay = None;
            }
            continue;
        }

        // Red de seguridad para salir de zen o whitepaper: en presets modeless
        // (standard/wordstar) `Esc` no esta bindeado, asi que lo usamos como escape
        // garantizado de los modos focus (con el chrome oculto el toggle no se ve).
        // Limpia ambos flags. En Vim NO lo interceptamos: `Esc` tiene semantica
        // (volver a Normal); ahi se sale con el mismo `z z`/`z w` que entro.
        if (state.zen || state.whitepaper) && key.code == KeyCode::Esc && !keys.keymap.is_modal() {
            state.zen = false;
            state.whitepaper = false;
            state.pending.clear();
            continue;
        }

        state.pending.push(key);
        match keys.keymap.resolve(workspace.active().mode, &state.pending) {
            Resolve::Action(action) => {
                state.pending.clear();
                // Despachamos por el mismo helper que usa la paleta, asi un action
                // resuelto por el keymap y uno elegido en la paleta recorren un
                // unico camino (sin duplicar la logica de overlays/zen/switcher).
                // Un error (de guardado) NO tumba el editor: se muestra y sigue. Si
                // la accion cambio de buffer, se resetea el scroll compartido.
                let before = workspace.active_index();
                match dispatch_action(action, &mut workspace, &mut keys, &mut state) {
                    Ok(true) => return Ok(()),
                    Ok(false) => {}
                    Err(e) => state.flash = Some(format!("save failed: {e}")),
                }
                if workspace.active_index() != before {
                    state.scroll = 0;
                }
            }
            // La secuencia es prefijo de un chord: esperar mas teclas.
            Resolve::Pending => {}
            // Secuencia no bindeada: cancela el chord (o un Esc tras un
            // prefijo) limpiando el buffer pendiente.
            Resolve::None => state.pending.clear(),
        }
    }
}

/// Despacha un `Action` resuelto (sea por el keymap o elegido en la paleta) sobre
/// el estado del loop. Devuelve `Ok(true)` si hay que salir del editor
/// (Quit/SaveAndQuit). Las acciones "de vista" (Search/Replace abren overlay,
/// ToggleZen togglea la vista, OpenSwitcher/OpenPalette abren su picker) las
/// maneja aca; el resto va por `apply_action` sobre el buffer activo.
///
/// Centralizar esto en un solo lugar evita duplicar la logica entre el match del
/// keymap y el `Accept` de la paleta, y garantiza que ambos caminos se comporten
/// igual (incluido salir del `run` ante Quit/SaveAndQuit).
fn dispatch_action(
    action: Action,
    workspace: &mut buffers::Workspace,
    keys: &mut KeymapState,
    state: &mut AppState,
) -> std::io::Result<bool> {
    match action {
        // Estas acciones tocan estado de la vista del loop, no el doc.
        Action::Search => state.overlay = Some(Overlay::new_search(workspace.active())),
        Action::Replace => state.overlay = Some(Overlay::new_replace()),
        Action::ToggleZen => state.zen = !state.zen,
        // Togglear el modo whitepaper (submenu view): zen + claro + columna
        // centrada. El theme claro y el centrado los aplican `run`/`draw` segun el
        // flag; aca solo se togglea.
        Action::ToggleWhitepaper => state.whitepaper = !state.whitepaper,
        // Togglear el theme claro (Latte) <-> oscuro en runtime (submenu view).
        Action::ToggleLightTheme => state.light_on = !state.light_on,
        // Ciclar el preset de keybindings activo (submenu view): reconstruye el
        // keymap con el preset siguiente (los overrides del usuario se reaplican
        // encima), refleja el cambio en la status bar/toolbar (se recalculan cada
        // frame a partir de `keys.keymap`) y lo persiste en el config. `preset_id`
        // (no `keymap.name()`) es la fuente de verdad del preset actual, porque un
        // `CustomKeymap` delega el nombre al preset base.
        Action::CycleKeymapPreset => {
            let next = next_preset(&keys.preset_id);
            keys.keymap = apply_overrides(keymap_from_name(next), &keys.binds);
            keys.preset_id = next.to_string();
            state.flash = Some(i18n::keymap_set_to(next));
            // Persistir best-effort, igual que el theme picker: si falla se avisa
            // en el flash, pero el keymap ya cambio en vivo de todos modos.
            if let Err(e) = config::persist_preset(next) {
                state.flash = Some(format!("keymap not saved: {e}"));
            }
        }
        // Exportar el buffer actual a HTML sin salir; el resultado (path o error)
        // va al flash, no tumba el editor. Escribe a `<archivo>.html` al lado.
        Action::ExportHtml => {
            state.flash = Some(match export_doc_to_html(workspace.active()) {
                Ok(out) => i18n::exported_to(&out),
                Err(e) => i18n::export_failed(e),
            });
        }
        // Exportar a PDF "via el navegador": HTML print-ready en el directorio
        // temporal, abierto en el navegador default para que el usuario guarde
        // como PDF desde el dialogo de impresion. Cualquier falla (export o
        // apertura) va al flash, no tumba el editor.
        Action::ExportPdf => {
            state.flash = Some(match export_doc_to_pdf(workspace.active()) {
                Ok(out) => match open_in_browser(&out) {
                    Ok(()) => i18n::exported_to(&out),
                    Err(e) => i18n::export_failed(e),
                },
                Err(e) => i18n::export_failed(e),
            });
        }
        // Nuevo archivo: crea un buffer vacio y lo enfoca. El draw reclampa el
        // scroll solo (el cursor del buffer nuevo arranca arriba).
        Action::NewBuffer => workspace.new_buffer(keys.keymap.initial_mode()),
        // Cambiar de buffer (cycle). El draw reclampa el scroll al nuevo buffer.
        Action::NextBuffer => workspace.next_buffer(),
        Action::PrevBuffer => workspace.prev_buffer(),
        // Cerrar el buffer activo. Con cambios sin guardar abrimos el prompt de
        // confirmacion (lo resuelve el loop); si esta limpio, cerramos directo. El
        // draw reclampa el scroll al buffer que queda enfocado.
        Action::CloseBuffer => {
            if workspace.active().dirty {
                state.confirm = Some(Confirm::CloseBuffer);
            } else {
                workspace.close_active(keys.keymap.initial_mode());
            }
        }
        Action::OpenSwitcher => {
            // Candidatos: PRIMERO los buffers abiertos (lo mas relevante de
            // alcanzar va arriba), con su marca dirty real; DESPUES los archivos
            // del proyecto (cwd recursivo) que no esten ya abiertos. Dedup por path.
            let mut candidates: Vec<std::path::PathBuf> = Vec::new();
            let mut unsaved: Vec<bool> = Vec::new();
            for (path, is_unsaved) in workspace.buffers() {
                candidates.push(path.to_path_buf());
                unsaved.push(is_unsaved);
            }
            for p in files::discover(".") {
                if !candidates.iter().any(|c| c == &p) {
                    candidates.push(p);
                    unsaved.push(false);
                }
            }
            state.switcher = Some(Switcher::new(candidates, unsaved));
        }
        // Abrir la paleta de comandos. Como `OpenPalette` se excluye del catalogo
        // de comandos, no hay forma de recursar desde la propia paleta.
        Action::OpenPalette => state.palette = Some(Palette::new(keys.keymap.as_ref())),
        // Abrir el theme picker. El "actual" que marca es lo que se ve ahora: papel
        // o claro si algun toggle de vista esta activo, si no la base oscura.
        Action::OpenThemePicker => {
            let current = if state.whitepaper {
                "paper"
            } else if state.light_on {
                "latte"
            } else {
                state.theme_id.as_str()
            };
            state.theme_picker = Some(ThemePicker::new(current));
        }
        _ => return apply_action(workspace.active_mut(), action, state.viewport_height),
    }
    Ok(false)
}

/// Post-pass de "paperwhite": en un theme con `background` y `text` definidos
/// (los claros, ej Latte), recorre el frame YA dibujado y pinta el fondo en cada
/// celda sin fondo propio y el color de texto en cada celda sin fg propio. Asi un
/// solo lugar deja el editor, el chrome y los pickers sobre un fondo claro, sin
/// tener que threadear el color por cada widget. En los themes oscuros
/// (`background`/`text` en `None`) es no-op: no toca el render y siguen
/// transparentes (dejan pasar el fondo del terminal).
fn apply_theme_fill(frame: &mut ratatui::Frame, theme: &Theme) {
    let (Some(bg), Some(fg)) = (theme.background, theme.text) else {
        return;
    };
    let buf = frame.buffer_mut();
    let area = buf.area;
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            let cell = &mut buf[(x, y)];
            // `Reset` es "sin color propio": ahi va el fondo/texto del theme; las
            // celdas con color explicito (headings, code, botones, resaltes) quedan.
            if cell.bg == ratatui::style::Color::Reset {
                cell.set_bg(bg);
            }
            if cell.fg == ratatui::style::Color::Reset {
                cell.set_fg(fg);
            }
        }
    }
}

/// Reduce `area` a una columna centrada horizontalmente de ancho `max_width` (o
/// el ancho disponible si la terminal es mas angosta). Es la "hoja" del modo
/// whitepaper: deja el texto en una columna legible y centrada en vez de pegado
/// al borde izquierdo ocupando todo el ancho. Solo toca x/width; alto y posicion
/// vertical quedan igual.
fn centered_column(area: Rect, max_width: u16) -> Rect {
    if area.width <= max_width {
        return area;
    }
    Rect {
        x: area.x + (area.width - max_width) / 2,
        width: max_width,
        ..area
    }
}

/// Dibuja el editor. Lee/escribe el estado de vista via `state` (scroll y
/// viewport_height se ajustan in situ para mantener el cursor visible).
fn draw(
    frame: &mut ratatui::Frame,
    doc: &Document,
    keymap: &dyn Keymap,
    theme: &Theme,
    wysiwyg_level: u8,
    state: &mut AppState,
    tabs: Option<Line<'static>>,
) {
    // La paleta y el switcher (mutuamente excluyentes) son popups flotantes que se
    // montan ENCIMA del editor: primero se dibuja el editor normal (queda de fondo)
    // y al final el overlay lo atenua y pinta su box centrado. Mientras haya un
    // overlay abierto el editor no debe mostrar su cursor (lo tapa el popup).
    let overlay_active =
        state.palette.is_some() || state.switcher.is_some() || state.theme_picker.is_some();

    // Snapshot de los flags de vista que se leen varias veces aca; evita tener
    // `&state` vivo mientras mas abajo se mutan `state.scroll`/`viewport_height`.
    // Whitepaper es un superset de zen para el chrome (oculta borde/toolbar/status
    // igual); ademas centra el editor en una columna fija (mas abajo).
    let whitepaper = state.whitepaper;
    let zen = state.zen || whitepaper;

    // Zen/focus: ocultamos todo el chrome (toolbar, status) para dejar solo el
    // texto. Excepcion: si hay un overlay de busqueda activo reservamos la ultima
    // linea para el minibuffer (si no, no se veria que se esta buscando). Fuera de
    // zen: editor (resto) + linea separadora + toolbar + status bar. Como el editor
    // ya no lleva marco (chrome minimal), la separadora (una regla tenue) marca el
    // "piso" de la zona de escritura y la despega del menu/estado.
    let (tabs_area, editor_area, sep_area, hints_area, status_area) = if zen {
        // En zen reservamos la ultima linea para el minibuffer cuando hay un overlay
        // de busqueda O un prompt de confirmacion activo: si no, el prompt seria
        // invisible y las teclas "no responderian" sin explicacion.
        if state.overlay.is_some() || state.confirm.is_some() {
            let [editor, mini] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
            (None, editor, None, None, Some(mini))
        } else {
            (None, frame.area(), None, None, None)
        }
    } else if tabs.is_some() {
        // Con tabs (>=2 buffers) reservamos una fila ARRIBA de todo para la barra.
        // Abajo: regla (piso de escritura) + toolbar + gap + status. La regla despega
        // el texto del chrome; el gap despega la toolbar del status.
        let [tabs_a, editor, sep, hints, _gap, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        (Some(tabs_a), editor, Some(sep), Some(hints), Some(status))
    } else {
        let [editor, sep, hints, _gap, status] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        (None, editor, Some(sep), Some(hints), Some(status))
    };

    // Whitepaper: el editor no ocupa todo el ancho sino una columna centrada de
    // ancho fijo (la "hoja"). Como el cursor se calcula a partir de
    // `editor_area.x`, al correr el area a la derecha el cursor la sigue solo. En
    // terminales mas angostas que `WHITEPAPER_WIDTH` queda el ancho completo.
    let editor_area = if whitepaper {
        centered_column(editor_area, WHITEPAPER_WIDTH)
    } else {
        editor_area
    };

    // Barra de tabs (si la hay): la fila reservada arriba.
    if let (Some(area), Some(line)) = (tabs_area, tabs) {
        frame.render_widget(Paragraph::new(line), area);
    }

    // Regla separadora: el "piso" de la zona de escritura. Una linea tenue de ancho
    // completo que despega el texto de la toolbar/status ahora que no hay marco.
    if let Some(area) = sep_area {
        let rule = "─".repeat(area.width as usize);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                rule,
                Style::default().fg(theme.marker),
            ))),
            area,
        );
    }

    // Chrome minimal (estilo editxr): el editor NO lleva marco en ningun modo, para
    // que el texto respire y el foco sea la escritura. `border` queda en 0 siempre;
    // se conserva como constante para el calculo de offsets del cursor/viewport (que
    // lo suman/restan de forma uniforme).
    let border: u16 = 0;
    // Margen izquierdo para que el texto no quede pegado al filo. Sin marco, damos
    // aire (2 celdas) en todos los modos; asi el texto queda en la misma columna que
    // antes (cuando el borde ocupaba 1 + 1 de padding). Suma al offset del cursor.
    let pad_left: u16 = 2;
    // Margen superior, para que el texto no arranque pegado al borde de arriba.
    // En papel son 2 (se siente mas una "hoja"); en el resto 1, una fila de aire
    // (tanto en normal como en zen el texto quedaba pegado arriba). Suma al offset
    // del cursor y resta del alto util (igual que `pad_left` con la columna).
    let pad_top: u16 = if whitepaper { 2 } else { 1 };
    // Alto util dentro del borde del Block y del margen superior.
    let viewport_height = editor_area.height.saturating_sub(2 * border + pad_top) as usize;
    // Lo exponemos al loop para que PageUp/PageDown sepan cuanto mover. Sigue
    // razonando en LINEAS del documento (no en filas visuales post-wrap): es
    // una aproximacion aceptable para paginar, no vale la pena el layout completo
    // solo para eso.
    state.viewport_height = viewport_height.max(1);

    // Coincidencias a resaltar segun el overlay (busqueda incremental o el
    // termino de busqueda del reemplazo). Sin overlay, no hay resaltado. La
    // coincidencia "actual" es la que arranca bajo el cursor (en busqueda el
    // cursor salto justo ahi); en reemplazo normalmente no hay y queda sin marcar.
    let text = doc.text();
    let matches = match state.overlay.as_ref() {
        Some(ov) => ov.highlights(&text),
        None => Vec::new(),
    };
    let current = if matches.is_empty() {
        None
    } else {
        matches.iter().position(|m| m.start == doc.cursor_byte())
    };

    // Chrome minimal: el editor va sin marco ni titulo en todos los modos (solo
    // texto con su margen). El path se ve en la barra de tabs (con >=2 buffers) y en
    // la status bar; no hace falta un titulo de marco. El padding coincide con
    // `pad_left`/`pad_top` (que ya usa el cursor).
    let block = Block::default().padding(Padding::new(pad_left, 0, pad_top, 0));
    // En Nivel 2 la linea con el cursor se renderiza como Nivel 1 (markers
    // visibles) para preservar el mapeo cursor->columna 1:1. Las demas lineas
    // ocultan los delimiters inline (ver `render::render`).
    // Ancho de la caja de codigo: el area util de texto (descontando borde y
    // margen izquierdo del Block) menos el margen derecho. Asi los bloques de
    // codigo se extienden casi de lado a lado pero dentro de un margen, en vez de
    // ajustarse al contenido. Padding derecho del Block es 0, por eso no se resta.
    let text_width = editor_area.width.saturating_sub(2 * border + pad_left) as usize;
    let code_box_width = text_width.saturating_sub(render::CODE_BOX_RIGHT_MARGIN);
    let (lines, no_wrap) = render::render(
        &text,
        doc.selection_byte_range(),
        &matches,
        current,
        theme,
        Some(doc.line),
        wysiwyg_level,
        code_box_width,
    );

    // Layout de soft wrap sobre las lineas pre-wrap que devuelve `render`
    // (una por linea del documento): parte cada linea, salvo las de grilla de
    // tabla (`no_wrap`), en filas visuales segun `text_width`. De aca en mas
    // scroll y cursor razonan en FILAS VISUALES, no en lineas del documento.
    let layout = wrap::layout(&lines, &no_wrap, text_width);

    // Columna visual del cursor sobre su linea renderizada: si esta sobre una
    // linea de bloque de codigo, el render le aplico un margen izquierdo
    // (`CODE_BOX_LEFT_PAD`) que corre el texto a la derecha, asi que lo sumamos
    // antes de mapear a fila/x (la linea activa siempre se renderiza Level 1
    // cruda, asi que el mapeo sigue valiendo).
    let code_indent = if render::code_line_flags(&text)
        .get(doc.line)
        .copied()
        .unwrap_or(false)
    {
        render::CODE_BOX_LEFT_PAD
    } else {
        0
    };
    let (cursor_row, x_in_row) = layout.row_and_x(doc.line, code_indent + doc.display_col());

    // Clamp defensivo: tras ediciones que achican el documento, el scroll
    // viejo puede haber quedado mas alla del final del layout nuevo.
    state.scroll = state.scroll.min(layout.total_rows().saturating_sub(1));
    // Ajustar scroll para que el cursor quede dentro del viewport (en filas
    // visuales: una linea envuelta puede ocupar mas de una).
    if viewport_height > 0 {
        if cursor_row < state.scroll {
            state.scroll = cursor_row;
        } else if cursor_row >= state.scroll + viewport_height {
            state.scroll = cursor_row + 1 - viewport_height;
        }
    }
    // Tras reclampar, lo leemos en un local: simplifica los usos de abajo y
    // evita re-tomar `&state` mientras se sigue dibujando.
    let scroll = state.scroll;

    let paragraph = Paragraph::new(wrap::visual_lines(lines, &layout))
        .block(block)
        .scroll((scroll as u16, 0));
    frame.render_widget(paragraph, editor_area);

    // Barra de atajos (toolbar estilo WordStar/Norton Commander): los atajos del
    // preset para el modo actual, reflejando los remapeos del usuario. En zen se
    // oculta (hints_area = None).
    if let Some(hints_area) = hints_area {
        frame.render_widget(
            hints_bar(keymap, doc.mode, &state.pending, theme),
            hints_area,
        );
    }

    // Status bar (o, con overlay abierto, el minibuffer; o un mensaje flash
    // transitorio, ej un error de guardado, que tiene prioridad sobre la status
    // bar normal). En zen sin overlay/flash no hay area y no se dibuja nada.
    if let Some(status_area) = status_area {
        if let Some(confirm) = state.confirm.as_ref() {
            // El prompt de confirmacion es modal: tiene prioridad sobre minibuffer,
            // flash y status bar. Mismo estilo invertido que el flash.
            let prompt = match confirm {
                Confirm::CloseBuffer => i18n::t(i18n::Key::ConfirmCloseUnsaved),
            };
            let style = Style::default().add_modifier(Modifier::REVERSED);
            frame.render_widget(
                Line::from(Span::styled(format!(" {prompt} "), style)),
                status_area,
            );
        } else if let Some(ov) = state.overlay.as_ref() {
            frame.render_widget(ov.minibuffer(), status_area);
        } else if let Some(msg) = state.flash.as_deref() {
            let style = Style::default().add_modifier(Modifier::REVERSED);
            frame.render_widget(
                Line::from(Span::styled(format!(" {msg} "), style)),
                status_area,
            );
        } else {
            frame.render_widget(status_bar(doc, keymap, &state.pending), status_area);
        }
    }

    // Cursor: sumando el margen (`border` es 0 con el chrome minimal, pero se deja
    // en la formula por uniformidad) mas `pad_left`/`pad_top`, y restando scroll
    // (ya en filas visuales). `x_in_row` es la columna *visual* dentro de la fila
    // (celdas, no indice de char), resuelta por `layout.row_and_x` mas arriba: ya
    // incluye `code_indent`, no sumarlo de nuevo. Con un overlay abierto no se
    // dibuja cursor de editor: lo tapa el popup y ratatui lo oculta.
    if !overlay_active && cursor_row >= scroll && cursor_row < scroll + viewport_height {
        let cursor_x = editor_area.x + border + pad_left + x_in_row as u16;
        let cursor_y = editor_area.y + border + pad_top + (cursor_row - scroll) as u16;
        if whitepaper {
            // En papel el cursor real del terminal usa un color fijo que sobre el
            // fondo claro suele quedar invisible. En vez de depender de el,
            // dibujamos un cursor sintetico: marcamos la celda bajo el cursor como
            // REVERSED. El post-pass `apply_theme_fill` le pone tinta/papel en sus
            // canales Reset y el REVERSED los intercambia, dejando un bloque de
            // tinta sobre el papel. No posicionamos el cursor del terminal (queda
            // oculto al no llamar a `set_cursor_position`) para no duplicarlo.
            if editor_area.contains(Position::new(cursor_x, cursor_y)) {
                frame.buffer_mut()[(cursor_x, cursor_y)]
                    .set_style(Style::default().add_modifier(Modifier::REVERSED));
            }
        } else {
            frame.set_cursor_position(Position::new(cursor_x, cursor_y));
        }
    }

    // Overlays flotantes (paleta / switcher / theme picker): se montan al FINAL
    // para quedar por encima del editor ya dibujado, atenuandolo de fondo.
    // Mutuamente excluyentes.
    if let Some(pal) = state.palette.as_ref() {
        pal.render(frame, theme);
    } else if let Some(sw) = state.switcher.as_ref() {
        sw.render(frame, theme);
    } else if let Some(tp) = state.theme_picker.as_ref() {
        tp.render(frame, theme);
    }
}

/// Construye la barra de atajos: cada hint se dibuja como un "boton" con fondo
/// propio (` tecla label `), con la tecla en acento+negrita y el label en el
/// texto normal sobre el mismo fondo; entre botones va un espacio SIN fondo para
/// separarlos.
///
/// Es dinamica: si hay un chord en curso (`pending` no vacio) y el preset tiene
/// continuaciones para ese prefijo, la barra muestra ese subconjunto, precedido
/// por el prefijo (ej `^P ▸`). Si no, muestra los atajos top-level del modo. Los
/// keybindings remapeados ya vienen reflejados en `keymap.hints`. Si no entran
/// todos, ratatui los trunca al ancho.
fn hints_bar(
    keymap: &dyn Keymap,
    mode: Mode,
    pending: &[KeyEvent],
    theme: &Theme,
) -> Line<'static> {
    // El boton entero comparte el fondo; la tecla ademas lleva acento y negrita.
    let button = Style::default().bg(theme.toolbar_button_bg);
    let key_style = button.fg(theme.heading_2).add_modifier(Modifier::BOLD);

    // Chord en curso con continuaciones conocidas -> mostrar ese subconjunto.
    let chord = if pending.is_empty() {
        Vec::new()
    } else {
        keymap.chord_hints(mode, pending)
    };
    let in_chord = !chord.is_empty();
    let hints = if in_chord { chord } else { keymap.hints(mode) };

    let mut spans: Vec<Span<'static>> = Vec::new();
    // En un chord, anteponer el prefijo (ej `^P ▸`) como contexto.
    if in_chord && let Some(prefix) = chord_indicator(pending) {
        spans.push(Span::styled(
            format!(" {prefix} ▸"),
            Style::default()
                .fg(theme.heading_1)
                .add_modifier(Modifier::BOLD),
        ));
    }
    for (i, hint) in hints.into_iter().enumerate() {
        // Separacion entre botones (sin fondo, deja ver el fondo del editor). El
        // primer boton sin chord lleva solo un margen de 1; con chord va tras el
        // prefijo, asi que tambien separa con 2.
        spans.push(Span::raw(if i == 0 && !in_chord { " " } else { "  " }));
        // Boton: padding + tecla (acento) + gap + label, todo sobre el fondo.
        spans.push(Span::styled(" ", button));
        spans.push(Span::styled(hint.keys, key_style));
        spans.push(Span::styled(format!(" {} ", hint.label), button));
    }
    Line::from(spans)
}

/// Construye la barra de estado: preset, modo (solo si es modal), path, dirty,
/// chord en curso y linea:col.
fn status_bar(doc: &Document, keymap: &dyn Keymap, pending: &[KeyEvent]) -> Line<'static> {
    // El modo solo tiene sentido en presets modales (Vim); en modeless no se
    // muestra "NORMAL/INSERT" porque no existen.
    let left = if keymap.is_modal() {
        let mode = match doc.mode {
            Mode::Normal => i18n::t(i18n::Key::ModeNormal),
            Mode::Insert => i18n::t(i18n::Key::ModeInsert),
            Mode::Visual => i18n::t(i18n::Key::ModeVisual),
        };
        format!(" {} · {} · {} ", keymap.name(), mode, doc.path.display())
    } else {
        // En modeless no hay modo; si hay una seleccion activa lo indicamos con
        // un sufijo SEL (en Vim eso ya lo cubre VISUAL).
        let sel = if doc.selection_range().is_some() {
            " · SEL"
        } else {
            ""
        };
        format!(" {} · {}{} ", keymap.name(), doc.path.display(), sel)
    };
    // Marca de "no a salvo en disco": cambios sin guardar O untitled/nunca
    // guardado. Mismo criterio que el switcher (ver `Document::unsaved`).
    let unsaved = if doc.unsaved() { "[+] " } else { "" };
    let left = format!("{}{}", left, unsaved);
    // Contador de palabras: con seleccion activa muestra "seleccionadas/total".
    let words = match doc.selection_word_count() {
        Some(sel) => i18n::words_count_selection(sel, doc.word_count()),
        None => i18n::words_count(doc.word_count()),
    };
    let right = format!(" {} · {}:{} ", words, doc.line + 1, doc.display_col() + 1);
    // Margen de 1 espacio SIN fondo antes del pill, para que el pill del status
    // arranque en la misma columna que el primer boton de la toolbar (que tambien
    // lleva 1 espacio de margen). Asi los bordes izquierdos quedan alineados.
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(left, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(" "),
    ];
    // Indicador de chord en curso (prefijo esperando la proxima tecla), tipo
    // "^K" / "^Q", para que el usuario sepa que esta en medio de una secuencia.
    if let Some(chord) = chord_indicator(pending) {
        spans.push(Span::styled(
            format!(" {} ", chord),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(right));
    Line::from(spans)
}

/// Representa el chord pendiente como texto (ej `^K`), o `None` si no hay
/// teclas pendientes. Se queda con la primer tecla del prefijo, que es la que
/// identifica el chord.
fn chord_indicator(pending: &[KeyEvent]) -> Option<String> {
    let first = pending.first()?;
    match first.code {
        KeyCode::Char(c) if first.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(format!("^{}", c.to_ascii_uppercase()))
        }
        _ => None,
    }
}

/// Aplica una accion semantica sobre el documento. Devuelve `Ok(true)` si hay
/// que salir del editor (Quit). `viewport_height` se usa para acciones que
/// dependen del alto visible (Page Up/Down); el resto de las acciones lo
/// ignora.
fn apply_action(
    doc: &mut Document,
    action: Action,
    viewport_height: usize,
) -> std::io::Result<bool> {
    match action {
        Action::CursorLeft => doc.move_left(),
        Action::CursorRight => doc.move_right(),
        Action::CursorUp => doc.move_up(),
        Action::CursorDown => doc.move_down(),
        Action::InsertChar(c) => doc.insert_char(c),
        Action::InsertNewline => doc.insert_newline(),
        Action::Backspace => doc.backspace(),
        Action::DeleteChar => doc.delete_char(),
        Action::EnterInsert => doc.mode = Mode::Insert,
        Action::EnterNormal => doc.mode = Mode::Normal,
        Action::InsertAfter => {
            doc.move_right_for_append();
            doc.mode = Mode::Insert;
        }
        Action::OpenLineBelow => {
            doc.open_line_below();
            doc.mode = Mode::Insert;
        }
        Action::LineStart => doc.move_to_line_start(),
        Action::LineEnd => doc.move_to_line_end(),
        Action::DocStart => doc.move_to_doc_start(),
        Action::DocEnd => doc.move_to_doc_end(),
        Action::PageUp => doc.move_page_up(viewport_height),
        Action::PageDown => doc.move_page_down(viewport_height),
        Action::Save => doc.save()?,
        Action::SaveAndQuit => {
            doc.save()?;
            return Ok(true);
        }
        Action::Quit => return Ok(true),
        Action::ToggleBold => toggle_inline_action(doc, InlineKind::Bold),
        Action::ToggleItalic => toggle_inline_action(doc, InlineKind::Italic),
        Action::ToggleCode => toggle_inline_action(doc, InlineKind::Code),
        Action::EnterVisual => {
            doc.mode = Mode::Visual;
            doc.start_selection();
        }
        Action::SelectLeft => doc.extend_left(),
        Action::SelectRight => doc.extend_right(),
        Action::SelectUp => doc.extend_up(),
        Action::SelectDown => doc.extend_down(),
        Action::DeleteSelection => {
            doc.delete_selection();
            // Si veniamos de Visual (Vim), la seleccion se consumio: volver a
            // Normal. En modeless no hay Visual, asi que esto no aplica.
            if doc.mode == Mode::Visual {
                doc.mode = Mode::Normal;
            }
        }
        Action::Undo => doc.undo(),
        Action::Redo => doc.redo(),
        Action::Yank => {
            doc.yank();
            // Si veniamos de Visual (Vim), la seleccion se consumio al copiar:
            // volver a Normal. En modeless no hay Visual, asi que no aplica.
            if doc.mode == Mode::Visual {
                doc.mode = Mode::Normal;
            }
        }
        Action::Paste => doc.paste(),
        // Search/Replace abren un overlay, ToggleZen togglea la vista,
        // OpenSwitcher abre el switcher y OpenPalette la paleta de comandos (todos
        // a nivel del loop); los intercepta `dispatch_action` antes de llegar aca.
        // Se listan para que el match siga exhaustivo.
        Action::Search
        | Action::Replace
        | Action::ToggleZen
        | Action::ToggleWhitepaper
        | Action::ToggleLightTheme
        | Action::ExportHtml
        | Action::ExportPdf
        | Action::NewBuffer
        | Action::NextBuffer
        | Action::PrevBuffer
        | Action::CloseBuffer
        | Action::OpenSwitcher
        | Action::OpenPalette
        | Action::OpenThemePicker
        | Action::CycleKeymapPreset => {}
    }
    Ok(false)
}

/// Aplica un toggle de estilo inline. Si veniamos del modo Visual de Vim, la
/// seleccion se consume con el toggle y volvemos a Normal.
fn toggle_inline_action(doc: &mut Document, kind: InlineKind) {
    doc.toggle_inline(kind);
    if doc.mode == Mode::Visual {
        doc.mode = Mode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use typebar_core::document::test_support::doc_with;

    /// Renderiza un frame con `draw` sobre un backend de prueba y devuelve todo el
    /// buffer como texto plano (filas separadas por `\n`). Sirve para verificar
    /// que cierto chrome aparece o no en pantalla.
    fn render_to_string(zen: bool, switcher: Option<Switcher>, palette: Option<Palette>) -> String {
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let doc = doc_with("hola mundo");
        let km = StandardKeymap;
        let theme = Theme::frappe();
        // 60x24: alto realista de terminal, con lugar para el chrome del popup
        // (borde + padding + prompt + footer) y varias filas de resultados.
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut state = AppState::new(false);
        state.zen = zen;
        state.switcher = switcher;
        state.palette = palette;
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let mut out = String::new();
        for y in 0..area.height {
            for x in 0..area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    #[test]
    fn draw_normal_muestra_chrome() {
        // Fuera de zen (chrome minimal, sin marco): la toolbar (`Save`, locale En por
        // default en tests) y el texto estan presentes. No hay borde: no aparece la
        // esquina superior del marco.
        let screen = render_to_string(false, None, None);
        assert!(screen.contains("Save"), "falta la toolbar");
        assert!(screen.contains("hola mundo"), "falta el texto");
        assert!(
            !screen.contains('┌') && !screen.contains('╭'),
            "no deberia haber marco alrededor del editor"
        );
        // La regla separadora (piso de la zona de escritura) esta presente.
        assert!(
            screen.contains('─'),
            "falta la regla que separa la escritura del menu/status"
        );
    }

    #[test]
    fn draw_zen_oculta_chrome_pero_muestra_texto() {
        // En zen: sin toolbar; solo el texto.
        let screen = render_to_string(true, None, None);
        assert!(
            !screen.contains("Save"),
            "la toolbar no deberia verse en zen"
        );
        assert!(
            !screen.contains('─'),
            "en zen no hay regla separadora (chrome oculto)"
        );
        assert!(
            screen.contains("hola mundo"),
            "el texto deberia seguir visible"
        );
    }

    /// Como `render_to_string` pero con whitepaper activo y un ancho de terminal
    /// dado, devuelve cada fila del buffer (para inspeccionar el centrado). El
    /// ancho lo controla el test porque el centrado depende de el.
    fn render_whitepaper(term_width: u16) -> Vec<String> {
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let doc = doc_with("hola mundo");
        let km = StandardKeymap;
        let theme = Theme::latte();
        let mut terminal = Terminal::new(TestBackend::new(term_width, 8)).unwrap();
        let mut state = AppState::new(false);
        state.whitepaper = true;
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect()
    }

    #[test]
    fn draw_whitepaper_oculta_chrome_y_centra_el_texto() {
        // Whitepaper es superset de zen: sin titulo ni toolbar. Y en una terminal
        // ancha (120 > WHITEPAPER_WIDTH) el texto queda centrado, es decir con
        // varios espacios de sangria a la izquierda (no pegado al borde).
        let rows = render_whitepaper(120);
        let screen = rows.join("\n");
        assert!(!screen.contains("typebar"), "no deberia verse el titulo");
        assert!(!screen.contains("Save"), "no deberia verse la toolbar");
        // Margen superior: el texto no arranca en la fila 0 (hay aire arriba).
        let fila_texto = rows
            .iter()
            .position(|r| r.contains("hola mundo"))
            .expect("el texto deberia seguir visible");
        assert!(
            fila_texto >= 2,
            "el texto deberia tener margen arriba (fila {fila_texto})"
        );
        let texto = rows
            .iter()
            .find(|r| r.contains("hola mundo"))
            .expect("el texto deberia seguir visible");
        let sangria = texto.len() - texto.trim_start().len();
        // Centro esperado: (120 - 72)/2 = 24, mas el margen izquierdo del bloque.
        assert!(
            sangria > 20,
            "el texto deberia estar centrado (sangria {sangria}): {texto:?}"
        );
    }

    #[test]
    fn draw_whitepaper_dibuja_cursor_sintetico_visible() {
        // En papel no usamos el cursor del terminal (invisible sobre fondo claro):
        // la celda bajo el cursor queda marcada REVERSED, y tras apply_theme_fill
        // termina como un bloque de tinta sobre papel. Ademas NO se posiciona el
        // cursor real del terminal.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let doc = doc_with("hola mundo"); // cursor en 0:0 -> primera celda de texto
        let km = StandardKeymap;
        let theme = Theme::paper();
        let mut terminal = Terminal::new(TestBackend::new(120, 8)).unwrap();
        let mut state = AppState::new(false);
        state.whitepaper = true;
        terminal
            .draw(|f| {
                draw(f, &doc, &km, &theme, 2, &mut state, None);
                apply_theme_fill(f, &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // Buscar la celda con la 'h' de "hola" (la primera del texto): debe estar
        // en REVERSED y con colores concretos (tinta/papel), nunca Reset.
        let area = *buf.area();
        let mut found = false;
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = &buf[(x, y)];
                if cell.symbol() == "h" {
                    assert!(
                        cell.modifier.contains(Modifier::REVERSED),
                        "la celda del cursor deberia estar REVERSED"
                    );
                    assert_eq!(cell.bg, theme.background.unwrap());
                    assert_eq!(cell.fg, theme.text.unwrap());
                    found = true;
                }
            }
        }
        assert!(found, "deberia encontrarse la celda del cursor (la 'h')");
    }

    #[test]
    fn draw_envuelve_una_linea_mas_larga_que_el_viewport_en_dos_filas() {
        // El bug real (typebar sin wrap ni scroll horizontal): una linea mas
        // larga que el viewport se recortaba y el sobrante quedaba invisible.
        // Con el soft wrap integrado, el sobrante baja a la fila visual
        // siguiente en vez de perderse.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        // Terminal de 60 columnas, sin marco, pad_left=2 => text_width = 58.
        // 58 'a' (sin espacios, para forzar corte duro exacto en el ancho) mas
        // "OVERFLOW": no entra en una sola fila.
        let long_line = format!("{}OVERFLOW", "a".repeat(58));
        let doc = doc_with(&long_line);
        let km = StandardKeymap;
        let theme = Theme::frappe();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut state = AppState::new(false);
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();
        // Fila visual 0 (pad_top=1 => pantalla y=1): las 58 'a'.
        assert!(
            rows[1].contains(&"a".repeat(58)),
            "la primera fila visual deberia mostrar las 58 'a': {:?}",
            rows[1]
        );
        assert!(
            !rows[1].contains("OVERFLOW"),
            "el sobrante NO deberia verse todavia en la primera fila: {:?}",
            rows[1]
        );
        // Fila visual 1 (pantalla y=2): el sobrante, visible (no recortado).
        assert!(
            rows[2].contains("OVERFLOW"),
            "el sobrante deberia bajar visible a la fila visual siguiente: {:?}",
            rows[2]
        );
    }

    #[test]
    fn draw_con_cursor_al_final_de_una_linea_larga_lo_ubica_en_la_segunda_fila() {
        // Con el cursor al FINAL de una linea que envuelve, el cursor debe
        // quedar posicionado en la SEGUNDA fila visual (donde cae realmente
        // el glifo), no en la primera fila del documento como antes de
        // integrar el wrap. Igual que `draw_whitepaper_dibuja_cursor_sintetico_visible`,
        // usamos whitepaper para poder assertar la celda REVERSED sin
        // depender del cursor real del terminal.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        // Whitepaper con terminal de 120 columnas: columna centrada de ancho
        // WHITEPAPER_WIDTH=72, pad_left=2 => text_width = 70. 70 'a' + 'Z'
        // (71 chars, sin espacios): la 'Z' no entra en la primera fila y cae
        // sola en la segunda.
        let long_line = format!("{}Z", "a".repeat(70));
        let mut doc = doc_with(&long_line);
        doc.line = 0;
        doc.col = 70; // cursor sobre la 'Z' (justo al final de la linea)
        let km = StandardKeymap;
        let theme = Theme::paper();
        let mut terminal = Terminal::new(TestBackend::new(120, 8)).unwrap();
        let mut state = AppState::new(false);
        state.whitepaper = true;
        terminal
            .draw(|f| {
                draw(f, &doc, &km, &theme, 2, &mut state, None);
                apply_theme_fill(f, &theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        // La 'Z' (unico caracter de la segunda fila visual) debe llevar el
        // cursor sintetico REVERSED.
        let mut found = false;
        let mut z_row = None;
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = &buf[(x, y)];
                if cell.symbol() == "Z" {
                    assert!(
                        cell.modifier.contains(Modifier::REVERSED),
                        "la celda de la 'Z' (cursor) deberia estar REVERSED"
                    );
                    z_row = Some(y);
                    found = true;
                }
            }
        }
        assert!(found, "deberia encontrarse la celda de la 'Z'");
        // La fila de la 'Z' es la SEGUNDA fila visual: distinta (posterior) a
        // la fila donde arrancan las 70 'a'.
        let a_row = (0..area.height)
            .find(|&y| (0..area.width).any(|x| buf[(x, y)].symbol() == "a"))
            .expect("deberia encontrarse alguna 'a' de la primera fila visual");
        assert!(
            z_row.unwrap() > a_row,
            "la 'Z' deberia quedar en una fila visual posterior a la de las 'a' (a_row={a_row}, z_row={z_row:?})"
        );
    }

    #[test]
    fn draw_seleccion_que_cruza_el_corte_de_wrap_pinta_ambas_filas() {
        // Edge case del wrap: una seleccion que abarca el limite entre dos
        // filas visuales debe pintar el bg de seleccion en AMBAS, no solo en
        // la fila donde arranca. Misma geometria que el smoke test de wrap:
        // terminal 60x24, pad_left=2 => text_width=58, "a"*58 + "OVERFLOW" se
        // envuelve en fila 0 = "a"*58 (chars [0,58)) y fila 1 = "OVERFLOW"
        // (chars [58,66)).
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        let long_line = format!("{}OVERFLOW", "a".repeat(58));
        let mut doc = doc_with(&long_line);
        // Seleccion [50,62): cruza el limite en 58 (col 50..58 en fila 0,
        // col 58..62 en fila 1). ASCII puro: char idx == byte idx.
        doc.line = 0;
        doc.col = 50;
        doc.start_selection();
        doc.col = 62;
        let km = StandardKeymap;
        let theme = Theme::frappe();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut state = AppState::new(false);
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        // Fila 0 (pantalla y=1): char 49 ('a', fuera de la seleccion, x=51) sin
        // bg de seleccion; char 50 (primer char seleccionado, x=52) y char 57
        // (ultimo char seleccionado de la fila, x=59) SI.
        assert_ne!(
            buf[(51, 1)].bg,
            theme.selection_bg,
            "char antes de la seleccion no deberia estar resaltado"
        );
        assert_eq!(
            buf[(52, 1)].bg,
            theme.selection_bg,
            "primer char seleccionado de la fila 0 deberia estar resaltado"
        );
        assert_eq!(
            buf[(59, 1)].bg,
            theme.selection_bg,
            "ultimo char seleccionado de la fila 0 (justo antes del corte) deberia estar resaltado"
        );

        // Fila 1 (pantalla y=2): char 58 ('O', x=2) y char 61 ('R', x=5) SI
        // resaltados (siguen en la seleccion tras el corte); char 62 ('F',
        // x=6, fuera de la seleccion) no.
        assert_eq!(
            buf[(2, 2)].bg,
            theme.selection_bg,
            "el sobrante de la seleccion deberia seguir resaltado en la fila 1"
        );
        assert_eq!(
            buf[(5, 2)].bg,
            theme.selection_bg,
            "ultimo char seleccionado de la fila 1 deberia estar resaltado"
        );
        assert_ne!(
            buf[(6, 2)].bg,
            theme.selection_bg,
            "char justo despues de la seleccion en la fila 1 no deberia estar resaltado"
        );
    }

    #[test]
    fn draw_tabla_mas_ancha_que_el_viewport_se_clipea_sin_desbordar_a_la_fila_siguiente() {
        // Una tabla mas ancha que el viewport se dibuja como grilla (no_wrap)
        // y ratatui la CLIPEA a lo ancho del area, sin envolverla: el sobrante
        // no debe aparecer en la fila visual siguiente (que pertenece a otra
        // linea de la grilla, ej la fila delimitadora).
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        // Celda de 80 'a': con el padding/bordes de la grilla la fila mide
        // bastante mas que el text_width (58) de una terminal de 60 columnas.
        let long_cell = "a".repeat(80);
        let source = format!("cursor\n| {long_cell} |\n| --- |\n| x |\n");
        let doc = doc_with(&source);
        let km = StandardKeymap;
        let theme = Theme::frappe();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut state = AppState::new(false);
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let rows: Vec<String> = (0..area.height)
            .map(|y| (0..area.width).map(|x| buf[(x, y)].symbol()).collect())
            .collect();

        // Fila visual 1 (pantalla y=2): el header de la tabla, clipeado: no
        // deberian verse las 80 'a' completas.
        assert!(
            !rows[2].contains(&"a".repeat(80)),
            "la fila de header deberia estar clipeada, no mostrar las 80 'a': {:?}",
            rows[2]
        );
        assert!(
            rows[2].contains('a'),
            "algo del contenido deberia seguir visible antes del clip: {:?}",
            rows[2]
        );
        // Fila visual siguiente (pantalla y=3, la delimitadora de la grilla):
        // el sobrante de la fila anterior NO debe filtrarse aca.
        assert!(
            !rows[3].contains('a'),
            "el sobrante de la tabla no deberia aparecer en la fila siguiente: {:?}",
            rows[3]
        );
    }

    #[test]
    fn draw_whitepaper_envuelve_dentro_de_la_columna_centrada() {
        // En whitepaper (terminal ancha, columna centrada de WHITEPAPER_WIDTH)
        // una linea que envuelve debe seguir arrancando su segunda fila en el
        // MISMO x que la primera (la columna centrada), no pegada al borde
        // izquierdo del terminal.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        // Terminal de 120 columnas: columna centrada de ancho WHITEPAPER_WIDTH=72,
        // pad_left=2 => text_width=70. 70 'a' + "OVERFLOW" (sin espacios): la
        // primera fila son las 70 'a', la segunda "OVERFLOW".
        let long_line = format!("{}OVERFLOW", "a".repeat(70));
        let doc = doc_with(&long_line);
        let km = StandardKeymap;
        let theme = Theme::latte();
        let mut terminal = Terminal::new(TestBackend::new(120, 8)).unwrap();
        let mut state = AppState::new(false);
        state.whitepaper = true;
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();

        // x esperado de la columna centrada: (120-72)/2 = 24, mas pad_left=2 => 26.
        let expected_x = 26u16;
        assert_eq!(
            buf[(expected_x, 2)].symbol(),
            "a",
            "la primera fila deberia arrancar en la columna centrada (x={expected_x})"
        );
        assert_eq!(
            buf[(expected_x, 3)].symbol(),
            "O",
            "la segunda fila (el sobrante 'OVERFLOW') deberia arrancar EN EL MISMO x que la primera, no pegada al borde"
        );
        // Y no deberia estar pegada al borde izquierdo del terminal (x=0).
        assert_ne!(
            buf[(0, 3)].symbol(),
            "O",
            "la continuacion no deberia arrancar pegada al borde del terminal"
        );
    }

    #[test]
    fn draw_scroll_en_filas_visuales_mantiene_visible_la_fila_del_cursor() {
        // Doc de una sola linea (corto en LINEAS) pero larga en caracteres, que
        // genera mas filas visuales que el viewport. Con el cursor al final del
        // doc, el scroll debe adelantarse en FILAS VISUALES para mantenerlo
        // visible: el texto del final aparece, el del principio scrollea afuera.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use typebar_core::document::test_support::doc_with;

        // Terminal 60x24: text_width=58, viewport_height=19 filas. Sin espacios
        // (palabra unica): corte duro cada 58 celdas. 5+1200+3=1208 chars =>
        // ceil(1208/58)=21 filas visuales, bastante mas que el viewport (19).
        let content = format!("START{}END", "x".repeat(1200));
        let mut doc = doc_with(&content);
        doc.line = 0;
        doc.col = content.chars().count() - 1; // cursor sobre la ultima 'D' de "END"
        let km = StandardKeymap;
        let theme = Theme::frappe();
        let mut terminal = Terminal::new(TestBackend::new(60, 24)).unwrap();
        let mut state = AppState::new(false);
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let screen: String = (0..area.height)
            .map(|y| {
                (0..area.width)
                    .map(|x| buf[(x, y)].symbol())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert!(
            !screen.contains("START"),
            "el principio del doc deberia haber scrolleado fuera de vista"
        );
        assert!(
            screen.contains("END"),
            "el final del doc (donde esta el cursor) deberia seguir visible"
        );
    }

    #[test]
    fn wrap_unicode_cjk_no_parte_ni_duplica_caracteres_entre_filas() {
        // Linea larga de caracteres CJK (ancho 2 cada uno): ninguna fila
        // visual debe exceder el ancho del viewport, y la concatenacion de
        // todas las filas debe reconstruir el texto original exacto (sin
        // caracteres partidos ni duplicados en el corte).
        let theme = Theme::frappe();
        let text_width = 58;
        let original = "中".repeat(40); // 40 * ancho 2 = 80 celdas
        let (lines, no_wrap) = render::render(&original, None, &[], None, &theme, Some(0), 1, 0);
        let layout = wrap::layout(&lines, &no_wrap, text_width);
        let rows = wrap::visual_lines(lines, &layout);

        assert!(rows.len() > 1, "80 celdas con ancho 58 deberia envolver");
        for row in &rows {
            let row_text: String = row.spans.iter().map(|s| s.content.as_ref()).collect();
            let w = typebar_core::text::display_width(&row_text);
            assert!(
                w <= text_width,
                "ninguna fila deberia exceder el ancho del viewport (fila {row_text:?} mide {w})"
            );
        }
        // Reconstruir el texto de todas las filas: exactamente el original,
        // sin perdidas ni duplicados en los cortes.
        let joined: String = rows
            .iter()
            .flat_map(|l| l.spans.iter().map(|s| s.content.as_ref()))
            .collect();
        assert_eq!(joined, original);
    }

    #[test]
    fn wrap_nivel2_envuelve_segun_el_texto_contraido_no_el_crudo() {
        // Linea INACTIVA con markers ocultos (negrita larga): el wrap debe
        // operar sobre el texto YA CONTRAIDO (Nivel 2, sin los `**`), no sobre
        // el crudo. 56 'a' + los 4 bytes de `**...**` (60 crudos) no entran en
        // un ancho de 58 si se envolviera el crudo, pero los 56 contraidos SI
        // entran en una sola fila.
        let theme = Theme::frappe();
        let text_width = 58;
        let raw = format!("**{}**", "a".repeat(56));
        assert!(
            typebar_core::text::display_width(&raw) > text_width,
            "el crudo (con marcadores) debe exceder el ancho para que el caso sea significativo"
        );
        // Una unica linea, `active_line: None` => en Nivel 2 se contrae (ver
        // `nivel2_oculta_aunque_active_line_sea_none` en render.rs).
        let (lines, no_wrap) = render::render(&raw, None, &[], None, &theme, None, 2, 0);
        assert_eq!(lines.len(), 1);
        let layout = wrap::layout(&lines, &no_wrap, text_width);
        assert_eq!(
            layout.total_rows(),
            1,
            "el texto contraido (56 'a') entra en una sola fila de ancho 58"
        );
        let rows = wrap::visual_lines(lines, &layout);
        let text: String = rows[0].spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(
            text,
            "a".repeat(56),
            "sin asteriscos: se envolvio el texto ya contraido"
        );
    }

    #[test]
    fn draw_switcher_monta_popup_sobre_el_editor_atenuado() {
        // Con el switcher abierto: se ve el prompt (locale En por default), los
        // candidatos, el borde redondeado del box y el footer de atajos. El editor
        // de fondo NO se borra: queda visible (atenuado) detras del popup.
        let sw = Switcher::new(
            vec![
                std::path::PathBuf::from("src/main.rs"),
                std::path::PathBuf::from("README.md"),
            ],
            vec![false, false],
        );
        let screen = render_to_string(false, Some(sw), None);
        assert!(
            screen.contains("go to file:"),
            "falta el prompt del switcher"
        );
        assert!(screen.contains("main.rs"), "falta un candidato");
        assert!(screen.contains("README.md"), "falta un candidato");
        assert!(screen.contains('╭'), "falta el borde redondeado del box");
        assert!(screen.contains("Esc"), "falta el footer de atajos");
        assert!(
            screen.contains("hola mundo"),
            "el documento de fondo deberia seguir visible (atenuado) detras del popup"
        );
    }

    #[test]
    fn draw_palette_monta_popup_sobre_el_editor_atenuado() {
        // Con la paleta abierta: se ve el prompt (locale En por default), algun
        // comando, el borde redondeado y el footer. El editor de fondo NO se borra:
        // queda visible (atenuado) detras del popup.
        let km = keybinding::StandardKeymap;
        let pal = Palette::new(&km);
        let screen = render_to_string(false, None, Some(pal));
        assert!(screen.contains("command:"), "falta el prompt de la paleta");
        assert!(screen.contains("Save"), "falta algun comando");
        assert!(screen.contains('╭'), "falta el borde redondeado del box");
        assert!(screen.contains("Esc"), "falta el footer de atajos");
        assert!(
            screen.contains("hola mundo"),
            "el documento de fondo deberia seguir visible (atenuado) detras del popup"
        );
    }

    #[test]
    fn draw_overlay_atenua_el_fondo_pero_no_el_popup() {
        // El punto clave del pulido: el editor detras del popup queda ATENUADO
        // (Modifier::DIM), no borrado. La 'h' de "hola mundo" (fila 1, col 2, fuera
        // del popup centrado) debe llevar DIM; una celda de adentro del box, no.
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let doc = doc_with("hola mundo");
        let km = StandardKeymap;
        let theme = Theme::frappe();
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
        let mut state = AppState::new(false);
        state.palette = Some(Palette::new(&km));
        terminal
            .draw(|f| draw(f, &doc, &km, &theme, 2, &mut state, None))
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        // 'h' del documento, fuera del popup (que arranca en x=9, y=2): atenuada.
        let h = &buf[(2, 1)];
        assert_eq!(
            h.symbol(),
            "h",
            "deberia verse la 'h' del documento de fondo"
        );
        assert!(
            h.modifier.contains(Modifier::DIM),
            "el documento de fondo deberia quedar atenuado"
        );
        // Centro del popup (col 30, fila 6): dentro del box, sin DIM (nitido).
        assert!(
            !buf[(30, 6)].modifier.contains(Modifier::DIM),
            "el interior del popup no deberia estar atenuado"
        );
    }

    /// Renderiza el editor con `theme`, corre `apply_theme_fill`, y devuelve por
    /// cada celda del buffer: si hubo alguna con el fondo del theme, alguna con su
    /// texto, y si quedo ALGUNA celda con fondo `Reset` (sin pintar).
    fn fill_report(theme: &Theme) -> (bool, bool, bool) {
        use keybinding::StandardKeymap;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::style::Color;

        let doc = doc_with("hola");
        let km = StandardKeymap;
        let mut terminal = Terminal::new(TestBackend::new(40, 8)).unwrap();
        let mut state = AppState::new(false);
        terminal
            .draw(|f| {
                draw(f, &doc, &km, theme, 2, &mut state, None);
                apply_theme_fill(f, theme);
            })
            .unwrap();
        let buf = terminal.backend().buffer().clone();
        let area = *buf.area();
        let (mut has_bg, mut has_fg, mut has_reset_bg) = (false, false, false);
        for y in 0..area.height {
            for x in 0..area.width {
                let cell = &buf[(x, y)];
                if Some(cell.bg) == theme.background {
                    has_bg = true;
                }
                if Some(cell.fg) == theme.text {
                    has_fg = true;
                }
                if cell.bg == Color::Reset {
                    has_reset_bg = true;
                }
            }
        }
        (has_bg, has_fg, has_reset_bg)
    }

    #[test]
    fn paperwhite_pinta_fondo_y_texto_en_theme_claro() {
        // Latte (claro) tiene background/text: el post-pass pinta cada celda, asi
        // queda fondo claro y texto oscuro, y NINGUNA celda en Reset.
        let (has_bg, has_fg, has_reset_bg) = fill_report(&Theme::latte());
        assert!(has_bg, "el theme claro deberia pintar el fondo");
        assert!(has_fg, "el theme claro deberia pintar el texto");
        assert!(
            !has_reset_bg,
            "no deberia quedar fondo sin pintar en el claro"
        );
    }

    #[test]
    fn paperwhite_no_op_en_theme_oscuro() {
        // frappe no tiene background/text (None): el fill es no-op, asi que quedan
        // celdas con fondo Reset (deja pasar el del terminal).
        let (.., has_reset_bg) = fill_report(&Theme::frappe());
        assert!(has_reset_bg, "el theme oscuro no deberia pintar el fondo");
    }

    /// Concatena el texto de todos los spans de una `Line` en un solo String,
    /// para poder hacer assertions sobre lo que muestra (igual que los tests de
    /// tabs/switcher inspeccionan sus lineas).
    fn line_to_string(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn apply_action_movimiento_no_pide_salir() {
        // Una accion normal de movimiento devuelve Ok(false): no hay que salir.
        let mut doc = doc_with("hola");
        let salir = apply_action(&mut doc, Action::CursorRight, 10).unwrap();
        assert!(!salir, "CursorRight no deberia pedir salir");
        assert_eq!(
            doc.cursor_byte(),
            1,
            "CursorRight deberia avanzar un grafema"
        );
    }

    #[test]
    fn apply_action_edicion_modifica_el_doc() {
        // Insertar un caracter modifica el buffer y no pide salir.
        let mut doc = doc_with("");
        assert!(!apply_action(&mut doc, Action::InsertChar('x'), 10).unwrap());
        assert_eq!(doc.text(), "x");
        // InsertNewline parte la linea.
        assert!(!apply_action(&mut doc, Action::InsertNewline, 10).unwrap());
        assert_eq!(doc.text(), "x\n");
        // Backspace borra hacia atras.
        assert!(!apply_action(&mut doc, Action::Backspace, 10).unwrap());
        assert_eq!(doc.text(), "x");
    }

    #[test]
    fn apply_action_quit_pide_salir() {
        // Quit devuelve Ok(true) sin tocar el documento.
        let mut doc = doc_with("hola");
        assert!(apply_action(&mut doc, Action::Quit, 10).unwrap());
        assert_eq!(doc.text(), "hola");
    }

    #[test]
    fn apply_action_save_and_quit_pide_salir() {
        // SaveAndQuit guarda y devuelve Ok(true). Usamos un path temporal para
        // que el save no escriba en el cwd del repo.
        let dir = std::env::temp_dir().join(format!("typebar-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("saq.md");
        let mut doc = doc_with("contenido");
        doc.path = path.clone();
        assert!(apply_action(&mut doc, Action::SaveAndQuit, 10).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "contenido");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn status_bar_sin_seleccion_muestra_total_de_palabras() {
        // Sin seleccion: la status bar muestra el total de palabras del doc.
        use keybinding::StandardKeymap;
        let doc = doc_with("uno dos tres");
        let km = StandardKeymap;
        let line = status_bar(&doc, &km, &[]);
        let text = line_to_string(&line);
        assert!(text.contains("3 words"), "deberia mostrar el total: {text}");
        // Sin seleccion no aparece el formato "N/M".
        assert!(
            !text.contains("/3 words"),
            "no deberia haber seleccion: {text}"
        );
    }

    #[test]
    fn status_bar_con_seleccion_muestra_seleccionadas_sobre_total() {
        // Con seleccion activa: muestra "N/M" (palabras seleccionadas / total).
        use keybinding::StandardKeymap;
        let mut doc = doc_with("uno dos tres");
        // Seleccionar las primeras 3 letras ("uno" = 1 palabra) extendiendo a la
        // derecha desde el inicio.
        doc.extend_right();
        doc.extend_right();
        doc.extend_right();
        let km = StandardKeymap;
        let line = status_bar(&doc, &km, &[]);
        let text = line_to_string(&line);
        assert!(
            text.contains("1/3 words"),
            "deberia mostrar sel/total: {text}"
        );
    }

    #[test]
    fn status_bar_doc_dirty_muestra_el_marcador() {
        // Un doc con cambios sin guardar muestra "[+]" en la status bar.
        use keybinding::StandardKeymap;
        let mut doc = doc_with("hola");
        doc.insert_char('!'); // ensucia el doc
        assert!(doc.dirty, "el doc deberia quedar dirty tras editar");
        let km = StandardKeymap;
        let line = status_bar(&doc, &km, &[]);
        let text = line_to_string(&line);
        assert!(
            text.contains("[+]"),
            "deberia mostrar el marcador dirty: {text}"
        );
    }

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(Vec::<String>::new().into_iter());
        assert_eq!(a.path, DEFAULT_PATH);
        // Sin `--keys` el preset queda sin resolver (lo decide el config).
        assert_eq!(a.preset, None);
        // Sin `--export-html` el flag de export queda en false.
        assert!(!a.export_html);
    }

    #[test]
    fn parse_args_export_html_setea_el_flag() {
        let a = parse_args(vec!["notas.md".to_string(), "--export-html".to_string()].into_iter());
        assert!(a.export_html);
        assert_eq!(a.path, "notas.md");
    }

    #[test]
    fn parse_args_help_setea_el_flag() {
        // Tanto `--help` como `-h` prenden el flag de ayuda.
        assert!(parse_args(vec!["--help".to_string()].into_iter()).help);
        assert!(parse_args(vec!["-h".to_string()].into_iter()).help);
        // Sin el flag, queda apagado.
        assert!(!parse_args(vec!["notas.md".to_string()].into_iter()).help);
    }

    #[test]
    fn centered_column_centra_y_respeta_terminales_angostas() {
        // Ancho mayor al maximo: se centra en una columna de WHITEPAPER_WIDTH.
        let full = Rect {
            x: 0,
            y: 3,
            width: 120,
            height: 10,
        };
        let col = centered_column(full, WHITEPAPER_WIDTH);
        assert_eq!(col.width, WHITEPAPER_WIDTH);
        assert_eq!(col.x, (120 - WHITEPAPER_WIDTH) / 2);
        // Alto y posicion vertical intactos.
        assert_eq!(col.y, 3);
        assert_eq!(col.height, 10);
        // Terminal mas angosta que el maximo: se usa todo el ancho (sin tocar).
        let narrow = Rect {
            x: 0,
            y: 0,
            width: 40,
            height: 5,
        };
        assert_eq!(centered_column(narrow, WHITEPAPER_WIDTH), narrow);
    }

    #[test]
    fn export_doc_to_html_escribe_el_html_del_buffer() {
        // Exporta el contenido EN MEMORIA (no del disco) a `<archivo>.html`.
        let dir = std::env::temp_dir().join(format!("typebar-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut doc = doc_with("# Hola");
        doc.path = dir.join("nota.md");
        let out = export_doc_to_html(&doc).unwrap();
        assert_eq!(out, dir.join("nota.html"));
        let html = std::fs::read_to_string(&out).unwrap();
        assert!(html.contains("<h1>Hola</h1>"), "html: {html}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn html_output_path_cambia_la_extension() {
        // Con extension: se reemplaza por `.html`.
        assert_eq!(
            html_output_path("notes.md"),
            std::path::PathBuf::from("notes.html")
        );
        // Sin extension: se agrega `.html`.
        assert_eq!(
            html_output_path("notes"),
            std::path::PathBuf::from("notes.html")
        );
    }

    #[test]
    fn pdf_temp_path_usa_el_stem_y_vive_en_temp_dir() {
        let out = pdf_temp_path(std::path::Path::new("notes.md"));
        assert_eq!(out, std::env::temp_dir().join("notes.print.html"));
    }

    #[test]
    fn pdf_temp_path_sin_extension_agrega_el_sufijo() {
        let out = pdf_temp_path(std::path::Path::new("notes"));
        assert_eq!(out, std::env::temp_dir().join("notes.print.html"));
    }

    #[test]
    fn pdf_temp_path_sin_stem_cae_a_untitled() {
        // Un path sin nombre de archivo (ej vacio) no tiene stem: cae a
        // "untitled" en vez de producir un path invalido/vacio.
        let out = pdf_temp_path(std::path::Path::new(""));
        assert_eq!(out, std::env::temp_dir().join("untitled.print.html"));
    }

    #[test]
    fn parse_args_posicional_es_path() {
        let a = parse_args(vec!["notas.md".to_string()].into_iter());
        assert_eq!(a.path, "notas.md");
        assert_eq!(a.preset, None);
    }

    #[test]
    fn parse_args_keys_separado() {
        let a = parse_args(vec!["--keys".to_string(), "vim".to_string()].into_iter());
        assert_eq!(a.preset.as_deref(), Some("vim"));
        assert_eq!(a.path, DEFAULT_PATH);
    }

    #[test]
    fn parse_args_keys_con_igual() {
        let a = parse_args(vec!["--keys=vim".to_string(), "notas.md".to_string()].into_iter());
        assert_eq!(a.preset.as_deref(), Some("vim"));
        assert_eq!(a.path, "notas.md");
    }

    #[test]
    fn parse_args_keys_despues_del_path() {
        let a = parse_args(
            vec![
                "notas.md".to_string(),
                "--keys".to_string(),
                "vim".to_string(),
            ]
            .into_iter(),
        );
        assert_eq!(a.preset.as_deref(), Some("vim"));
        assert_eq!(a.path, "notas.md");
    }

    /// Construye una `Config` con un preset dado para los tests de precedencia.
    /// La seccion `[ui]` queda en su default (irrelevante para estos tests).
    fn config_con_preset(preset: Option<&str>) -> config::Config {
        config::Config {
            keybindings: config::KeybindingsConfig {
                preset: preset.map(str::to_string),
                bind: Vec::new(),
            },
            ui: config::UiConfig::default(),
        }
    }

    #[test]
    fn precedencia_cli_gana_sobre_config() {
        // El flag `--keys` siempre manda, aunque el config diga otra cosa.
        let config = config_con_preset(Some("wordstar"));
        let preset = resolve_preset(Some("vim".to_string()), &config);
        assert_eq!(preset, "vim");
    }

    #[test]
    fn precedencia_config_cuando_no_hay_cli() {
        // Sin `--keys`, gana el preset del config file.
        let config = config_con_preset(Some("vim"));
        let preset = resolve_preset(None, &config);
        assert_eq!(preset, "vim");
    }

    #[test]
    fn precedencia_default_sin_cli_ni_config() {
        let config = config_con_preset(None);
        let preset = resolve_preset(None, &config);
        assert_eq!(preset, DEFAULT_PRESET);
    }

    #[test]
    fn config_con_preset_invalido_cae_a_default() {
        // Un preset desconocido en el config se ignora y cae al default.
        let config = config_con_preset(Some("loquesea"));
        let preset = resolve_preset(None, &config);
        assert_eq!(preset, DEFAULT_PRESET);
    }
}
