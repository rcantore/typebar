//! Preset `vim`: modal estilo Vim minimo. Normal para moverse/comandos, Insert
//! para tipear. Replica el comportamiento previo hardcodeado del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{Action, Keymap, Resolve, has_ctrl};
use crate::document::Mode;

pub struct VimKeymap;

impl VimKeymap {
    /// Resolucion de teclas en modo Normal.
    fn resolve_normal(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                _ => Resolve::None,
            };
        }
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => Resolve::Action(Action::CursorLeft),
            KeyCode::Char('l') | KeyCode::Right => Resolve::Action(Action::CursorRight),
            KeyCode::Char('k') | KeyCode::Up => Resolve::Action(Action::CursorUp),
            KeyCode::Char('j') | KeyCode::Down => Resolve::Action(Action::CursorDown),
            KeyCode::Char('i') => Resolve::Action(Action::EnterInsert),
            KeyCode::Char('a') => Resolve::Action(Action::InsertAfter),
            KeyCode::Char('x') => Resolve::Action(Action::DeleteChar),
            KeyCode::Char('o') => Resolve::Action(Action::OpenLineBelow),
            KeyCode::Char('q') => Resolve::Action(Action::Quit),
            _ => Resolve::None,
        }
    }

    /// Resolucion de teclas en modo Insert.
    fn resolve_insert(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                _ => Resolve::None,
            };
        }
        match key.code {
            KeyCode::Esc => Resolve::Action(Action::EnterNormal),
            KeyCode::Enter => Resolve::Action(Action::InsertNewline),
            KeyCode::Backspace => Resolve::Action(Action::Backspace),
            KeyCode::Char(c) => Resolve::Action(Action::InsertChar(c)),
            _ => Resolve::None,
        }
    }
}

impl Keymap for VimKeymap {
    fn resolve(&self, mode: Mode, keys: &[KeyEvent]) -> Resolve {
        // Sin chords: solo se resuelven secuencias de una sola tecla.
        if keys.len() != 1 {
            return Resolve::None;
        }
        match mode {
            Mode::Normal => self.resolve_normal(keys[0]),
            Mode::Insert => self.resolve_insert(keys[0]),
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

#[cfg(test)]
mod tests {
    use super::VimKeymap;
    use crate::document::Mode;
    use crate::keybinding::test_support::{ctrl, key, resolve1};
    use crate::keybinding::{Action, Keymap, Resolve};
    use ratatui::crossterm::event::KeyCode;

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
            resolve1(&km, Mode::Normal, key(KeyCode::Char('h'))),
            Resolve::Action(Action::CursorLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('l'))),
            Resolve::Action(Action::CursorRight)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('i'))),
            Resolve::Action(Action::EnterInsert)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('a'))),
            Resolve::Action(Action::InsertAfter)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('o'))),
            Resolve::Action(Action::OpenLineBelow)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('x'))),
            Resolve::Action(Action::DeleteChar)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('q'))),
            Resolve::Action(Action::Quit)
        );
    }

    #[test]
    fn vim_normal_ctrl_s_guarda() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, ctrl(KeyCode::Char('s'))),
            Resolve::Action(Action::Save)
        );
    }

    #[test]
    fn vim_normal_no_inserta_texto() {
        // En Normal, una letra sin binding (ej 'z') no inserta nada.
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('z'))),
            Resolve::None
        );
    }

    #[test]
    fn vim_insert_esc_vuelve_a_normal() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Esc)),
            Resolve::Action(Action::EnterNormal)
        );
    }

    #[test]
    fn vim_insert_inserta_chars() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Char('a'))),
            Resolve::Action(Action::InsertChar('a'))
        );
    }
}
