//! Preset `vim`: modal estilo Vim minimo. Normal para moverse/comandos, Insert
//! para tipear. Replica el comportamiento previo hardcodeado del editor.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{
    Action, Hint, Keymap, Resolve, format_hints, has_ctrl, is_format_prefix, resolve_format_second,
    resolve_view_second, view_hints,
};
use crate::document::Mode;

/// True si `key` es el prefijo del submenu "view" de Vim: la tecla `z` sin
/// CONTROL. En Vim `z` ya es el prefijo de comandos de vista (scroll/folds), asi
/// que el zen/focus cuelga ahi (`z z`). Solo aplica en Normal (en Insert `z` se
/// tipea); ver `resolve`. Comparte la segunda tecla con standard/wordstar via
/// `resolve_view_second`.
fn is_vim_view_prefix(key: KeyEvent) -> bool {
    !has_ctrl(key) && matches!(key.code, KeyCode::Char('z'))
}

/// Teclas "modernas" de salto (Home/End/PgUp/PgDn): aunque Vim canonico usa
/// `0`/`$`/`Ctrl-B`/`Ctrl-F`, en Insert y Normal aceptamos las teclas fisicas
/// para que la edicion cotidiana no sorprenda. En Visual no estan bindeadas
/// para no introducir SelectLineStart/etc; usar los movimientos canonicos.
fn modern_motion(key: KeyEvent) -> Option<Action> {
    match key.code {
        KeyCode::Home => Some(Action::LineStart),
        KeyCode::End => Some(Action::LineEnd),
        KeyCode::PageUp => Some(Action::PageUp),
        KeyCode::PageDown => Some(Action::PageDown),
        _ => None,
    }
}

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
                // Ctrl-G: abrir el switcher de archivos ("Go to file").
                KeyCode::Char('g') => Resolve::Action(Action::OpenSwitcher),
                // Ctrl-A: abrir la paleta de comandos ("Actions"). Tentativo.
                KeyCode::Char('a') => Resolve::Action(Action::OpenPalette),
                // Ctrl-N: nuevo archivo (buffer vacio).
                KeyCode::Char('n') => Resolve::Action(Action::NewBuffer),
                // Ctrl-PageDown/Up: cambiar de buffer (estilo tabs de browser).
                KeyCode::PageDown => Resolve::Action(Action::NextBuffer),
                KeyCode::PageUp => Resolve::Action(Action::PrevBuffer),
                _ => Resolve::None,
            };
        }
        if let Some(action) = modern_motion(key) {
            return Resolve::Action(action);
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
            // `z`: prefijo del submenu "view" (zen, etc.). Espera la segunda
            // tecla (ver `resolve`). Idiomatico: en Vim `z` es vista/scroll.
            KeyCode::Char('z') => Resolve::Pending,
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
                // Ctrl-G: abrir el switcher de archivos ("Go to file").
                KeyCode::Char('g') => Resolve::Action(Action::OpenSwitcher),
                // Ctrl-A: abrir la paleta de comandos ("Actions"). Tentativo.
                KeyCode::Char('a') => Resolve::Action(Action::OpenPalette),
                // Ctrl-N: nuevo archivo (buffer vacio).
                KeyCode::Char('n') => Resolve::Action(Action::NewBuffer),
                // Ctrl-PageDown/Up: cambiar de buffer (estilo tabs de browser).
                KeyCode::PageDown => Resolve::Action(Action::NextBuffer),
                KeyCode::PageUp => Resolve::Action(Action::PrevBuffer),
                _ => Resolve::None,
            };
        }
        if let Some(action) = modern_motion(key) {
            return Resolve::Action(action);
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
            // Submenu "view" `z` + letra (z = zen): solo en Normal (en Insert la
            // `z` se tipea como texto).
            [prefix, second] if mode == Mode::Normal && is_vim_view_prefix(*prefix) => {
                resolve_view_second(*second)
            }
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
        use crate::i18n::{Key, t};
        match mode {
            Mode::Normal => vec![
                Hint::new(Action::NewBuffer, "^N", t(Key::HintNew)),
                Hint::new(Action::EnterInsert, "i", t(Key::HintInsert)),
                Hint::new(Action::EnterVisual, "v", t(Key::HintVisual)),
                Hint::new(Action::OpenSwitcher, "^G", t(Key::HintSwitcher)),
                Hint::new(Action::OpenPalette, "^A", t(Key::HintPalette)),
                Hint::new(Action::Search, "/", t(Key::HintSearch)),
                Hint::new(Action::Undo, "u", t(Key::HintUndo)),
                Hint::new(Action::Paste, "p", t(Key::HintPaste)),
                Hint::new(Action::Save, "^S", t(Key::HintSave)),
                Hint::prefix("^P", t(Key::HintFormatPrefix)),
                Hint::prefix("z", t(Key::HintViewPrefix)),
                Hint::new(Action::Quit, "q", t(Key::HintQuit)),
            ],
            Mode::Insert => vec![
                Hint::new(Action::EnterNormal, "Esc", t(Key::HintNormal)),
                Hint::new(Action::Save, "^S", t(Key::HintSave)),
                Hint::prefix("^P", t(Key::HintFormatPrefix)),
            ],
            Mode::Visual => vec![
                Hint::new(Action::Yank, "y", t(Key::HintYank)),
                Hint::new(Action::DeleteSelection, "x", t(Key::HintDelete)),
                Hint::prefix("^P", t(Key::HintFormatPrefix)),
                Hint::new(Action::EnterNormal, "Esc", t(Key::HintNormal)),
            ],
        }
    }

    fn chord_hints(&self, mode: Mode, pending: &[KeyEvent]) -> Vec<Hint> {
        // `Ctrl-P` (formato) funciona en cualquier modo; el submenu `z` (view)
        // solo en Normal, igual que en `resolve`.
        match pending {
            [k] if is_format_prefix(*k) => format_hints(),
            [k] if mode == Mode::Normal && is_vim_view_prefix(*k) => view_hints(),
            _ => Vec::new(),
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
        // En Normal, una letra sin binding (ej 'w') no inserta nada.
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('w'))),
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
    fn vim_home_end_pgup_pgdn_en_normal_e_insert() {
        let km = VimKeymap;
        for mode in [Mode::Normal, Mode::Insert] {
            assert_eq!(
                resolve1(&km, mode, key(KeyCode::Home)),
                Resolve::Action(Action::LineStart)
            );
            assert_eq!(
                resolve1(&km, mode, key(KeyCode::End)),
                Resolve::Action(Action::LineEnd)
            );
            assert_eq!(
                resolve1(&km, mode, key(KeyCode::PageUp)),
                Resolve::Action(Action::PageUp)
            );
            assert_eq!(
                resolve1(&km, mode, key(KeyCode::PageDown)),
                Resolve::Action(Action::PageDown)
            );
        }
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

    #[test]
    fn vim_submenu_view_zen_solo_en_normal() {
        // En Normal `z` es el prefijo view: queda pendiente y `z z` togglea zen.
        let km = VimKeymap;
        assert_eq!(
            resolve1(&km, Mode::Normal, key(KeyCode::Char('z'))),
            Resolve::Pending
        );
        let z = key(KeyCode::Char('z'));
        assert_eq!(
            km.resolve(Mode::Normal, &[z, key(KeyCode::Char('z'))]),
            Resolve::Action(Action::ToggleZen)
        );
        // En Insert `z` es texto (no prefijo): se inserta y la secuencia `z z` no
        // resuelve a un chord.
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Char('z'))),
            Resolve::Action(Action::InsertChar('z'))
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[z, key(KeyCode::Char('z'))]),
            Resolve::None
        );
    }
}
