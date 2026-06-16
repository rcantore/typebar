//! Modelo del documento editable: buffer de texto + estado del cursor.
//!
//! El buffer es un `ropey::Rope`. El cursor se guarda como (linea, `col`) donde
//! `col` es un indice de *char* dentro de la linea, con la invariante de que
//! siempre cae sobre un limite de grafema: el cursor se mueve y borra por
//! grafema completo (nunca adentro de un cluster ZWJ, emoji o marca
//! combinante). La columna *visual* (celdas de terminal) se calcula aparte con
//! `display_col`, porque char != columna cuando hay CJK/emoji (ver `text.rs`).
//!
//! La `impl Document` se reparte en submodulos: la geometria/guardado vive
//! aca, la edicion en `edit` y el movimiento en `motion`. Los submodulos son
//! descendientes de `document`, asi que pueden tocar los campos privados del
//! struct.

mod clipboard;
mod edit;
mod history;
mod motion;
mod select;

use std::io;
use std::path::{Path, PathBuf};

use ropey::Rope;

use crate::text::LineGraphemes;

/// Modo de edicion estilo Vim minimo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
    /// Modo Visual de Vim: se extiende la seleccion con los movimientos y se
    /// opera sobre ella (toggle de estilo, borrado). Solo lo usa el preset Vim;
    /// los presets modeless manejan la seleccion sin cambiar de modo.
    Visual,
}

/// Documento en memoria con cursor y estado de guardado.
pub struct Document {
    buffer: Rope,
    /// Linea del cursor (0-based).
    pub line: usize,
    /// Columna del cursor en chars dentro de la linea (0-based). Invariante:
    /// siempre sobre un limite de grafema.
    pub col: usize,
    /// Columna *visual* (en celdas) deseada para movimiento vertical: al
    /// subir/bajar tratamos de volver a esta columna de pantalla aunque
    /// pasemos por lineas mas cortas o con anchos distintos.
    preferred_display_col: usize,
    pub mode: Mode,
    pub path: PathBuf,
    pub dirty: bool,
    /// Ancla de la seleccion: char-index ABSOLUTO donde empezo a seleccionarse.
    /// `None` = sin seleccion. El rango va del min al max entre ancla y cursor
    /// (ver `select`).
    selection_anchor: Option<usize>,
    /// Pila de estados para deshacer (el tope es el estado mas reciente). Cada
    /// mutacion empuja un snapshot del estado PREVIO aca antes de mutar (ver
    /// `history`).
    undo_stack: Vec<history::Snapshot>,
    /// Pila de estados para rehacer. Se llena al deshacer y se vacia ante
    /// cualquier edicion nueva.
    redo_stack: Vec<history::Snapshot>,
    /// Flag de coalescing del tipeo: si la ultima mutacion fue un `insert_char`,
    /// los `insert_char` siguientes no toman un snapshot nuevo (asi `undo` borra
    /// toda la corrida de tipeo de una). Un movimiento o cualquier otra mutacion
    /// lo apaga, cortando el grupo.
    last_was_insert: bool,
    /// Portapapeles INTERNO del editor, usado como FALLBACK cuando el clipboard
    /// del SO no esta disponible o una operacion falla (ej. headless/CI): texto
    /// copiado con `yank` que `paste` reinserta. `None` = vacio. No persiste
    /// entre sesiones.
    clipboard: Option<String>,
    /// Handle al clipboard del SO via `arboard`. Es lazy/opcional a proposito:
    /// `arboard::Clipboard::new()` puede fallar en entornos sin servidor de
    /// portapapeles (CI, ssh sin X11, etc.), y en ese caso queda en `None` y
    /// todo cae al buffer interno. No es `Clone`, por eso se maneja por
    /// operacion y nunca entra en los snapshots de undo.
    sys_clipboard: Option<arboard::Clipboard>,
}

impl Document {
    /// Abre el archivo en `path`. Si no existe, arranca con buffer vacio (se
    /// crea recien al guardar).
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        let buffer = match std::fs::read_to_string(&path) {
            Ok(text) => Rope::from_str(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Rope::new(),
            Err(e) => return Err(e),
        };
        Ok(Self {
            buffer,
            line: 0,
            col: 0,
            preferred_display_col: 0,
            mode: Mode::Normal,
            path,
            dirty: false,
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_was_insert: false,
            clipboard: None,
            // Intentamos abrir el clipboard del SO al inicio. Si falla (headless,
            // sin X11, etc.) queda en `None` y se usa el buffer interno. Nunca
            // paniquea: `.ok()` se traga el error a proposito.
            sys_clipboard: arboard::Clipboard::new().ok(),
        })
    }

    /// Texto completo del documento, para pasarselo al render (reparseo full
    /// cada frame: aceptable para este milestone).
    pub fn text(&self) -> String {
        self.buffer.to_string()
    }

    // --- Helpers de geometria interna --------------------------------------

