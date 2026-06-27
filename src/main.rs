//! typebar: editor de Markdown WYSIWYG para terminal (milestone editable minimo).
//!
//! Render "soft WYSIWYG": los marcadores (`**`, `*`, backticks, `#`) siempre
//! quedan visibles y dimmeados, asi el cursor se mueve 1:1 sobre el texto y la
//! posicion (linea, col) del buffer mapea directo a la columna en pantalla.
//!
//! El teclado se maneja por presets intercambiables (ver `keybinding`): por
//! default `standard` (modeless, flechas), con `vim` (modal) y `wordstar`
//! (modeless con chords tipo `Ctrl-K S`) opt-in via el flag `--keys`. El loop
//! acumula teclas en un buffer `pending` para resolver secuencias multi-tecla.

mod buffers;
mod config;
mod document;
mod export;
mod files;
mod fuzzy;
mod i18n;
mod keybinding;
mod markdown;
mod overlay;
mod palette;
mod render;
mod search;
mod switcher;
mod tabs;
mod text;
mod theme;

use document::{Document, Mode};
use keybinding::{Action, Binding, CustomKeymap, Keymap, Resolve, keymap_from_name, parse_binding};
use markdown::InlineKind;
use overlay::Overlay;
use palette::{Palette, PaletteOutcome};
use switcher::{Switcher, SwitcherOutcome};
use theme::Theme;

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
    // Preset base + overrides del usuario encima (si los hay).
    let keymap = apply_overrides(keymap_from_name(&preset), &config.keybindings.bind);

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
        keymap.as_ref(),
        Themes {
            dark: dark_theme,
            light: light_theme,
            paper: Theme::paper(),
        },
        configured_is_light,
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

