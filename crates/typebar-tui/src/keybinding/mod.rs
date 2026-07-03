//! Sistema de keybindings con presets intercambiables.
//!
//! La idea es desacoplar la *tecla fisica* de la *accion semantica*: cada
//! preset implementa el trait `Keymap` y traduce una *secuencia* de `KeyEvent`
//! (segun el modo actual) a una `Action`. El loop de `main` aplica la `Action`
//! sobre el `Document` sin saber que preset esta activo.
//!
//! Para soportar **chords** (secuencias multi-tecla tipo `Ctrl-K S`) el trait
//! recibe el buffer de teclas pendientes y devuelve un `Resolve`: una accion
//! completa, `Pending` (la secuencia es prefijo de un chord, esperar mas
//! teclas) o `None` (no bindeada). El loop acumula teclas hasta resolver.
//!
//! Presets (uno por submodulo):
//! - `standard`: modeless (siempre se inserta), flechas para moverse. DEFAULT.
//! - `vim`: modal (Normal/Insert), replica el comportamiento Vim minimo.
//! - `wordstar`: modeless con chords, homenaje al editor clasico (diamante de
//!   navegacion `Ctrl-E/X/S/D` + chords `Ctrl-K`/`Ctrl-Q`).
//!
//! Encima de cualquier preset, el usuario puede definir overrides en
//! `config.toml` (ver `custom`): un `CustomKeymap` envuelve el preset base y
//! antepone los bindings del usuario, cayendo al preset cuando una secuencia no
//! esta overrideada.

mod custom;
mod standard;
mod vim;
mod wordstar;

pub use custom::{Binding, CustomKeymap, parse_binding};
pub use standard::StandardKeymap;
pub use vim::VimKeymap;
pub use wordstar::WordstarKeymap;

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

use typebar_core::document::Mode;