    /// Largo en chars de una linea SIN contar el `\n` final (si lo hay). Es el
    /// maximo `col` valido para moverse (en Normal el cursor no pasa del ultimo
    /// char; en Insert puede pararse justo despues, ver `max_col`).
    fn line_len_chars(&self, line: usize) -> usize {
        if line >= self.buffer.len_lines() {
            return 0;
        }
        let slice = self.buffer.line(line);
        let mut len = slice.len_chars();
        // Restar el salto de linea final si esta presente.
        if len > 0 {
            let last = slice.char(len - 1);
            if last == '\n' {
                len -= 1;
                // Soportar CRLF: restar tambien el '\r'.
                if len > 0 && slice.char(len - 1) == '\r' {
                    len -= 1;
                }
            }
        }
        len
    }

    /// Columna maxima donde puede pararse el cursor en la linea actual. En
    /// Insert puede pararse despues del ultimo char (para tipear al final).
    fn max_col(&self, line: usize) -> usize {
        self.line_len_chars(line)
    }

    /// Indice de char absoluto del cursor actual.
    fn cursor_char_idx(&self) -> usize {
        self.buffer.line_to_char(self.line) + self.col
    }

    /// Re-clampea `col` al largo de la linea actual (tras moverse o editar).
    fn clamp_col(&mut self) {
        let max = self.max_col(self.line);
        if self.col > max {
            self.col = max;
        }
    }

    /// Texto de una linea SIN el `\n` final, para analisis de grafemas.
    fn line_text(&self, line: usize) -> String {
        if line >= self.buffer.len_lines() {
            return String::new();
        }
        let start = self.buffer.line_to_char(line);
        let end = start + self.line_len_chars(line);
        self.buffer.slice(start..end).to_string()
    }

    /// Analisis de grafemas de una linea (limites + anchos).
    fn graphemes(&self, line: usize) -> LineGraphemes {
        LineGraphemes::analyze(&self.line_text(line))
    }

    /// Columna *visual* (en celdas) del cursor en la linea actual. Es lo que la
    /// UI usa para posicionar el cursor real de terminal.
    pub fn display_col(&self) -> usize {
        self.graphemes(self.line).display_col(self.col)
    }

    /// Fija la columna visual deseada a la actual (tras moverse en horizontal o
    /// editar; NO se llama al moverse en vertical, para preservarla).
    fn sync_preferred(&mut self) {
        self.preferred_display_col = self.display_col();
    }

    /// Reposiciona el cursor (line, col) a partir de un char-index absoluto,
    /// clampeando al documento.
    fn set_cursor_char_idx(&mut self, idx: usize) {
        let max = self.buffer.len_chars();
        let idx = idx.min(max);
        let line = self.buffer.char_to_line(idx);
        let line_start = self.buffer.line_to_char(line);
        self.line = line;
        self.col = idx - line_start;
    }

    // --- Guardado ----------------------------------------------------------

    /// Escribe el buffer al path y limpia `dirty`.
    pub fn save(&mut self) -> io::Result<()> {
        std::fs::write(&self.path, self.buffer.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

/// Helpers compartidos por los tests de los submodulos de `document`.
#[cfg(test)]
pub(crate) mod test_support {
    use super::{Document, Mode};
    use ropey::Rope;
    use std::path::PathBuf;

    /// Construye un `Document` desde texto crudo seteando los campos privados
    /// directamente (atajo comun en los tests).
    pub(crate) fn doc_with(text: &str) -> Document {
        Document {
            buffer: Rope::from_str(text),
            line: 0,
            col: 0,
            preferred_display_col: 0,
            mode: Mode::Normal,
            path: PathBuf::from("scratch.md"),
            dirty: false,
            selection_anchor: None,
            undo_stack: Vec::new(),
            redo_stack: Vec::new(),
            last_was_insert: false,
            clipboard: None,
            // En tests no abrimos el clipboard del SO a proposito: en CI no
            // existe y ademas seria estado global compartido entre tests. Con
            // `None` forzamos el path de FALLBACK al buffer interno, que es
            // justo lo que queremos verificar de forma deterministica.
            sys_clipboard: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::doc_with;

    #[test]
    fn line_len_ignora_newline() {
        let d = doc_with("hola\n");
        assert_eq!(d.line_len_chars(0), 4);
    }

    // --- Grafemas anchos / multi-char --------------------------------------

    #[test]
    fn display_col_cuenta_celdas_no_chars() {
        // "a中b": el CJK ocupa 2 celdas. col es char-index; display_col es celdas.
        let mut d = doc_with("a中b");
        d.col = 0;
        assert_eq!(d.display_col(), 0);
        d.col = 1; // despues de 'a'
        assert_eq!(d.display_col(), 1);
        d.col = 2; // despues de '中' (2 celdas)
        assert_eq!(d.display_col(), 3);
    }
}
