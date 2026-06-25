//! Keybindings remapeables por el usuario: un *overlay* sobre un preset base.
//!
//! `CustomKeymap` envuelve cualquier `Keymap` (standard/vim/wordstar) y antepone
//! una tabla de bindings definida por el usuario en `config.toml`. La regla de
//! precedencia es simple: **el overlay es dueño de su arbol de prefijos**. Al
//! resolver una secuencia de teclas:
//!
//! 1. Si la secuencia matchea EXACTO un binding del usuario -> esa `Action`.
//! 2. Si la secuencia es prefijo (estricto) de algun binding del usuario ->
//!    `Pending` (esperar mas teclas para completar el chord del usuario).
//! 3. Si no, se delega en el preset base.
//!
//! Esto implica que si el usuario remapea una tecla (o arranca un chord en ella)
//! "tapa" lo que el preset hacia con esa misma secuencia. Es el comportamiento
//! esperado de un override y mantiene la resolucion predecible.
//!
//! Cada binding puede acotarse a un `Mode` (Normal/Insert/Visual) o ser
//! agnostico (`None`, aplica en todos). En presets modeless el modo siempre es
//! Insert, asi que un binding con `mode = "normal"` alli simplemente nunca
//! matchea (inofensivo).
//!
//! Formato de las teclas: tokens separados por espacios, cada token es una tecla
//! con modificadores opcionales unidos por `-`, ej: `ctrl-s`, `ctrl-k ctrl-x`,
//! `shift-right`. Los modificadores validos son `ctrl`/`shift`/`alt`. Las teclas
//! con nombre: `left right up down enter backspace esc tab space delete home end
//! pageup pagedown minus plus`; cualquier otra de un solo caracter se toma literal.
//! Una letra combinada con `ctrl` se normaliza a minuscula (convencion de la
//! terminal: `Ctrl-S` llega como `Char('s') + CONTROL`).

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{Action, Hint, Keymap, Resolve};
use crate::document::Mode;

/// Una tecla individual normalizada para comparar contra los `KeyEvent` que
/// entrega la terminal: solo nos quedamos con codigo y los modificadores
/// relevantes (ctrl/shift/alt).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KeySpec {
    code: KeyCode,
    mods: KeyModifiers,
}

impl KeySpec {
    /// True si este spec describe al `KeyEvent` dado (comparando codigo y los
    /// modificadores relevantes, ignorando bits de estado/keypad que la terminal
    /// pueda agregar).
    fn matches(&self, key: &KeyEvent) -> bool {
        self.code == key.code && self.mods == relevant_mods(key.modifiers)
    }
}

/// Un binding del usuario: secuencia de teclas -> accion, opcionalmente acotado
/// a un modo. Publico pero opaco: se construye via `parse_binding`.
#[derive(Debug, Clone)]
pub struct Binding {
    mode: Option<Mode>,
    keys: Vec<KeySpec>,
    action: Action,
}

/// Keymap que aplica overrides del usuario encima de un preset base.
pub struct CustomKeymap {
    base: Box<dyn Keymap>,
    bindings: Vec<Binding>,
}

impl CustomKeymap {
    /// Envuelve `base` con la tabla de `bindings` del usuario. No valida
    /// conflictos entre bindings: ante secuencias ambiguas gana el primero que
    /// matchea exacto (orden del config), y cualquier prefijo dispara `Pending`.
    pub fn new(base: Box<dyn Keymap>, bindings: Vec<Binding>) -> Self {
        CustomKeymap { base, bindings }
    }
}

impl Keymap for CustomKeymap {
    fn resolve(&self, mode: Mode, keys: &[KeyEvent]) -> Resolve {
        let mut is_prefix = false;
        for b in &self.bindings {
            // Un binding acotado a un modo solo aplica en ese modo.
            if let Some(m) = b.mode
                && m != mode
            {
                continue;
            }
            if seq_matches(&b.keys, keys) {
                return Resolve::Action(b.action);
            }
            // Prefijo estricto: la secuencia tipeada es comienzo de un chord del
            // usuario mas largo -> hay que esperar mas teclas.
            if b.keys.len() > keys.len() && seq_matches(&b.keys[..keys.len()], keys) {
                is_prefix = true;
            }
        }
        if is_prefix {
            return Resolve::Pending;
        }
        // El overlay no toca esta secuencia: la maneja el preset base.
        self.base.resolve(mode, keys)
    }