/// Operaciones semanticas que un keymap puede producir a partir de una tecla.
/// El loop principal las traduce a llamadas concretas sobre el `Document`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    CursorLeft,
    CursorRight,
    CursorUp,
    CursorDown,
    InsertChar(char),
    InsertNewline,
    Backspace,
    DeleteChar,
    EnterInsert,
    EnterNormal,
    /// La `a` de Vim: mover un grafema a la derecha y entrar a Insert.
    InsertAfter,
    /// La `o` de Vim: abrir una linea debajo y entrar a Insert.
    OpenLineBelow,
    /// Inicio de la linea actual (col 0).
    LineStart,
    /// Fin de la linea actual.
    LineEnd,
    /// Inicio del documento (linea 0, col 0).
    DocStart,
    /// Fin del documento (ultima linea con contenido).
    DocEnd,
    /// Avanzar el cursor una pagina (~ alto del viewport) hacia arriba.
    PageUp,
    /// Avanzar el cursor una pagina (~ alto del viewport) hacia abajo.
    PageDown,
    Save,
    /// Guardar y salir (el `Ctrl-K D`/`Ctrl-K X` de WordStar).
    SaveAndQuit,
    Quit,
    /// Togglear negrita (`**`) sobre la palabra bajo el cursor.
    ToggleBold,
    /// Togglear italica (`*`) sobre la palabra bajo el cursor.
    ToggleItalic,
    /// Togglear codigo inline (`` ` ``) sobre la palabra bajo el cursor.
    ToggleCode,
    /// Entrar al modo Visual de Vim (empieza una seleccion en el cursor).
    EnterVisual,
    /// Extender la seleccion un grafema a la izquierda/derecha o una linea
    /// arriba/abajo (Visual en Vim, Shift+flechas en presets modeless).
    SelectLeft,
    SelectRight,
    SelectUp,
    SelectDown,
    /// Borrar el rango seleccionado.
    DeleteSelection,
    /// Deshacer la ultima edicion.
    Undo,
    /// Rehacer la ultima edicion deshecha.
    Redo,
    /// Copiar la seleccion al portapapeles interno (yank).
    Yank,
    /// Pegar el portapapeles interno en el cursor (paste).
    Paste,
    /// Abrir el overlay de busqueda incremental.
    Search,
    /// Abrir el overlay de buscar y reemplazar.
    Replace,
    /// Togglear el modo zen/focus: oculta todo el chrome (borde, toolbar, status
    /// bar) para dejar solo el texto. Es estado de la vista del loop, no del
    /// documento; building block del modo whitepaper.
    ToggleZen,
    /// Abrir el switcher de archivos (fuzzy finder): lista los archivos del
    /// proyecto y los buffers abiertos para abrir/cambiar de uno. Opera a nivel
    /// workspace, no del documento; lo maneja `run`.
    OpenSwitcher,
    /// Abrir la paleta de comandos (estilo M-x): un overlay que fuzzy-filtra los
    /// comandos del editor por nombre y ejecuta el elegido. Como el switcher,
    /// opera a nivel del loop (`run`): al aceptar, despacha el `Action` elegido
    /// por el mismo camino que el keymap.
    OpenPalette,
    /// Togglear el theme claro (Catppuccin Latte) en runtime, desde el submenu
    /// "view" (`^O L` / `z l`). Es estado de la vista del loop, no del documento;
    /// alterna entre el theme configurado (oscuro) y Latte (claro).
    ToggleLightTheme,
    /// Crear un buffer nuevo y vacio ("new file") y enfocarlo. Opera a nivel
    /// workspace, no del documento; lo maneja `run`.
    NewBuffer,
    /// Enfocar el buffer siguiente / anterior (cycle con wraparound). Nivel
    /// workspace; lo maneja `run`.
    NextBuffer,
    PrevBuffer,
    /// Cerrar el buffer activo. Si tiene cambios sin guardar, `run` abre un prompt
    /// de confirmacion antes de cerrar; si no, cierra directo. Cerrar el unico
    /// buffer lo reemplaza por uno vacio (nunca quedan cero tabs). Nivel workspace;
    /// lo maneja `run`.
    CloseBuffer,
    /// Togglear el modo whitepaper desde el submenu "view" (`^O W` / `z w`):
    /// orquesta zen + theme claro + columna de ancho fijo centrada, para la
    /// sensacion "hoja de papel"/typewriter. Estado de la vista del loop, no del
    /// documento; lo maneja `run`. Construido sobre zen + el theme Latte.
    ToggleWhitepaper,
    /// Exportar el buffer ACTUAL a HTML standalone sin salir del editor,
    /// mostrando el resultado en la status (flash). A diferencia del flag CLI
    /// `--export-html`, exporta el contenido en memoria (cambios sin guardar
    /// incluidos). Lo maneja `run` (escribe el archivo y setea el flash).
    ExportHtml,
}

/// Resultado de resolver una secuencia de teclas contra un keymap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolve {
    /// La secuencia completa una accion.
    Action(Action),
    /// La secuencia es prefijo de un chord; esperar mas teclas.
    Pending,
    /// La secuencia no esta bindeada (cancela el buffer pendiente).
    None,
}

/// Un atajo a mostrar en la barra de atajos (toolbar estilo WordStar/Norton
/// Commander): la combinacion de teclas y que hace. `action` permite que el
/// overlay de keybindings remapeados reescriba `keys` con la tecla configurada
/// por el usuario en vez de la del preset; un hint *estructural* (prefijo de
/// chord como `^P`, sin una accion concreta) usa `None` y no se remapea.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    /// La accion que dispara (para reflejar remapeos del usuario). `None` en
    /// hints estructurales que no mapean a una accion unica (ej un prefijo de
    /// chord como `^P` para formato).
    pub action: Option<Action>,
    /// Etiqueta de la(s) tecla(s), ej `^S` o `^K X`.
    pub keys: String,
    /// Descripcion corta de la accion, ej `Guardar`.
    pub label: &'static str,
}

impl Hint {
    /// Hint comun: una accion concreta con su atajo.
    fn new(action: Action, keys: &str, label: &'static str) -> Self {
        Hint {
            action: Some(action),
            keys: keys.to_string(),
            label,
        }
    }

