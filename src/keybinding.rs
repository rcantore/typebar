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
//! Presets actuales:
//! - `standard`: modeless (siempre se inserta), flechas para moverse. DEFAULT.
//! - `vim`: modal (Normal/Insert), replica el comportamiento Vim minimo.
//! - `wordstar`: modeless con chords, homenaje al editor clasico (diamante de
//!   navegacion `Ctrl-E/X/S/D` + chords `Ctrl-K`/`Ctrl-Q`).

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

/// Devuelve true si la tecla trae el modificador CONTROL.
fn has_ctrl(key: KeyEvent) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
}

/// Preset modeless: siempre se inserta texto, las flechas mueven el cursor. Es
/// el comportamiento esperado por la mayoria de la gente (no hay modos).
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

/// Preset modal estilo Vim minimo: Normal para moverse/comandos, Insert para
/// tipear. Replica el comportamiento previo hardcodeado del editor.
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

/// Preset modeless con chords, homenaje a WordStar. Navegacion por el "diamante"
/// de Ctrl (E/X/S/D = arriba/abajo/izquierda/derecha) y comandos de dos teclas
/// con prefijos `Ctrl-K` (bloque/archivo) y `Ctrl-Q` (movimiento rapido).
///
/// Nota historica: en WordStar `Ctrl-S` es IZQUIERDA (no guardar); guardar es el
/// chord `Ctrl-K S`. Se respeta esa autenticidad.
pub struct WordstarKeymap;

impl WordstarKeymap {
    /// Resolucion de una unica tecla: diamante, edicion basica o prefijo de
    /// chord (`Ctrl-K`/`Ctrl-Q` solos devuelven `Pending`).
    fn resolve_single(&self, key: KeyEvent) -> Resolve {
        if has_ctrl(key) {
            return match key.code {
                // Diamante de navegacion.
                KeyCode::Char('e') => Resolve::Action(Action::CursorUp),
                KeyCode::Char('x') => Resolve::Action(Action::CursorDown),
                KeyCode::Char('s') => Resolve::Action(Action::CursorLeft),
                KeyCode::Char('d') => Resolve::Action(Action::CursorRight),
                // Prefijos de chord: esperan una segunda tecla.
                KeyCode::Char('k') | KeyCode::Char('q') => Resolve::Pending,
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

    /// Resolucion de un chord de dos teclas: prefijo `Ctrl-K`/`Ctrl-Q` + una
    /// letra plana (case-insensitive).
    fn resolve_chord(&self, prefix: KeyEvent, second: KeyEvent) -> Resolve {
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

/// Construye el preset segun su nombre. Si no matchea ninguno conocido, cae al
/// default `standard` (modeless).
pub fn keymap_from_name(name: &str) -> Box<dyn Keymap> {
    match name {
        "vim" => Box::new(VimKeymap),
        "wordstar" => Box::new(WordstarKeymap),
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

    /// Resuelve una secuencia de una sola tecla (atajo comun en los tests).
    fn resolve1(km: &dyn Keymap, mode: Mode, k: KeyEvent) -> Resolve {
        km.resolve(mode, &[k])
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

    // --- WordstarKeymap ----------------------------------------------------

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

    // --- Seleccion de preset ----------------------------------------------

    #[test]
    fn nombre_desconocido_cae_a_standard() {
        assert_eq!(keymap_from_name("loquesea").name(), "standard");
        assert_eq!(keymap_from_name("vim").name(), "vim");
        assert_eq!(keymap_from_name("standard").name(), "standard");
        assert_eq!(keymap_from_name("wordstar").name(), "wordstar");
    }
}
