//! Modelo del documento editable: buffer de texto + estado del cursor.
//!
//! El buffer es un `ropey::Rope`. El cursor se guarda como (linea, columna) en
//! *chars* (no bytes), porque la UI razona en columnas visuales y el render es
//! 1:1 (los marcadores nunca se ocultan, asi que char == columna en pantalla,
//! salvo anchos raros que quedan fuera de scope de este milestone).

use std::io;
use std::path::{Path, PathBuf};

use ropey::Rope;

/// Modo de edicion estilo Vim minimo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Normal,
    Insert,
}

/// Documento en memoria con cursor y estado de guardado.
pub struct Document {
    buffer: Rope,
    /// Linea del cursor (0-based).
    pub line: usize,
    /// Columna del cursor en chars dentro de la linea (0-based).
    pub col: usize,
    /// Columna deseada para movimiento vertical: al subir/bajar tratamos de
    /// volver a esta columna aunque hayamos pasado por lineas mas cortas.
    preferred_col: usize,
    pub mode: Mode,
    pub path: PathBuf,
    pub dirty: bool,
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
            preferred_col: 0,
            mode: Mode::Normal,
            path,
            dirty: false,
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

    // --- Edicion -----------------------------------------------------------

    /// Inserta un char imprimible en el cursor y avanza una columna.
    pub fn insert_char(&mut self, c: char) {
        let idx = self.cursor_char_idx();
        self.buffer.insert_char(idx, c);
        self.col += 1;
        self.preferred_col = self.col;
        self.dirty = true;
    }

    /// Inserta un salto de linea en el cursor; baja a la linea nueva, col 0.
    pub fn insert_newline(&mut self) {
        let idx = self.cursor_char_idx();
        self.buffer.insert_char(idx, '\n');
        self.line += 1;
        self.col = 0;
        self.preferred_col = 0;
        self.dirty = true;
    }

    /// Borra el char previo al cursor. Si col == 0, junta con la linea de
    /// arriba (el cursor cae al punto de union).
    pub fn backspace(&mut self) {
        let idx = self.cursor_char_idx();
        if idx == 0 {
            return; // inicio del documento: nada que borrar
        }
        if self.col > 0 {
            self.buffer.remove(idx - 1..idx);
            self.col -= 1;
        } else {
            // col == 0: unirse a la linea anterior. El cursor queda donde
            // terminaba esa linea.
            let prev = self.line - 1;
            let prev_len = self.line_len_chars(prev);
            self.buffer.remove(idx - 1..idx); // borra el '\n' anterior
            self.line = prev;
            self.col = prev_len;
        }
        self.preferred_col = self.col;
        self.dirty = true;
    }

    /// Borra el char bajo el cursor (la 'x' de Vim). No hace nada si el cursor
    /// esta sobre el `\n` virtual del final de linea.
    pub fn delete_char(&mut self) {
        let idx = self.cursor_char_idx();
        if self.col >= self.line_len_chars(self.line) {
            return; // no hay char bajo el cursor (estamos al final de la linea)
        }
        self.buffer.remove(idx..idx + 1);
        self.clamp_col();
        self.preferred_col = self.col;
        self.dirty = true;
    }

    // --- Movimiento --------------------------------------------------------

    pub fn move_left(&mut self) {
        if self.col > 0 {
            self.col -= 1;
        }
        self.preferred_col = self.col;
    }

    pub fn move_right(&mut self) {
        let max = self.max_col(self.line);
        if self.col < max {
            self.col += 1;
        }
        self.preferred_col = self.col;
    }

    pub fn move_up(&mut self) {
        if self.line > 0 {
            self.line -= 1;
            self.col = self.preferred_col.min(self.max_col(self.line));
        }
    }

    pub fn move_down(&mut self) {
        // Ultima linea valida: len_lines()-1, pero si el buffer termina en '\n'
        // ropey cuenta una linea extra vacia; la permitimos como destino valido.
        if self.line + 1 < self.buffer.len_lines() {
            self.line += 1;
            self.col = self.preferred_col.min(self.max_col(self.line));
        }
    }