    /// Hint estructural: un prefijo de chord (ej `^P` -> Formato) que no
    /// dispara una accion concreta por si solo. No participa del remapeo.
    fn prefix(keys: &str, label: &'static str) -> Self {
        Hint {
            action: None,
            keys: keys.to_string(),
            label,
        }
    }
}

/// Contrato de un preset de teclado.
pub trait Keymap {
    /// Resuelve una secuencia de teclas en el modo actual: accion completa,
    /// `Pending` (prefijo de un chord) o `None` (no bindeada).
    fn resolve(&self, mode: Mode, keys: &[KeyEvent]) -> Resolve;
    /// Si el preset usa modos (Vim) o es modeless (standard).
    fn is_modal(&self) -> bool;
    /// Modo inicial al abrir el editor.
    fn initial_mode(&self) -> Mode;
    /// Nombre del preset para la status bar.
    fn name(&self) -> &'static str;
    /// Atajos a mostrar en la barra de atajos para el modo dado, en orden de
    /// aparicion. Cada preset expone los suyos mas utiles.
    fn hints(&self, mode: Mode) -> Vec<Hint>;
    /// Atajos de CONTINUACION de un chord en curso: dado el prefijo ya tipeado
    /// (`pending`), que acciones se alcanzan con la proxima tecla. La `keys` de
    /// cada hint es solo esa proxima tecla (el prefijo ya se muestra aparte).
    /// Default vacio: los presets sin chords no aportan continuaciones.
    fn chord_hints(&self, _mode: Mode, _pending: &[KeyEvent]) -> Vec<Hint> {
        Vec::new()
    }
}

use ratatui::crossterm::event::KeyCode;

/// Devuelve true si la tecla trae el modificador CONTROL. Compartido por los
/// submodulos de presets.
fn has_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Comandos de workspace uniformes en los tres presets, todos bajo CONTROL:
/// `^G` abre el switcher de archivos ("Go to file"), `^A` la paleta de comandos
/// ("Actions"), `^N` crea un buffer nuevo, `^W` cierra el activo, y
/// `Ctrl-PageDown`/`Ctrl-PageUp` ciclan al buffer siguiente/anterior (estilo tabs
/// de browser). Devuelve el `Action` correspondiente, o `None` si `key` no trae
/// CONTROL o no es uno de estos atajos. Cada preset lo consulta PRIMERO en su rama
/// ctrl para no repetir estas ramas identicas; las teclas idiosincraticas de cada
/// preset siguen aparte.
fn workspace_ctrl_command(key: KeyEvent) -> Option<Action> {
    if !has_ctrl(key) {
        return None;
    }
    match key.code {
        KeyCode::Char('g') => Some(Action::OpenSwitcher),
        KeyCode::Char('a') => Some(Action::OpenPalette),
        KeyCode::Char('n') => Some(Action::NewBuffer),
        KeyCode::Char('w') => Some(Action::CloseBuffer),
        KeyCode::PageDown => Some(Action::NextBuffer),
        KeyCode::PageUp => Some(Action::PrevBuffer),
        _ => None,
    }
}

/// Devuelve true si `key` es el prefijo de formato `Ctrl-P`. El chord `Ctrl-P`
/// seguido de una letra (`b`/`i`/`c`) togglea negrita/italica/codigo, uniforme
/// en los tres presets.
fn is_format_prefix(key: KeyEvent) -> bool {
    has_ctrl(key) && matches!(key.code, KeyCode::Char('p'))
}

/// Devuelve true si `key` es el prefijo del submenu "view" `Ctrl-O` (standard y
/// wordstar; homenaje al *Onscreen format* del WordStar real). En vim el submenu
/// cuelga de `z` y se detecta aparte. Ver `resolve_view_second`/`view_hints`.
fn is_view_prefix(key: KeyEvent) -> bool {
    has_ctrl(key) && matches!(key.code, KeyCode::Char('o'))
}

