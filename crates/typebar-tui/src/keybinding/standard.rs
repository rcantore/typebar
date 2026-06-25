//! Preset `standard`: modeless. Siempre se inserta texto, las flechas mueven el
//! cursor. Es el comportamiento esperado por la mayoria de la gente (no hay
//! modos). Default del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    Action, Hint, Keymap, Resolve, format_hints, has_ctrl, is_format_prefix, is_view_prefix,
    resolve_format_second, resolve_view_second, view_hints,
};
use crate::document::Mode;

/// Devuelve la accion de extender seleccion si `key` es una flecha con SHIFT, o
/// `None` si no aplica. Las flechas SIN shift siguen el camino normal (que
/// colapsa). Compartido por standard y wordstar.
pub(super) fn shift_arrow_select(key: KeyEvent) -> Option<Action> {
    if !key.modifiers.contains(KeyModifiers::SHIFT) {
        return None;
    }
    match key.code {
        KeyCode::Left => Some(Action::SelectLeft),
        KeyCode::Right => Some(Action::SelectRight),
        KeyCode::Up => Some(Action::SelectUp),
        KeyCode::Down => Some(Action::SelectDown),
        _ => None,
    }
}

pub struct StandardKeymap;

impl StandardKeymap {
    /// Resolucion de una unica tecla. `Ctrl-P` solo queda pendiente (prefijo de
    /// formato, ver `resolve`).
    fn resolve_key(&self, key: KeyEvent) -> Resolve {
        // Shift+flecha extiende la seleccion (las flechas sin shift colapsan).
        if let Some(action) = shift_arrow_select(key) {
            return Resolve::Action(action);
        }
        // Atajos con CONTROL primero (no deben tipearse como texto).
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                KeyCode::Char('q') => Resolve::Action(Action::Quit),
                // Ctrl-Z deshace, Ctrl-Y rehace (convencion moderna).
                KeyCode::Char('z') => Resolve::Action(Action::Undo),
                KeyCode::Char('y') => Resolve::Action(Action::Redo),
                // Ctrl-C copia (yank), Ctrl-V pega (paste): atajos modernos. El
                // raw mode captura Ctrl-C asi que no interrumpe el proceso.
                KeyCode::Char('c') => Resolve::Action(Action::Yank),
                KeyCode::Char('v') => Resolve::Action(Action::Paste),
                // Ctrl-F busca, Ctrl-R reemplaza (convencion moderna).
                KeyCode::Char('f') => Resolve::Action(Action::Search),
                KeyCode::Char('r') => Resolve::Action(Action::Replace),
                // Ctrl-B: negrita directa (memoria muscular). Ctrl-I no se
                // bindea: en la terminal es indistinguible de Tab; por eso la
                // italica va por el chord Ctrl-P I.
                KeyCode::Char('b') => Resolve::Action(Action::ToggleBold),
                // Ctrl-P: prefijo de formato, espera la segunda tecla.
                KeyCode::Char('p') => Resolve::Pending,
                // Ctrl-O: prefijo del submenu "view" (zen, etc.).
                KeyCode::Char('o') => Resolve::Pending,
                // Ctrl-G: abrir el switcher de archivos ("Go to file").
                KeyCode::Char('g') => Resolve::Action(Action::OpenSwitcher),
                // Ctrl-A: abrir la paleta de comandos ("Actions"). Tentativo:
                // remapeable por el usuario.
                KeyCode::Char('a') => Resolve::Action(Action::OpenPalette),
                _ => Resolve::None,
            };
        }
        match key.code {
            KeyCode::Left => Resolve::Action(Action::CursorLeft),
            KeyCode::Right => Resolve::Action(Action::CursorRight),
            KeyCode::Up => Resolve::Action(Action::CursorUp),
            KeyCode::Down => Resolve::Action(Action::CursorDown),
            // Saltos comunes (esperados por usuarios de cualquier editor).
            KeyCode::Home => Resolve::Action(Action::LineStart),
            KeyCode::End => Resolve::Action(Action::LineEnd),
            KeyCode::PageUp => Resolve::Action(Action::PageUp),
            KeyCode::PageDown => Resolve::Action(Action::PageDown),
            KeyCode::Enter => Resolve::Action(Action::InsertNewline),
            KeyCode::Backspace => Resolve::Action(Action::Backspace),
            KeyCode::Char(c) => Resolve::Action(Action::InsertChar(c)),
            _ => Resolve::None,
        }
    }
}

