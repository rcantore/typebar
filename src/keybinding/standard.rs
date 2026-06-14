//! Preset `standard`: modeless. Siempre se inserta texto, las flechas mueven el
//! cursor. Es el comportamiento esperado por la mayoria de la gente (no hay
//! modos). Default del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{Action, Keymap, Resolve, has_ctrl};
use crate::document::Mode;

pub struct StandardKeymap;

impl StandardKeymap {
    /// Resolucion de una unica tecla (standard no tiene chords).
    fn resolve_key(&self, key: KeyEvent) -> Resolve {
        // Atajos con CONTROL primero (no deben tipearse como texto).
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                KeyCode::Char('q') => Resolve::Action(Action::Quit),
                _ => Resolve::None,
            };
        }
        match key.code {
            KeyCode::Left => Resolve::Action(Action::CursorLeft),
            KeyCode::Right => Resolve::Action(Action::CursorRight),
            KeyCode::Up => Resolve::Action(Action::CursorUp),
            KeyCode::Down => Resolve::Action(Action::CursorDown),
            KeyCode::Enter => Resolve::Action(Action::InsertNewline),
            KeyCode::Backspace => Resolve::Action(Action::Backspace),
            KeyCode::Char(c) => Resolve::Action(Action::InsertChar(c)),
            _ => Resolve::None,
        }
    }
}

impl Keymap for StandardKeymap {
    fn resolve(&self, _mode: Mode, keys: &[KeyEvent]) -> Resolve {
        // Sin chords: solo se resuelven secuencias de una sola tecla.
        if keys.len() != 1 {
            return Resolve::None;
        }
        self.resolve_key(keys[0])
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

#[cfg(test)]
mod tests {
    use super::StandardKeymap;
    use crate::document::Mode;
    use crate::keybinding::test_support::{ctrl, key, resolve1};
    use crate::keybinding::{Action, Keymap, Resolve};
    use ratatui::crossterm::event::KeyCode;

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
    fn standard_no_bindea_esc() {
        let km = StandardKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Esc)),
            Resolve::None
        );
    }

    #[test]
    fn standard_secuencia_larga_es_none() {
        // Standard no tiene chords: cualquier secuencia de mas de una tecla
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
}