/// Resuelve la SEGUNDA tecla de un chord de formato `Ctrl-P` + letra
/// (case-insensitive): `b` -> negrita, `i` -> italica, `c` -> codigo. Cualquier
/// otra tecla cancela (`None`). Compartido por los tres presets.
fn resolve_format_second(second: KeyEvent) -> Resolve {
    let letter = match second.code {
        KeyCode::Char(c) => c.to_ascii_lowercase(),
        _ => return Resolve::None,
    };
    match letter {
        'b' => Resolve::Action(Action::ToggleBold),
        'i' => Resolve::Action(Action::ToggleItalic),
        'c' => Resolve::Action(Action::ToggleCode),
        _ => Resolve::None,
    }
}

/// Hints de continuacion del chord de formato `Ctrl-P` + letra, compartido por
/// los tres presets (igual que `resolve_format_second` comparte su resolucion).
fn format_hints() -> Vec<Hint> {
    use typebar_core::i18n::{Key, t};
    vec![
        Hint::new(Action::ToggleBold, "B", t(Key::HintBold)),
        Hint::new(Action::ToggleItalic, "I", t(Key::HintItalic)),
        Hint::new(Action::ToggleCode, "C", t(Key::HintCode)),
    ]
}

/// Submenu "view": prefijo de toggles de vista. El prefijo fisico difiere por
/// preset (homenaje a cada idioma: `Ctrl-O` —el *Onscreen format* de WordStar—
/// en standard/wordstar, `z` —el prefijo de comandos de vista de Vim— en vim),
/// pero la SEGUNDA tecla y los hints son compartidos para que la familia crezca
/// uniforme (`Z` -> zen, `L` -> theme light, `W` -> whitepaper).
///
/// Resuelve la segunda tecla del submenu (case-insensitive). Cualquier otra
/// cancela (`None`).
fn resolve_view_second(second: KeyEvent) -> Resolve {
    let letter = match second.code {
        KeyCode::Char(c) => c.to_ascii_lowercase(),
        _ => return Resolve::None,
    };
    match letter {
        'z' => Resolve::Action(Action::ToggleZen),
        'l' => Resolve::Action(Action::ToggleLightTheme),
        'w' => Resolve::Action(Action::ToggleWhitepaper),
        _ => Resolve::None,
    }
}

/// Hints de continuacion del submenu "view", compartido por los tres presets
/// (igual que `format_hints`). La `keys` es solo la segunda tecla.
fn view_hints() -> Vec<Hint> {
    use typebar_core::i18n::{Key, t};
    vec![
        Hint::new(Action::ToggleZen, "Z", t(Key::HintZen)),
        Hint::new(Action::ToggleLightTheme, "L", t(Key::HintLight)),
        Hint::new(Action::ToggleWhitepaper, "W", t(Key::HintWhitepaper)),
    ]
}

/// Construye el preset segun su nombre. Si no matchea ninguno conocido, cae al
/// default `standard` (modeless).
pub fn keymap_from_name(name: &str) -> Box<dyn Keymap> {
    match name {
        "vim" => Box::new(VimKeymap),
        "wordstar" => Box::new(WordstarKeymap),
        _ => Box::new(StandardKeymap),
    }
}

/// Helpers compartidos por los tests de los submodulos de presets.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{Keymap, Resolve};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use typebar_core::document::Mode;

    /// KeyEvent simple sin modificadores.
    pub(crate) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// KeyEvent con CONTROL.
    pub(crate) fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// KeyEvent con SHIFT (para las Shift+flechas de los presets modeless).
    pub(crate) fn shift(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::SHIFT)
    }

    /// Resuelve una secuencia de una sola tecla (atajo comun en los tests).
    pub(crate) fn resolve1(km: &dyn Keymap, mode: Mode, k: KeyEvent) -> Resolve {
        km.resolve(mode, &[k])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nombre_desconocido_cae_a_standard() {
        assert_eq!(keymap_from_name("loquesea").name(), "standard");
        assert_eq!(keymap_from_name("vim").name(), "vim");
        assert_eq!(keymap_from_name("standard").name(), "standard");
        assert_eq!(keymap_from_name("wordstar").name(), "wordstar");
    }
}
