//! Preset `vim`: modal estilo Vim minimo. Normal para moverse/comandos, Insert
//! para tipear. Replica el comportamiento previo hardcodeado del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{Action, Keymap, Resolve, has_ctrl, is_format_prefix, resolve_format_second};
use crate::document::Mode;

pub struct VimKeymap;

impl VimKeymap {
    /// Resolucion de teclas en modo Normal.
    fn resolve_normal(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                // Ctrl-P: prefijo de formato (agnostico al modo).
                KeyCode::Char('p') => Resolve::Pending,
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
                // Ctrl-P: prefijo de formato (agnostico al modo).
                KeyCode::Char('p') => Resolve::Pending,
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
        match keys {
            // Chord de formato `Ctrl-P` + letra: funciona en CUALQUIER modo (el
            // formato es agnostico al modo Vim).
            [prefix, second] if is_format_prefix(*prefix) => resolve_format_second(*second),
            [single] => match mode {
                Mode::Normal => self.resolve_normal(*single),
                Mode::Insert => self.resolve_insert(*single),
            },
            _ => Resolve::None,
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

    #[test]
    fn vim_ctrl_p_pendiente_en_ambos_modos() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, ctrl(KeyCode::Char('p'))),
            Resolve::Pending
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('p'))),
            Resolve::Pending
        );
    }

    #[test]
    fn vim_chord_formato_en_cualquier_modo() {
        // El chord de formato es agnostico al modo: anda igual en Normal e Insert.
        let km = VimKeymap;
        let p = ctrl(KeyCode::Char('p'));
        for mode in [Mode::Normal, Mode::Insert] {
            assert_eq!(
                km.resolve(mode, &[p, key(KeyCode::Char('b'))]),
                Resolve::Action(Action::ToggleBold)
            );
            assert_eq!(
                km.resolve(mode, &[p, key(KeyCode::Char('i'))]),
                Resolve::Action(Action::ToggleItalic)
            );
            assert_eq!(
                km.resolve(mode, &[p, key(KeyCode::Char('c'))]),
                Resolve::Action(Action::ToggleCode)
            );
        }
    }
}
