//! Sistema de keybindings con presets intercambiables.
//!
//! La idea es desacoplar la *tecla fisica* de la *accion semantica*: cada
//! preset implementa el trait `Keymap` y traduce un `KeyEvent` (segun el modo
//! actual) a una `Action`. El loop de `main` aplica la `Action` sobre el
//! `Document` sin saber que preset esta activo.
//!
//! Presets actuales:
//! - `standard`: modeless (siempre se inserta), flechas para moverse. DEFAULT.
//! - `vim`: modal (Normal/Insert), replica el comportamiento Vim minimo.
//!
//! El preset `wordstar` (homenaje, basado en chords tipo `Ctrl-K S`) queda
//! pendiente: requiere soporte de secuencias multi-tecla que todavia no existe.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
    Save,
    Quit,
}

/// Contrato de un preset de teclado.
pub trait Keymap {
    /// Resuelve una tecla en el modo actual a una accion (o None si no esta
    /// bindeada).
    fn resolve(&self, mode: Mode, key: KeyEvent) -> Option<Action>;
    /// Si el preset usa modos (Vim) o es modeless (standard).
    fn is_modal(&self) -> bool;
    /// Modo inicial al abrir el editor.
    fn initial_mode(&self) -> Mode;
    /// Nombre del preset para la status bar.
    fn name(&self) -> &'static str;
}

/// Devuelve true si la tecla trae el modificador CONTROL.
fn has_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Preset modeless: siempre se inserta texto, las flechas mueven el cursor. Es
/// el comportamiento esperado por la mayoria de la gente (no hay modos).
pub struct StandardKeymap;

impl Keymap for StandardKeymap {
    fn resolve(&self, _mode: Mode, key: KeyEvent) -> Option<Action> {
        // Atajos con CONTROL primero (no deben tipearse como texto).
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Some(Action::Save),
                KeyCode::Char('q') => Some(Action::Quit),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Left => Some(Action::CursorLeft),
            KeyCode::Right => Some(Action::CursorRight),
            KeyCode::Up => Some(Action::CursorUp),
            KeyCode::Down => Some(Action::CursorDown),
            KeyCode::Enter => Some(Action::InsertNewline),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char(c) => Some(Action::InsertChar(c)),
            _ => None,
        }
    }

    fn is_modal(&self) -> bool {
        false
    }

    fn initial_mode(&self) -> Mode {
        Mode::Insert
    }

    fn name(&self) -> &'static str {
        "standard"
    }
}

/// Preset modal estilo Vim minimo: Normal para moverse/comandos, Insert para
/// tipear. Replica el comportamiento previo hardcodeado del editor.
pub struct VimKeymap;

impl VimKeymap {
    /// Resolucion de teclas en modo Normal.
    fn resolve_normal(&self, key: KeyEvent) -> Option<Action> {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Some(Action::Save),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => Some(Action::CursorLeft),
            KeyCode::Char('l') | KeyCode::Right => Some(Action::CursorRight),
            KeyCode::Char('k') | KeyCode::Up => Some(Action::CursorUp),
            KeyCode::Char('j') | KeyCode::Down => Some(Action::CursorDown),
            KeyCode::Char('i') => Some(Action::EnterInsert),
            KeyCode::Char('a') => Some(Action::InsertAfter),
            KeyCode::Char('x') => Some(Action::DeleteChar),
            KeyCode::Char('o') => Some(Action::OpenLineBelow),
            KeyCode::Char('q') => Some(Action::Quit),
            _ => None,
        }
    }

    /// Resolucion de teclas en modo Insert.
    fn resolve_insert(&self, key: KeyEvent) -> Option<Action> {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Some(Action::Save),
                _ => None,
            };
        }
        match key.code {
            KeyCode::Esc => Some(Action::EnterNormal),
            KeyCode::Enter => Some(Action::InsertNewline),
            KeyCode::Backspace => Some(Action::Backspace),
            KeyCode::Char(c) => Some(Action::InsertChar(c)),
            _ => None,
        }
    }
}

impl Keymap for VimKeymap {
    fn resolve(&self, mode: Mode, key: KeyEvent) -> Option<Action> {
        match mode {
            Mode::Normal => self.resolve_normal(key),
            Mode::Insert => self.resolve_insert(key),
        }
    }