impl Keymap for StandardKeymap {
    fn resolve(&self, _mode: Mode, keys: &[KeyEvent]) -> Resolve {
        match keys {
            [single] => self.resolve_key(*single),
            // Chord de formato: `Ctrl-P` + letra (b/i/c).
            [prefix, second] if is_format_prefix(*prefix) => resolve_format_second(*second),
            // Submenu "view": `Ctrl-O` + letra (z = zen).
            [prefix, second] if is_view_prefix(*prefix) => resolve_view_second(*second),
            _ => Resolve::None,
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

    fn hints(&self, _mode: Mode) -> Vec<Hint> {
        use crate::i18n::{Key, t};
        vec![
            Hint::new(Action::Save, "^S", t(Key::HintSave)),
            Hint::new(Action::OpenSwitcher, "^G", t(Key::HintSwitcher)),
            Hint::new(Action::OpenPalette, "^A", t(Key::HintPalette)),
            Hint::new(Action::Search, "^F", t(Key::HintSearch)),
            Hint::new(Action::Replace, "^R", t(Key::HintReplace)),
            Hint::new(Action::ToggleBold, "^B", t(Key::HintBold)),
            Hint::new(Action::Undo, "^Z", t(Key::HintUndo)),
            Hint::new(Action::Yank, "^C", t(Key::HintYank)),
            Hint::new(Action::Paste, "^V", t(Key::HintPaste)),
            // Prefijo de chord de formato: avisa al usuario que `^P` abre un
            // submenu (negrita/italica/codigo). No es una accion: no se remapea.
            Hint::prefix("^P", t(Key::HintFormatPrefix)),
            Hint::prefix("^O", t(Key::HintViewPrefix)),
            Hint::new(Action::Quit, "^Q", t(Key::HintQuit)),
        ]
    }

    fn chord_hints(&self, _mode: Mode, pending: &[KeyEvent]) -> Vec<Hint> {
        // Chords de standard: el prefijo de formato `Ctrl-P` y el submenu `Ctrl-O`.
        match pending {
            [k] if is_format_prefix(*k) => format_hints(),
            [k] if is_view_prefix(*k) => view_hints(),
            _ => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::StandardKeymap;
    use crate::document::Mode;
    use crate::keybinding::test_support::{ctrl, key, resolve1, shift};
    use crate::keybinding::{Action, Keymap, Resolve};
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn standard_shift_flechas_extienden_seleccion() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Right)),
            Resolve::Action(Action::SelectRight)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Left)),
            Resolve::Action(Action::SelectLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Up)),
            Resolve::Action(Action::SelectUp)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Down)),
            Resolve::Action(Action::SelectDown)
        );
    }

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
            resolve1(&km, Mode::Insert, key(KeyCode::Char('a'))),
            Resolve::Action(Action::InsertChar('a'))
        );
    }

    #[test]
    fn standard_flechas_mueven() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Right)),
            Resolve::Action(Action::CursorRight)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Left)),
            Resolve::Action(Action::CursorLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Up)),
            Resolve::Action(Action::CursorUp)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Down)),
            Resolve::Action(Action::CursorDown)
        );
    }

    #[test]
    fn standard_atajos_de_control() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('s'))),
            Resolve::Action(Action::Save)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('q'))),
            Resolve::Action(Action::Quit)
        );
    }

    #[test]
    fn standard_undo_redo() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('z'))),
            Resolve::Action(Action::Undo)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('y'))),
            Resolve::Action(Action::Redo)
        );
    }

    #[test]
    fn standard_ctrl_c_y_ctrl_v_clipboard() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('c'))),
            Resolve::Action(Action::Yank)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('v'))),
            Resolve::Action(Action::Paste)
        );
    }

    #[test]
    fn standard_no_bindea_esc() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Esc)),
            Resolve::None
        );
    }

    #[test]
    fn standard_secuencia_no_prefijo_es_none() {
        // Una secuencia de dos teclas que no arranca con el prefijo de formato
        // no se resuelve.
        let km = StandardKeymap;
        assert_eq!(
            km.resolve(
                Mode::Insert,
                &[key(KeyCode::Char('a')), key(KeyCode::Char('b'))]
            ),
            Resolve::None
        );
    }

    #[test]
    fn standard_ctrl_p_solo_da_pending() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('p'))),
            Resolve::Pending
        );
    }

    #[test]
    fn standard_chord_formato_togglea() {
        let km = StandardKeymap;
        let p = ctrl(KeyCode::Char('p'));
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('b'))]),
            Resolve::Action(Action::ToggleBold)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('i'))]),
            Resolve::Action(Action::ToggleItalic)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('c'))]),
            Resolve::Action(Action::ToggleCode)
        );
        // Case-insensitive.
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('B'))]),
            Resolve::Action(Action::ToggleBold)
        );
        // Letra no bindeada cancela.
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('z'))]),
            Resolve::None
        );
    }

    #[test]
    fn standard_home_end_pgup_pgdown() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Home)),
            Resolve::Action(Action::LineStart)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::End)),
            Resolve::Action(Action::LineEnd)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::PageUp)),
            Resolve::Action(Action::PageUp)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::PageDown)),
            Resolve::Action(Action::PageDown)
        );
    }

    #[test]
    fn standard_ctrl_o_submenu_view_zen() {
        // `^O` solo queda pendiente (prefijo del submenu view); `^O Z` togglea zen.
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('o'))),
            Resolve::Pending
        );
        let o = ctrl(KeyCode::Char('o'));
        assert_eq!(
            km.resolve(Mode::Insert, &[o, key(KeyCode::Char('z'))]),
            Resolve::Action(Action::ToggleZen)
        );
        // Case-insensitive.
        assert_eq!(
            km.resolve(Mode::Insert, &[o, key(KeyCode::Char('Z'))]),
            Resolve::Action(Action::ToggleZen)
        );
        // Segunda tecla no bindeada cancela.
        assert_eq!(
            km.resolve(Mode::Insert, &[o, key(KeyCode::Char('w'))]),
            Resolve::None
        );
    }

    #[test]
    fn standard_ctrl_b_es_negrita() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('b'))),
            Resolve::Action(Action::ToggleBold)
        );
    }
}