/// Estado de VISTA del run loop: todo lo que vive entre frames y que `draw` lee y
/// las acciones tocan, pero que NO es ni el documento, ni el keymap, ni los themes,
/// ni el nivel de wysiwyg (ese contexto es aparte, fijo durante el run). Agrupar
/// estos campos en un struct evita threadear 8-9 params sueltos por `draw` y
/// `dispatch_action`. Lo posee `run`.
struct AppState {
    /// Offset vertical de scroll: primera linea visible del documento. `draw` lo
    /// ajusta para mantener el cursor dentro del viewport.
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
    /// Mensaje transitorio en la status bar (ej "save failed: ..."): se muestra en
    /// el proximo frame y se limpia al apretar la siguiente tecla. Evita que un
    /// error de guardado tumbe el editor (writing-first: nunca perder el buffer).
    flash: Option<String>,
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
            flash: None,
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

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    doc: Document,
    keymap: &dyn Keymap,
    themes: Themes,
    light_on: bool,
    wysiwyg_level: u8,
) -> std::io::Result<()> {
    // Los buffers abiertos. El editor siempre opera sobre el activo
    // (`workspace.active*`); el multi-archivo es transparente para draw/acciones/
    // overlays. Arranca con el documento que abrio `main`.
    let mut workspace = buffers::Workspace::new(doc);
    // Estado de vista del loop (scroll, zen, overlay, pickers, etc.) agrupado.
    let mut state = AppState::new(light_on);

    loop {
        // Theme activo: el modo whitepaper usa su theme monocromo (tinta sobre
        // papel) y gana sobre todo; si no, el claro (Latte) cuando el toggle `^O L`
        // esta on; si no, el configurado (oscuro). Se recalcula cada frame.
        let theme = if state.whitepaper {
            &themes.paper
        } else if state.light_on {
            &themes.light
        } else {
            &themes.dark
        };
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
            let (line, hits) = tabs::build(&titles, workspace.active_index(), theme);
            (Some(line), hits)
        } else {
            (None, Vec::new())
        };
        terminal.draw(|frame| {
            draw(
                frame,
                workspace.active(),
                keymap,
                theme,
                wysiwyg_level,
                &mut state,
                tab_line.clone(),
            );
            // Paperwhite: si el theme activo es claro, pinta fondo/texto sobre el
            // frame ya dibujado (editor, chrome y pickers de una). No-op en oscuros.
            apply_theme_fill(frame, theme);
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
                    match dispatch_action(action, &mut workspace, keymap, &mut state) {
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
                        .open_or_switch(&path, keymap.initial_mode())
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
        if (state.zen || state.whitepaper) && key.code == KeyCode::Esc && !keymap.is_modal() {
            state.zen = false;
            state.whitepaper = false;
            state.pending.clear();
            continue;
        }

        state.pending.push(key);
        match keymap.resolve(workspace.active().mode, &state.pending) {
            Resolve::Action(action) => {
                state.pending.clear();
                // Despachamos por el mismo helper que usa la paleta, asi un action
                // resuelto por el keymap y uno elegido en la paleta recorren un
                // unico camino (sin duplicar la logica de overlays/zen/switcher).
                // Un error (de guardado) NO tumba el editor: se muestra y sigue. Si
                // la accion cambio de buffer, se resetea el scroll compartido.
                let before = workspace.active_index();
                match dispatch_action(action, &mut workspace, keymap, &mut state) {
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
    keymap: &dyn Keymap,
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
        // Exportar el buffer actual a HTML sin salir; el resultado (path o error)
        // va al flash, no tumba el editor. Escribe a `<archivo>.html` al lado.
        Action::ExportHtml => {
            state.flash = Some(match export_doc_to_html(workspace.active()) {
                Ok(out) => i18n::exported_to(&out),
                Err(e) => i18n::export_failed(e),
            });
        }
        // Nuevo archivo: crea un buffer vacio y lo enfoca. El draw reclampa el
        // scroll solo (el cursor del buffer nuevo arranca arriba).
        Action::NewBuffer => workspace.new_buffer(keymap.initial_mode()),
        // Cambiar de buffer (cycle). El draw reclampa el scroll al nuevo buffer.
        Action::NextBuffer => workspace.next_buffer(),
        Action::PrevBuffer => workspace.prev_buffer(),
        Action::OpenSwitcher => {
            // Candidatos: archivos del proyecto (cwd recursivo) mas los buffers
            // abiertos que no esten ya en la lista (p.ej. fuera del cwd), para
            // poder volver a cualquiera.
            let mut candidates = files::discover(".");
            for p in workspace.paths() {
                if !candidates.iter().any(|c| c == p) {
                    candidates.push(p.to_path_buf());
                }
            }
            state.switcher = Some(Switcher::new(candidates));
        }
        // Abrir la paleta de comandos. Como `OpenPalette` se excluye del catalogo
        // de comandos, no hay forma de recursar desde la propia paleta.
        Action::OpenPalette => state.palette = Some(Palette::new(keymap)),
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
    // La paleta y el switcher (mutuamente excluyentes) tapan todo: cada uno se
    // dibuja via su modulo y corta el draw. El render vive en el modulo respectivo.
    if let Some(pal) = state.palette.as_ref() {
        pal.render(frame, theme);
        return;
    }
    if let Some(sw) = state.switcher.as_ref() {
        sw.render(frame, theme);
        return;
    }

    // Snapshot de los flags de vista que se leen varias veces aca; evita tener
    // `&state` vivo mientras mas abajo se mutan `state.scroll`/`viewport_height`.
    // Whitepaper es un superset de zen para el chrome (oculta borde/toolbar/status
    // igual); ademas centra el editor en una columna fija (mas abajo).
    let whitepaper = state.whitepaper;
    let zen = state.zen || whitepaper;

    // Zen/focus: ocultamos todo el chrome (borde, toolbar, status) para dejar
    // solo el texto. Excepcion: si hay un overlay de busqueda activo reservamos
    // la ultima linea para el minibuffer (si no, no se veria que se esta
    // buscando). Fuera de zen: editor (resto) + toolbar + gap + status bar; el
    // gap de 1 linea separa visualmente el chrome de comandos del de estado.
    let (tabs_area, editor_area, hints_area, status_area) = if zen {
        if state.overlay.is_some() {
            let [editor, mini] =
                Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());
            (None, editor, None, Some(mini))
        } else {
            (None, frame.area(), None, None)
        }
    } else if tabs.is_some() {
        // Con tabs (>=2 buffers) reservamos una fila ARRIBA de todo para la barra.
        let [tabs_a, editor, hints, _gap, status] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        (Some(tabs_a), editor, Some(hints), Some(status))
    } else {
        let [editor, hints, _gap, status] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());
        (None, editor, Some(hints), Some(status))
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

    // En zen no hay borde (el editor ocupa todo); fuera de zen, el Block bordered
    // come 1 linea arriba y 1 abajo. Este offset alinea el alto util y el cursor.
    let border: u16 = if zen { 0 } else { 1 };
    // Margen izquierdo, para que el texto no quede pegado al borde. En zen (sin
    // marco) le damos un poco mas de aire (2) ya que no hay borde que separe;
    // fuera de zen alcanza con 1 (el borde ya separa). Suma al offset del cursor.
    let pad_left: u16 = if zen { 2 } else { 1 };
    // Alto util dentro del borde del Block.
    let viewport_height = editor_area.height.saturating_sub(2 * border) as usize;
    // Lo exponemos al loop para que PageUp/PageDown sepan cuanto mover.
    state.viewport_height = viewport_height.max(1);

    // Ajustar scroll para que el cursor quede dentro del viewport.
    if viewport_height > 0 {
        if doc.line < state.scroll {
            state.scroll = doc.line;
        } else if doc.line >= state.scroll + viewport_height {
            state.scroll = doc.line + 1 - viewport_height;
        }
    }
    // Tras reclampar, lo leemos en un local: simplifica los usos de abajo y
    // evita re-tomar `&state` mientras se sigue dibujando.
    let scroll = state.scroll;

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

    // En zen el Block va sin borde ni titulo (solo texto), pero con el mismo
    // margen izquierdo; fuera de zen, bordered con el path en el titulo. El
    // padding tiene que coincidir con `pad_left` (que ya usa el cursor).
    let block = if zen {
        Block::default().padding(Padding::new(pad_left, 0, 0, 0))
    } else {
        Block::bordered()
            .title(format!(" typebar · {} ", doc.path.display()))
            .padding(Padding::new(pad_left, 0, 0, 0))
    };
    // En Nivel 2 la linea con el cursor se renderiza como Nivel 1 (markers
    // visibles) para preservar el mapeo cursor->columna 1:1. Las demas lineas
    // ocultan los delimiters inline (ver `render::render`).
    let lines = render::render(
        &text,
        doc.selection_byte_range(),
        &matches,
        current,
        theme,
        Some(doc.line),
        wysiwyg_level,
    );
    let paragraph = Paragraph::new(lines)
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
        if let Some(ov) = state.overlay.as_ref() {
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

    // Cursor: +1,+1 por el borde del Block, y restando scroll. La X es la columna
    // *visual* (celdas), no el indice de char: asi cae sobre el glifo que dibujo el
    // render aunque haya CJK/emoji de doble ancho.
    if doc.line >= scroll {
        let cursor_x = editor_area.x + border + pad_left + doc.display_col() as u16;
        let cursor_y = editor_area.y + border + (doc.line - scroll) as u16;
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
    let dirty = if doc.dirty { "[+] " } else { "" };
    let left = format!("{}{}", left, dirty);
    // Contador de palabras: con seleccion activa muestra "seleccionadas/total".
    let words = match doc.selection_word_count() {
        Some(sel) => i18n::words_count_selection(sel, doc.word_count()),
        None => i18n::words_count(doc.word_count()),
    };
    let right = format!(" {} · {}:{} ", words, doc.line + 1, doc.display_col() + 1);
    let mut spans = vec![
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
        | Action::NewBuffer
        | Action::NextBuffer
        | Action::PrevBuffer
        | Action::OpenSwitcher
        | Action::OpenPalette => {}
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
    use document::test_support::doc_with;

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
        let mut terminal = Terminal::new(TestBackend::new(60, 12)).unwrap();
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
        // Fuera de zen: el borde con el titulo (`typebar`) y la toolbar (`Save`,
        // locale En por default en tests) estan presentes, igual que el texto.
        let screen = render_to_string(false, None, None);
        assert!(screen.contains("typebar"), "falta el titulo del borde");
        assert!(screen.contains("Save"), "falta la toolbar");
        assert!(screen.contains("hola mundo"), "falta el texto");
    }

    #[test]
    fn draw_zen_oculta_chrome_pero_muestra_texto() {
        // En zen: sin borde/titulo ni toolbar; solo el texto.
        let screen = render_to_string(true, None, None);
        assert!(
            !screen.contains("typebar"),
            "el titulo no deberia verse en zen"
        );
        assert!(
            !screen.contains("Save"),
            "la toolbar no deberia verse en zen"
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
    fn draw_switcher_tapa_el_editor_y_muestra_prompt_y_candidatos() {
        // Con el switcher abierto: se ve el prompt (locale En por default) y los
        // candidatos, y NO el texto del editor de fondo.
        let sw = Switcher::new(vec![
            std::path::PathBuf::from("src/main.rs"),
            std::path::PathBuf::from("README.md"),
        ]);
        let screen = render_to_string(false, Some(sw), None);
        assert!(
            screen.contains("go to file:"),
            "falta el prompt del switcher"
        );
        assert!(screen.contains("main.rs"), "falta un candidato");
        assert!(screen.contains("README.md"), "falta un candidato");
        assert!(
            !screen.contains("hola mundo"),
            "el editor de fondo no deberia verse con el switcher abierto"
        );
    }

    #[test]
    fn draw_palette_tapa_el_editor_y_muestra_prompt_y_comandos() {
        // Con la paleta abierta: se ve el prompt (locale En por default) y algun
        // comando, y NO el texto del editor de fondo.
        let km = keybinding::StandardKeymap;
        let pal = Palette::new(&km);
        let screen = render_to_string(false, None, Some(pal));
        assert!(screen.contains("command:"), "falta el prompt de la paleta");
        assert!(screen.contains("Save"), "falta algun comando");
        assert!(
            !screen.contains("hola mundo"),
            "el editor de fondo no deberia verse con la paleta abierta"
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
