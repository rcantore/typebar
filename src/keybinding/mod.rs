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

mod standard;
mod vim;
mod wordstar;

pub use standard::StandardKeymap;
pub use vim::VimKeymap;
pub use wordstar::WordstarKeymap;

use ratatui::crossterm::event::{KeyEvent, KeyModifiers};

use crate::document::Mode;

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
}

use ratatui::crossterm::event::KeyCode;

/// Devuelve true si la tecla trae el modificador CONTROL. Compartido por los
/// submodulos de presets.
fn has_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Devuelve true si `key` es el prefijo de formato `Ctrl-P`. El chord `Ctrl-P`
/// seguido de una letra (`b`/`i`/`c`) togglea negrita/italica/codigo, uniforme
/// en los tres presets.
fn is_format_prefix(key: KeyEvent) -> bool {
    has_ctrl(key) && matches!(key.code, KeyCode::Char('p'))
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
    use crate::document::Mode;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    /// KeyEvent simple sin modificadores.
    pub(crate) fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// KeyEvent con CONTROL.
    pub(crate) fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
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
