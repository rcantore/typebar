//! Integracion de busqueda/reemplazo con el `Document`: saltar el cursor a una
//! coincidencia y reemplazar todas las ocurrencias. La busqueda en si (encontrar
//! los rangos) vive en `crate::search`; aca solo se aplica sobre el buffer.

use ropey::Rope;

use super::Document;

impl Document {
    /// Offset en BYTES del cursor actual. Lo usa el overlay de busqueda para
    /// encontrar la coincidencia "siguiente" a partir de aca.
    pub fn cursor_byte(&self) -> usize {
        self.buffer.char_to_byte(self.cursor_char_idx())
    }

    /// Mueve el cursor al char correspondiente al offset en BYTES `byte`
    /// (clampeado al documento). Lo usa el overlay de busqueda para saltar a una
    /// coincidencia. Corta la corrida de tipeo.
    pub fn move_cursor_to_byte(&mut self, byte: usize) {
        self.last_was_insert = false;
        let byte = byte.min(self.buffer.len_bytes());
        let char_idx = self.buffer.byte_to_char(byte);
        self.set_cursor_char_idx(char_idx);
        self.sync_preferred();
    }

    /// Reemplaza TODAS las ocurrencias literales de `needle` por `replacement` y
    /// devuelve cuantas se reemplazaron. No toca el buffer ni toma snapshot si no
    /// hay coincidencias (asi un reemplazo sin match no ensucia el undo ni el
    /// flag `dirty`). El cursor se reclampa al documento resultante.
    pub fn replace_all(&mut self, needle: &str, replacement: &str) -> usize {
        let text = self.text();
        let matches = crate::search::find_all(&text, needle);
        if matches.is_empty() {
            return 0;
        }
        self.snapshot();
        self.last_was_insert = false;

        // Reconstruir el texto pegando los tramos entre matches con el reemplazo.
        let mut new = String::with_capacity(text.len());
        let mut last = 0;
        for m in &matches {
            new.push_str(&text[last..m.start]);
            new.push_str(replacement);
            last = m.end;
        }
        new.push_str(&text[last..]);

        self.buffer = Rope::from_str(&new);
        self.clear_selection();
        // El reemplazo corre las posiciones: reclampar el cursor al nuevo texto.
        let last_line = self.buffer.len_lines().saturating_sub(1);
        if self.line > last_line {
            self.line = last_line;
        }
        self.clamp_col();
        self.sync_preferred();
        self.dirty = true;
        matches.len()
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;

    #[test]
    fn replace_all_reemplaza_y_cuenta() {
        let mut d = doc_with("foo bar foo");
        let n = d.replace_all("foo", "baz");
        assert_eq!(n, 2);
        assert_eq!(d.text(), "baz bar baz");
        assert!(d.dirty);
    }

    #[test]
    fn replace_all_sin_match_no_toca_nada() {
        let mut d = doc_with("hola");
        let n = d.replace_all("xyz", "abc");
        assert_eq!(n, 0);
        assert_eq!(d.text(), "hola");
        // Sin match no se ensucia el documento.
        assert!(!d.dirty);
    }

    #[test]
    fn replace_all_es_undoable() {
        let mut d = doc_with("a a a");
        d.replace_all("a", "bb");
        assert_eq!(d.text(), "bb bb bb");
        d.undo();
        assert_eq!(d.text(), "a a a");
    }

    #[test]
    fn replace_all_distinta_longitud_reclampa_cursor() {
        // Reemplazo que acorta el texto: el cursor no debe quedar fuera de rango.
        let mut d = doc_with("aaaa");
        d.col = 4;
        d.replace_all("aaaa", "x");
        assert_eq!(d.text(), "x");
        assert!(d.col <= 1);
    }

    #[test]
    fn move_cursor_to_byte_salta_a_la_posicion() {
        let mut d = doc_with("ab\ncd");
        d.move_cursor_to_byte(4); // 'd': linea 1, col 1
        assert_eq!((d.line, d.col), (1, 1));
    }
}
