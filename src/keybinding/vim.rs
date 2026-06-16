//! Preset `vim`: modal estilo Vim minimo. Normal para moverse/comandos, Insert
//! para tipear. Replica el comportamiento previo hardcodeado del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{Action, Hint, Keymap, Resolve, has_ctrl, is_format_prefix, resolve_format_second};
use crate::document::Mode;

pub struct VimKeymap;

impl VimKeymap {
    /// Resolucion de teclas en modo Normal.
    fn resolve_normal(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                // Ctrl-R: rehacer (lo canonico de Vim).
                KeyCode::Char('r') => Resolve::Action(Action::Redo),
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
            KeyCode::Char('v') => Resolve::Action(Action::EnterVisual),
            KeyCode::Char('x') => Resolve::Action(Action::DeleteChar),
            KeyCode::Char('o') => Resolve::Action(Action::OpenLineBelow),
            // `p` pega el portapapeles interno (canonico de Vim).
            KeyCode::Char('p') => Resolve::Action(Action::Paste),
            // `u` deshace (canonico de Vim; en Insert no se bindea).
            KeyCode::Char('u') => Resolve::Action(Action::Undo),
            // `/` busca (canonico de Vim). El reemplazo (`:s`) no se modela; se
            // puede bindear a gusto con los keybindings remapeables.
            KeyCode::Char('/') => Resolve::Action(Action::Search),
            KeyCode::Char('q') => Resolve::Action(Action::Quit),
            _ => Resolve::None,
        }
    }

    /// Resolucion de teclas en modo Visual: las teclas de movimiento extienden
    /// la seleccion, `x`/`d` la borran, `Esc` vuelve a Normal. El chord de
    /// formato `Ctrl-P` se maneja aparte (ver `resolve`) y opera sobre la
    /// seleccion.
    fn resolve_visual(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                KeyCode::Char('s') => Resolve::Action(Action::Save),
                KeyCode::Char('p') => Resolve::Pending,
                _ => Resolve::None,
            };
        }
        match key.code {
            KeyCode::Char('h') | KeyCode::Left => Resolve::Action(Action::SelectLeft),
            KeyCode::Char('l') | KeyCode::Right => Resolve::Action(Action::SelectRight),
            KeyCode::Char('k') | KeyCode::Up => Resolve::Action(Action::SelectUp),
            KeyCode::Char('j') | KeyCode::Down => Resolve::Action(Action::SelectDown),
            KeyCode::Char('x') | KeyCode::Char('d') => Resolve::Action(Action::DeleteSelection),
            // `y` copia la seleccion al portapapeles interno (yank de Vim).
            KeyCode::Char('y') => Resolve::Action(Action::Yank),
            KeyCode::Esc => Resolve::Action(Action::EnterNormal),
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
                Mode::Visual => self.resolve_visual(*single),
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

    fn hints(&self, mode: Mode) -> Vec<Hint> {
        match mode {
            Mode::Normal => vec![
                Hint::new(Action::EnterInsert, "i", "Insertar"),
                Hint::new(Action::EnterVisual, "v", "Visual"),
                Hint::new(Action::Search, "/", "Buscar"),
                Hint::new(Action::Undo, "u", "Deshacer"),
                Hint::new(Action::Paste, "p", "Pegar"),
                Hint::new(Action::Save, "^S", "Guardar"),
                Hint::new(Action::Quit, "q", "Salir"),
            ],
            Mode::Insert => vec![
                Hint::new(Action::EnterNormal, "Esc", "Normal"),
                Hint::new(Action::Save, "^S", "Guardar"),
            ],
            Mode::Visual => vec![
                Hint::new(Action::Yank, "y", "Copiar"),
                Hint::new(Action::DeleteSelection, "x", "Borrar"),
                Hint::new(Action::EnterNormal, "Esc", "Normal"),
            ],
        }
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
    fn vim_normal_undo_redo() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('u'))),
            Resolve::Action(Action::Undo)
        );
        assert_eq!(
            resolve1(&km, Mode::Normal, ctrl(KeyCode::Char('r'))),
            Resolve::Action(Action::Redo)
        );
    }

    #[test]
    fn vim_normal_v_entra_a_visual() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('v'))),
            Resolve::Action(Action::EnterVisual)
        );
    }

    #[test]
    fn vim_visual_movimiento_extiende_seleccion() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('h'))),
            Resolve::Action(Action::SelectLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('l'))),
            Resolve::Action(Action::SelectRight)
        );
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('k'))),
            Resolve::Action(Action::SelectUp)
        );
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('j'))),
            Resolve::Action(Action::SelectDown)
        );
    }

    #[test]
    fn vim_visual_esc_vuelve_a_normal() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Esc)),
            Resolve::Action(Action::EnterNormal)
        );
    }

    #[test]
    fn vim_visual_d_y_x_borran_seleccion() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('d'))),
            Resolve::Action(Action::DeleteSelection)
        );
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('x'))),
            Resolve::Action(Action::DeleteSelection)
        );
    }

    #[test]
    fn vim_normal_p_pega() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('p'))),
            Resolve::Action(Action::Paste)
        );
    }

    #[test]
    fn vim_visual_y_copia_seleccion() {
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Visual, key(KeyCode::Char('y'))),
            Resolve::Action(Action::Yank)
        );
    }

    #[test]
    fn vim_chord_formato_en_cualquier_modo() {
        // El chord de formato es agnostico al modo: anda igual en Normal, Insert
        // y Visual (en Visual opera sobre la seleccion).
        let km = VimKeymap;
        let p = ctrl(KeyCode::Char('p'));
        for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
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