    fn is_modal(&self) -> bool {
        self.base.is_modal()
    }

    fn initial_mode(&self) -> Mode {
        self.base.initial_mode()
    }

    fn name(&self) -> &'static str {
        self.base.name()
    }

    fn hints(&self, mode: Mode) -> Vec<Hint> {
        let mut hints = self.base.hints(mode);
        // Reflejar los remapeos: si el usuario rebindeo la accion de un hint (en
        // este modo o de forma agnostica), mostrar SU tecla en vez de la del
        // preset. Los hints estructurales (prefijos sin accion) no se remapean.
        for hint in &mut hints {
            let Some(action) = hint.action else { continue };
            if let Some(b) = self
                .bindings
                .iter()
                .find(|b| b.action == action && b.mode.is_none_or(|m| m == mode))
            {
                hint.keys = render_key_label(&b.keys);
            }
        }
        hints
    }

    fn chord_hints(&self, mode: Mode, pending: &[KeyEvent]) -> Vec<Hint> {
        // Las continuaciones de chords se delegan al preset base. (Reflejar
        // chords definidos por el usuario en la barra queda pendiente: requiere
        // mapear cada accion a su etiqueta, que hoy solo conocen los presets.)
        self.base.chord_hints(mode, pending)
    }
}

/// Etiqueta legible de una secuencia de teclas para la barra de atajos, ej
/// `^K X` o `⇧→`. Los tokens se separan por espacio.
fn render_key_label(specs: &[KeySpec]) -> String {
    specs
        .iter()
        .map(render_one_key)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Etiqueta de una sola tecla: prefijo de modificadores (`^` ctrl, `⇧` shift,
/// `M-` alt) + el nombre de la tecla.
fn render_one_key(spec: &KeySpec) -> String {
    let mut s = String::new();
    if spec.mods.contains(KeyModifiers::CONTROL) {
        s.push('^');
    }
    if spec.mods.contains(KeyModifiers::SHIFT) {
        s.push('⇧');
    }
    if spec.mods.contains(KeyModifiers::ALT) {
        s.push_str("M-");
    }
    s.push_str(&key_code_label(spec.code));
    s
}

/// Nombre corto de un `KeyCode` para la barra de atajos. Las letras bajo Ctrl se
/// muestran en mayuscula (convencion `^S`).
fn key_code_label(code: KeyCode) -> String {
    match code {
        KeyCode::Char(c) => c.to_ascii_uppercase().to_string(),
        KeyCode::Left => "←".to_string(),
        KeyCode::Right => "→".to_string(),
        KeyCode::Up => "↑".to_string(),
        KeyCode::Down => "↓".to_string(),
        KeyCode::Enter => "Enter".to_string(),
        KeyCode::Backspace => "Bksp".to_string(),
        KeyCode::Esc => "Esc".to_string(),
        KeyCode::Tab => "Tab".to_string(),
        KeyCode::Delete => "Del".to_string(),
        KeyCode::Home => "Home".to_string(),
        KeyCode::End => "End".to_string(),
        KeyCode::PageUp => "PgUp".to_string(),
        KeyCode::PageDown => "PgDn".to_string(),
        other => format!("{other:?}"),
    }
}

/// True si cada `KeySpec` de `specs` describe al `KeyEvent` en la misma posicion
/// y las longitudes coinciden.
fn seq_matches(specs: &[KeySpec], keys: &[KeyEvent]) -> bool {
    specs.len() == keys.len() && specs.iter().zip(keys).all(|(s, k)| s.matches(k))
}

/// Recorta los modificadores a los que nos importan para el matcheo. La terminal
/// puede setear bits extra (keypad, etc.) que no queremos comparar.
fn relevant_mods(m: KeyModifiers) -> KeyModifiers {
    m & (KeyModifiers::CONTROL | KeyModifiers::SHIFT | KeyModifiers::ALT)
}

/// Parsea un binding del usuario a partir de sus campos crudos del config.
/// Devuelve `Err(mensaje)` si las teclas, la accion o el modo no son validos;
/// `main` reporta el error y descarta ese binding (no aborta el arranque).
pub fn parse_binding(keys: &str, action: &str, mode: Option<&str>) -> Result<Binding, String> {
    let keys = parse_key_seq(keys)?;
    let action = parse_action(action)?;
    let mode = match mode {
        Some(m) => Some(parse_mode(m)?),
        None => None,
    };
    Ok(Binding { mode, keys, action })
}

/// Parsea una secuencia de teclas (`"ctrl-k ctrl-x"`) a una lista de `KeySpec`.
/// Vacia es un error (un binding sin teclas no tiene sentido).
fn parse_key_seq(s: &str) -> Result<Vec<KeySpec>, String> {
    let specs: Vec<KeySpec> = s
        .split_whitespace()
        .map(parse_key)
        .collect::<Result<_, _>>()?;
    if specs.is_empty() {
        return Err("secuencia de teclas vacia".to_string());
    }
    Ok(specs)
}

/// Parsea un token de tecla (`"ctrl-s"`, `"shift-right"`, `"a"`, `"-"`) a un
/// `KeySpec`. El ultimo segmento separado por `-` es la tecla; los previos son
/// modificadores. Un token de un solo caracter se toma literal (asi `"-"` es la
/// tecla guion sin ambiguedad con el separador).
fn parse_key(token: &str) -> Result<KeySpec, String> {
    if token.chars().count() == 1 {
        return Ok(KeySpec {
            code: KeyCode::Char(token.chars().next().unwrap()),
            mods: KeyModifiers::NONE,
        });
    }
    let mut parts: Vec<&str> = token.split('-').collect();
    let key_part = parts
        .pop()
        .filter(|s| !s.is_empty())
        .ok_or_else(|| format!("tecla incompleta: {token:?}"))?;

    let mut mods = KeyModifiers::NONE;
    for m in parts {
        match m.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => mods |= KeyModifiers::CONTROL,
            "shift" => mods |= KeyModifiers::SHIFT,
            "alt" | "opt" | "option" => mods |= KeyModifiers::ALT,
            "" => return Err(format!("modificador vacio en {token:?}")),
            other => return Err(format!("modificador desconocido: {other:?}")),
        }
    }

    let mut code = parse_key_code(key_part)?;
    // Convencion de terminal: una letra con CONTROL llega en minuscula.
    if mods.contains(KeyModifiers::CONTROL)
        && let KeyCode::Char(c) = code
    {
        code = KeyCode::Char(c.to_ascii_lowercase());
    }
    Ok(KeySpec { code, mods })
}

/// Traduce la parte "tecla" de un token a un `KeyCode`. Un solo caracter va
/// literal; los nombres conocidos a su `KeyCode` correspondiente.
fn parse_key_code(s: &str) -> Result<KeyCode, String> {
    if s.chars().count() == 1 {
        return Ok(KeyCode::Char(s.chars().next().unwrap()));
    }
    let code = match s.to_ascii_lowercase().as_str() {
        "left" => KeyCode::Left,
        "right" => KeyCode::Right,
        "up" => KeyCode::Up,
        "down" => KeyCode::Down,
        "enter" | "return" => KeyCode::Enter,
        "backspace" | "bs" => KeyCode::Backspace,
        "esc" | "escape" => KeyCode::Esc,
        "tab" => KeyCode::Tab,
        "space" => KeyCode::Char(' '),
        "delete" | "del" => KeyCode::Delete,
        "home" => KeyCode::Home,
        "end" => KeyCode::End,
        "pageup" => KeyCode::PageUp,
        "pagedown" => KeyCode::PageDown,
        "minus" => KeyCode::Char('-'),
        "plus" => KeyCode::Char('+'),
        other => return Err(format!("tecla desconocida: {other:?}")),
    };
    Ok(code)
}

/// Traduce un nombre de accion en kebab-case a su `Action`. No incluye
/// `InsertChar` (es el fallback de tipeo, no algo que se remapee) pero si
/// `insert-newline`.
fn parse_action(s: &str) -> Result<Action, String> {
    let action = match s.trim().to_ascii_lowercase().as_str() {
        "cursor-left" => Action::CursorLeft,
        "cursor-right" => Action::CursorRight,
        "cursor-up" => Action::CursorUp,
        "cursor-down" => Action::CursorDown,
        "insert-newline" => Action::InsertNewline,
        "backspace" => Action::Backspace,
        "delete-char" => Action::DeleteChar,
        "enter-insert" => Action::EnterInsert,
        "enter-normal" => Action::EnterNormal,
        "insert-after" => Action::InsertAfter,
        "open-line-below" => Action::OpenLineBelow,
        "line-start" => Action::LineStart,
        "line-end" => Action::LineEnd,
        "doc-start" => Action::DocStart,
        "doc-end" => Action::DocEnd,
        "page-up" => Action::PageUp,
        "page-down" => Action::PageDown,
        "save" => Action::Save,
        "save-and-quit" => Action::SaveAndQuit,
        "quit" => Action::Quit,
        "toggle-bold" => Action::ToggleBold,
        "toggle-italic" => Action::ToggleItalic,
        "toggle-code" => Action::ToggleCode,
        "enter-visual" => Action::EnterVisual,
        "select-left" => Action::SelectLeft,
        "select-right" => Action::SelectRight,
        "select-up" => Action::SelectUp,
        "select-down" => Action::SelectDown,
        "delete-selection" => Action::DeleteSelection,
        "undo" => Action::Undo,
        "redo" => Action::Redo,
        "yank" => Action::Yank,
        "paste" => Action::Paste,
        "search" => Action::Search,
        "replace" => Action::Replace,
        "toggle-zen" => Action::ToggleZen,
        "open-switcher" => Action::OpenSwitcher,
        "open-palette" => Action::OpenPalette,
        other => return Err(format!("accion desconocida: {other:?}")),
    };
    Ok(action)
}

/// Traduce un nombre de modo a `Mode`.
fn parse_mode(s: &str) -> Result<Mode, String> {
    match s.trim().to_ascii_lowercase().as_str() {
        "normal" => Ok(Mode::Normal),
        "insert" => Ok(Mode::Insert),
        "visual" => Ok(Mode::Visual),
        other => Err(format!("modo desconocido: {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinding::StandardKeymap;
    use ratatui::crossterm::event::KeyEvent;

    fn ev(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    #[test]
    fn parsea_tecla_simple_con_ctrl_en_minuscula() {
        let spec = parse_key("ctrl-S").unwrap();
        assert_eq!(spec.code, KeyCode::Char('s'));
        assert_eq!(spec.mods, KeyModifiers::CONTROL);
    }

    #[test]
    fn parsea_tecla_con_nombre_y_shift() {
        let spec = parse_key("shift-right").unwrap();
        assert_eq!(spec.code, KeyCode::Right);
        assert_eq!(spec.mods, KeyModifiers::SHIFT);
    }

    #[test]
    fn parsea_guion_literal() {
        let spec = parse_key("-").unwrap();
        assert_eq!(spec.code, KeyCode::Char('-'));
        assert_eq!(spec.mods, KeyModifiers::NONE);
    }

    #[test]
    fn token_invalido_es_error() {
        assert!(parse_key("ctrl-").is_err());
        assert!(parse_key("hyper-x").is_err());
        assert!(parse_key("noexiste").is_err());
    }

    #[test]
    fn secuencia_vacia_es_error() {
        assert!(parse_key_seq("   ").is_err());
    }

    #[test]
    fn parsea_secuencia_de_chord() {
        let specs = parse_key_seq("ctrl-k ctrl-x").unwrap();
        assert_eq!(specs.len(), 2);
        assert_eq!(specs[0].code, KeyCode::Char('k'));
        assert_eq!(specs[1].code, KeyCode::Char('x'));
    }

    #[test]
    fn accion_y_modo_desconocidos_son_error() {
        assert!(parse_action("frobnicate").is_err());
        assert!(parse_mode("godmode").is_err());
        assert_eq!(parse_action("save-and-quit").unwrap(), Action::SaveAndQuit);
        assert_eq!(parse_action("toggle-zen").unwrap(), Action::ToggleZen);
        assert_eq!(parse_mode("visual").unwrap(), Mode::Visual);
    }

    #[test]
    fn override_simple_gana_al_preset() {
        // standard mapea Ctrl-Q -> Quit; lo remapeamos a Save.
        let b = parse_binding("ctrl-q", "save", None).unwrap();
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![b]);
        assert_eq!(
            km.resolve(
                Mode::Insert,
                &[ev(KeyCode::Char('q'), KeyModifiers::CONTROL)]
            ),
            Resolve::Action(Action::Save)
        );
    }

    #[test]
    fn tecla_no_overrideada_cae_al_preset() {
        let b = parse_binding("ctrl-q", "save", None).unwrap();
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![b]);
        // Ctrl-S no fue tocado: sigue siendo Save del preset.
        assert_eq!(
            km.resolve(
                Mode::Insert,
                &[ev(KeyCode::Char('s'), KeyModifiers::CONTROL)]
            ),
            Resolve::Action(Action::Save)
        );
        // Una letra normal sigue insertandose via el preset.
        assert_eq!(
            km.resolve(Mode::Insert, &[ev(KeyCode::Char('a'), KeyModifiers::NONE)]),
            Resolve::Action(Action::InsertChar('a'))
        );
    }

    #[test]
    fn chord_del_usuario_da_pending_y_luego_action() {
        // Definimos un chord nuevo Ctrl-K Ctrl-X -> SaveAndQuit sobre standard.
        let b = parse_binding("ctrl-k ctrl-x", "save-and-quit", None).unwrap();
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![b]);
        let ck = ev(KeyCode::Char('k'), KeyModifiers::CONTROL);
        let cx = ev(KeyCode::Char('x'), KeyModifiers::CONTROL);
        // Primera tecla: prefijo del chord -> Pending.
        assert_eq!(km.resolve(Mode::Insert, &[ck]), Resolve::Pending);
        // Secuencia completa -> Action.
        assert_eq!(
            km.resolve(Mode::Insert, &[ck, cx]),
            Resolve::Action(Action::SaveAndQuit)
        );
    }

    #[test]
    fn binding_acotado_a_modo_solo_aplica_en_ese_modo() {
        // Override solo en Normal: en Insert no debe aplicar.
        let b = parse_binding("ctrl-d", "cursor-right", Some("normal")).unwrap();
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![b]);
        let cd = ev(KeyCode::Char('d'), KeyModifiers::CONTROL);
        assert_eq!(
            km.resolve(Mode::Normal, &[cd]),
            Resolve::Action(Action::CursorRight)
        );
        // En Insert el override no aplica; standard no bindea Ctrl-D -> None.
        assert_eq!(km.resolve(Mode::Insert, &[cd]), Resolve::None);
    }

    #[test]
    fn delega_propiedades_del_preset_base() {
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![]);
        assert_eq!(km.name(), "standard");
        assert!(!km.is_modal());
        assert_eq!(km.initial_mode(), Mode::Insert);
    }

    #[test]
    fn hints_reflejan_el_remapeo_del_usuario() {
        // standard muestra Guardar con "^S"; si el usuario lo remapea a Ctrl-W,
        // la barra de atajos debe mostrar "^W".
        let b = parse_binding("ctrl-w", "save", None).unwrap();
        let km = CustomKeymap::new(Box::new(StandardKeymap), vec![b]);
        let save = km
            .hints(Mode::Insert)
            .into_iter()
            .find(|h| h.action == Some(Action::Save))
            .expect("deberia haber hint de Guardar");
        assert_eq!(save.keys, "^W");
    }

    #[test]
    fn render_key_label_de_un_chord() {
        let specs = parse_key_seq("ctrl-k x").unwrap();
        assert_eq!(render_key_label(&specs), "^K X");
        let arrow = parse_key_seq("shift-right").unwrap();
        assert_eq!(render_key_label(&arrow), "⇧→");
    }
}
