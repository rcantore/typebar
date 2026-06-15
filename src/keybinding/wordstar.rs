//! Preset `wordstar`: modeless con chords, homenaje a WordStar. Navegacion por
//! el "diamante" de Ctrl (E/X/S/D = arriba/abajo/izquierda/derecha) y comandos
//! de dos teclas con prefijos `Ctrl-K` (bloque/archivo) y `Ctrl-Q` (movimiento
//! rapido).
//!
//! Nota historica: en WordStar `Ctrl-S` es IZQUIERDA (no guardar); guardar es el
//! chord `Ctrl-K S`. Se respeta esa autenticidad.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

use super::{Action, Keymap, Resolve, has_ctrl, resolve_format_second};
use crate::document::Mode;

pub struct WordstarKeymap;

impl WordstarKeymap {
    /// Resolucion de una unica tecla: diamante, edicion basica o prefijo de
    /// chord (`Ctrl-K`/`Ctrl-Q` solos devuelven `Pending`).
    fn resolve_single(&self, key: KeyEvent) -> Resolve {
        // Shift+flecha extiende la seleccion (las flechas sin shift colapsan).
        if let Some(action) = super::standard::shift_arrow_select(key) {
            return Resolve::Action(action);
        }
        if has_ctrl(key) {
            return match key.code {
                // Diamante de navegacion.
                KeyCode::Char('e') => Resolve::Action(Action::CursorUp),
                KeyCode::Char('x') => Resolve::Action(Action::CursorDown),
                KeyCode::Char('s') => Resolve::Action(Action::CursorLeft),
                KeyCode::Char('d') => Resolve::Action(Action::CursorRight),
                // Ctrl-Z deshace, Ctrl-Y rehace (convencion moderna; el raw mode
                // captura Ctrl-Z asi que no suspende el proceso).
                KeyCode::Char('z') => Resolve::Action(Action::Undo),
                KeyCode::Char('y') => Resolve::Action(Action::Redo),
                // Prefijos de chord: esperan una segunda tecla. `Ctrl-P` es el
                // prefijo de formato (negrita/italica/codigo), uniforme con los
                // otros presets.
                KeyCode::Char('k') | KeyCode::Char('q') | KeyCode::Char('p') => Resolve::Pending,
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

    /// Resolucion de un chord de dos teclas: prefijo `Ctrl-K`/`Ctrl-Q`/`Ctrl-P`
    /// + una letra plana (case-insensitive).
    fn resolve_chord(&self, prefix: KeyEvent, second: KeyEvent) -> Resolve {
        // `Ctrl-P` + letra: formato (negrita/italica/codigo), compartido con los
        // otros presets.
        if matches!(prefix.code, KeyCode::Char('p')) {
            return resolve_format_second(second);
        }
        // La segunda tecla se acepta como letra plana, sin importar mayuscula.
        let letter = match second.code {
            KeyCode::Char(c) => c.to_ascii_lowercase(),
            _ => return Resolve::None,
        };
        match prefix.code {
            KeyCode::Char('k') => match letter {
                's' => Resolve::Action(Action::Save),
                'd' | 'x' => Resolve::Action(Action::SaveAndQuit),
                'q' => Resolve::Action(Action::Quit),
                _ => Resolve::None,
            },
            KeyCode::Char('q') => match letter {
                's' => Resolve::Action(Action::LineStart),
                'd' => Resolve::Action(Action::LineEnd),
                'r' => Resolve::Action(Action::DocStart),
                'c' => Resolve::Action(Action::DocEnd),
                _ => Resolve::None,
            },
            _ => Resolve::None,
        }
    }
}

impl Keymap for WordstarKeymap {
    fn resolve(&self, _mode: Mode, keys: &[KeyEvent]) -> Resolve {
        match keys {
            [single] => self.resolve_single(*single),
            [prefix, second] => self.resolve_chord(*prefix, *second),
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
        "wordstar"
    }
}

#[cfg(test)]
mod tests {
    use super::WordstarKeymap;
    use crate::document::Mode;
    use crate::keybinding::test_support::{ctrl, key, resolve1, shift};
    use crate::keybinding::{Action, Keymap, Resolve};
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn wordstar_shift_flechas_extienden_seleccion() {
        let km = WordstarKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Left)),
            Resolve::Action(Action::SelectLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, shift(KeyCode::Right)),
            Resolve::Action(Action::SelectRight)
        );
    }

    #[test]
    fn wordstar_es_modeless_y_arranca_en_insert() {
        let km = WordstarKeymap;
        assert!(!km.is_modal());
        assert_eq!(km.initial_mode(), Mode::Insert);
        assert_eq!(km.name(), "wordstar");
    }

    #[test]
    fn wordstar_inserta_chars_y_flechas() {
        let km = WordstarKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Char('a'))),
            Resolve::Action(Action::InsertChar('a'))
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, key(KeyCode::Left)),
            Resolve::Action(Action::CursorLeft)
        );
    }

    #[test]
    fn wordstar_diamante_de_navegacion() {
        // Ctrl-E/X/S/D = arriba/abajo/izquierda/derecha (Ctrl-S es IZQUIERDA).
        let km = WordstarKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('e'))),
            Resolve::Action(Action::CursorUp)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('x'))),
            Resolve::Action(Action::CursorDown)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('s'))),
            Resolve::Action(Action::CursorLeft)
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('d'))),
            Resolve::Action(Action::CursorRight)
        );
    }

    #[test]
    fn wordstar_undo_redo() {
        let km = WordstarKeymap;
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
    fn wordstar_prefijos_dan_pending() {
        let km = WordstarKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('k'))),
            Resolve::Pending
        );
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('q'))),
            Resolve::Pending
        );
    }

    #[test]
    fn wordstar_chords_ctrl_k() {
        let km = WordstarKeymap;
        let prefix = ctrl(KeyCode::Char('k'));
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('s'))]),
            Resolve::Action(Action::Save)
        );
        // Case-insensitive: la mayuscula tambien vale.
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('S'))]),
            Resolve::Action(Action::Save)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('d'))]),
            Resolve::Action(Action::SaveAndQuit)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('x'))]),
            Resolve::Action(Action::SaveAndQuit)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('q'))]),
            Resolve::Action(Action::Quit)
        );
    }

    #[test]
    fn wordstar_chords_ctrl_q() {
        let km = WordstarKeymap;
        let prefix = ctrl(KeyCode::Char('q'));
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('s'))]),
            Resolve::Action(Action::LineStart)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('d'))]),
            Resolve::Action(Action::LineEnd)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('r'))]),
            Resolve::Action(Action::DocStart)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('c'))]),
            Resolve::Action(Action::DocEnd)
        );
    }

    #[test]
    fn wordstar_ctrl_p_pendiente_y_chord_formato() {
        let km = WordstarKeymap;
        assert_eq!(
            resolve1(&km, Mode::Insert, ctrl(KeyCode::Char('p'))),
            Resolve::Pending
        );
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
        // Case-insensitive y letra invalida cancela.
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('C'))]),
            Resolve::Action(Action::ToggleCode)
        );
        assert_eq!(
            km.resolve(Mode::Insert, &[p, key(KeyCode::Char('z'))]),
            Resolve::None
        );
    }

    #[test]
    fn wordstar_chord_invalido_da_none() {
        // Ctrl-K seguido de una tecla no bindeada cancela.
        let km = WordstarKeymap;
        let prefix = ctrl(KeyCode::Char('k'));
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Char('z'))]),
            Resolve::None
        );
        // Y una segunda tecla que no es Char tampoco.
        assert_eq!(
            km.resolve(Mode::Insert, &[prefix, key(KeyCode::Esc)]),
            Resolve::None
        );
    }

    #[test]
    fn wordstar_secuencia_larga_es_none() {
        let km = WordstarKeymap;
        let prefix = ctrl(KeyCode::Char('k'));
        assert_eq!(
            km.resolve(
                Mode::Insert,
                &[prefix, key(KeyCode::Char('s')), key(KeyCode::Char('d'))]
            ),
            Resolve::None
        );
    }
}