    fn is_modal(&self) -> bool {
        true
    }

    fn initial_mode(&self) -> Mode {
        Mode::Normal
    }

    fn name(&self) -> &'static str {
        "vim"
    }
}

/// Construye el preset segun su nombre. Si no matchea ninguno conocido, cae al
/// default `standard` (modeless).
pub fn keymap_from_name(name: &str) -> Box<dyn Keymap> {
    match name {
        "vim" => Box::new(VimKeymap),
        _ => Box::new(StandardKeymap),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper para armar un KeyEvent simple sin modificadores.
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Helper para armar un KeyEvent con CONTROL.
    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    // --- StandardKeymap ----------------------------------------------------

    #[test]
    fn standard_es_modeless_y_arranca_en_insert() {
        let km = StandardKeymap;
        assert!(!km.is_modal());
        assert_eq!(km.initial_mode(), Mode::Insert);
        assert_eq!(km.name(), "standard");
    }

    #[test]
    fn standard_inserta_chars() {
        let km = StandardKeymap;
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Char('a'))),
            Some(Action::InsertChar('a'))
        );
    }

    #[test]
    fn standard_flechas_mueven() {
        let km = StandardKeymap;
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Right)),
            Some(Action::CursorRight)
        );
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Left)),
            Some(Action::CursorLeft)
        );
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Up)),
            Some(Action::CursorUp)
        );
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Down)),
            Some(Action::CursorDown)
        );
    }

    #[test]
    fn standard_atajos_de_control() {
        let km = StandardKeymap;
        assert_eq!(
            km.resolve(Mode::Insert, ctrl(KeyCode::Char('s'))),
            Some(Action::Save)
        );
        assert_eq!(
            km.resolve(Mode::Insert, ctrl(KeyCode::Char('q'))),
            Some(Action::Quit)
        );
    }

    #[test]
    fn standard_no_bindea_esc() {
        let km = StandardKeymap;
        assert_eq!(km.resolve(Mode::Insert, key(KeyCode::Esc)), None);
    }

    // --- VimKeymap ---------------------------------------------------------

    #[test]
    fn vim_es_modal_y_arranca_en_normal() {
        let km = VimKeymap;
        assert!(km.is_modal());
        assert_eq!(km.initial_mode(), Mode::Normal);
        assert_eq!(km.name(), "vim");
    }

    #[test]
    fn vim_normal_movimiento_y_modos() {
        let km = VimKeymap;
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('h'))),
            Some(Action::CursorLeft)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('l'))),
            Some(Action::CursorRight)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('i'))),
            Some(Action::EnterInsert)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('a'))),
            Some(Action::InsertAfter)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('o'))),
            Some(Action::OpenLineBelow)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('x'))),
            Some(Action::DeleteChar)
        );
        assert_eq!(
            km.resolve(Mode::Normal, key(KeyCode::Char('q'))),
            Some(Action::Quit)
        );
    }

    #[test]
    fn vim_normal_ctrl_s_guarda() {
        let km = VimKeymap;
        assert_eq!(
            km.resolve(Mode::Normal, ctrl(KeyCode::Char('s'))),
            Some(Action::Save)
        );
    }

    #[test]
    fn vim_normal_no_inserta_texto() {
        // En Normal, una letra sin binding (ej 'z') no inserta nada.
        let km = VimKeymap;
        assert_eq!(km.resolve(Mode::Normal, key(KeyCode::Char('z'))), None);
    }

    #[test]
    fn vim_insert_esc_vuelve_a_normal() {
        let km = VimKeymap;
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Esc)),
            Some(Action::EnterNormal)
        );
    }

    #[test]
    fn vim_insert_inserta_chars() {
        let km = VimKeymap;
        assert_eq!(
            km.resolve(Mode::Insert, key(KeyCode::Char('a'))),
            Some(Action::InsertChar('a'))
        );
    }

    // --- Seleccion de preset ----------------------------------------------

    #[test]
    fn nombre_desconocido_cae_a_standard() {
        assert_eq!(keymap_from_name("loquesea").name(), "standard");
        assert_eq!(keymap_from_name("vim").name(), "vim");
        assert_eq!(keymap_from_name("standard").name(), "standard");
    }
}