    /// Entra a Insert *despues* del cursor (la 'a' de Vim).
    pub fn move_right_for_append(&mut self) {
        let max = self.max_col(self.line);
        if self.col < max {
            self.col += 1;
        }
        self.preferred_col = self.col;
    }

    /// Abre una linea nueva debajo de la actual y deja el cursor ahi (la 'o').
    pub fn open_line_below(&mut self) {
        let line_end = self.buffer.line_to_char(self.line) + self.line_len_chars(self.line);
        self.buffer.insert_char(line_end, '\n');
        self.line += 1;
        self.col = 0;
        self.preferred_col = 0;
        self.dirty = true;
    }

    // --- Guardado ----------------------------------------------------------

    /// Escribe el buffer al path y limpia `dirty`.
    pub fn save(&mut self) -> io::Result<()> {
        std::fs::write(&self.path, self.buffer.to_string())?;
        self.dirty = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc_with(text: &str) -> Document {
        Document {
            buffer: Rope::from_str(text),
            line: 0,
            col: 0,
            preferred_col: 0,
            mode: Mode::Normal,
            path: PathBuf::from("scratch.md"),
            dirty: false,
        }
    }

    #[test]
    fn insert_char_avanza_y_marca_dirty() {
        let mut d = doc_with("");
        d.insert_char('h');
        d.insert_char('i');
        assert_eq!(d.text(), "hi");
        assert_eq!((d.line, d.col), (0, 2));
        assert!(d.dirty);
    }

    #[test]
    fn insert_newline_baja_de_linea() {
        let mut d = doc_with("ab");
        d.col = 1;
        d.insert_newline();
        assert_eq!(d.text(), "a\nb");
        assert_eq!((d.line, d.col), (1, 0));
    }

    #[test]
    fn backspace_dentro_de_linea() {
        let mut d = doc_with("abc");
        d.col = 2;
        d.backspace();
        assert_eq!(d.text(), "ac");
        assert_eq!(d.col, 1);
    }

    #[test]
    fn backspace_junta_lineas() {
        let mut d = doc_with("ab\ncd");
        d.line = 1;
        d.col = 0;
        d.backspace();
        assert_eq!(d.text(), "abcd");
        assert_eq!((d.line, d.col), (0, 2));
    }

    #[test]
    fn backspace_inicio_no_hace_nada() {
        let mut d = doc_with("abc");
        d.backspace();
        assert_eq!(d.text(), "abc");
        assert_eq!((d.line, d.col), (0, 0));
    }

    #[test]
    fn delete_char_borra_bajo_cursor() {
        let mut d = doc_with("abc");
        d.col = 1;
        d.delete_char();
        assert_eq!(d.text(), "ac");
        assert_eq!(d.col, 1);
    }

    #[test]
    fn delete_char_al_final_no_hace_nada() {
        let mut d = doc_with("ab");
        d.col = 2;
        d.delete_char();
        assert_eq!(d.text(), "ab");
    }

    #[test]
    fn movimiento_clampa_a_lineas_cortas() {
        let mut d = doc_with("largo\nx\notra");
        d.col = 5; // fin de "largo"
        d.preferred_col = 5;
        d.move_down(); // a "x" (len 1): col se clampa a 1
        assert_eq!((d.line, d.col), (1, 1));
        d.move_down(); // a "otra" (len 4): vuelve a preferred_col min 4
        assert_eq!((d.line, d.col), (2, 4));
    }

    #[test]
    fn move_right_no_pasa_del_fin() {
        let mut d = doc_with("ab");
        d.move_right();
        d.move_right();
        d.move_right();
        assert_eq!(d.col, 2);
    }

    #[test]
    fn open_line_below_inserta_y_baja() {
        let mut d = doc_with("ab\ncd");
        d.col = 1; // sobre la primer linea
        d.open_line_below();
        assert_eq!(d.text(), "ab\n\ncd");
        assert_eq!((d.line, d.col), (1, 0));
    }

    #[test]
    fn line_len_ignora_newline() {
        let d = doc_with("hola\n");
        assert_eq!(d.line_len_chars(0), 4);
    }
}
