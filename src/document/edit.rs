//! Operaciones de edicion del `Document`: insercion, borrado y toggle de
//! estilos inline (negrita/italica/codigo). Todas marcan `dirty` y mantienen el
//! cursor sobre un limite de grafema.

use crate::markdown::InlineKind;

use super::Document;

impl Document {
    /// Inserta un char imprimible en el cursor y avanza una columna (char).
    pub fn insert_char(&mut self, c: char) {
        let idx = self.cursor_char_idx();
        self.buffer.insert_char(idx, c);
        self.col += 1;
        self.sync_preferred();
        self.dirty = true;
    }

    /// Inserta un salto de linea en el cursor; baja a la linea nueva, col 0.
    pub fn insert_newline(&mut self) {
        let idx = self.cursor_char_idx();
        self.buffer.insert_char(idx, '\n');
        self.line += 1;
        self.col = 0;
        self.preferred_display_col = 0;
        self.dirty = true;
    }

    /// Borra el *grafema* previo al cursor (un emoji/bandera/cluster ZWJ entero,
    /// no un char suelto). Si col == 0, junta con la linea de arriba.
    pub fn backspace(&mut self) {
        if self.col == 0 && self.line == 0 {
            return; // inicio del documento: nada que borrar
        }
        if self.col > 0 {
            let prev = self.graphemes(self.line).prev_boundary(self.col);
            let base = self.buffer.line_to_char(self.line);
            self.buffer.remove(base + prev..base + self.col);
            self.col = prev;
        } else {
            // col == 0: unirse a la linea anterior. El cursor queda donde
            // terminaba esa linea.
            let prev = self.line - 1;
            let prev_len = self.line_len_chars(prev);
            let idx = self.cursor_char_idx();
            self.buffer.remove(idx - 1..idx); // borra el '\n' anterior
            self.line = prev;
            self.col = prev_len;
        }
        self.sync_preferred();
        self.dirty = true;
    }

    /// Borra el *grafema* bajo el cursor (la 'x' de Vim). No hace nada si el
    /// cursor esta al final de la linea.
    pub fn delete_char(&mut self) {
        let g = self.graphemes(self.line);
        if self.col >= g.len_chars() {
            return; // no hay grafema bajo el cursor (estamos al final)
        }
        let next = g.next_boundary(self.col);
        let base = self.buffer.line_to_char(self.line);
        self.buffer.remove(base + self.col..base + next);
        self.clamp_col();
        self.sync_preferred();
        self.dirty = true;
    }

    /// Abre una linea nueva debajo de la actual y deja el cursor ahi (la 'o').
    pub fn open_line_below(&mut self) {
        let line_end = self.buffer.line_to_char(self.line) + self.line_len_chars(self.line);
        self.buffer.insert_char(line_end, '\n');
        self.line += 1;
        self.col = 0;
        self.preferred_display_col = 0;
        self.dirty = true;
    }

    // --- Estilos inline (toggle de negrita/italica/codigo) -----------------

    /// Togglea el enfasis `kind` sobre la PALABRA bajo el cursor.
    ///
    /// - Si el cursor ya esta dentro de un enfasis de ese tipo (detectado via el
    ///   AST de tree-sitter), DESTOGGLEA: quita los marcadores de apertura y
    ///   cierre.
    /// - Si no, y hay una palabra (corrida de alfanumericos) bajo el cursor, la
    ///   ENVUELVE con el marcador.
    /// - Si no hay palabra (cursor en espacio/vacio), inserta el par de
    ///   marcadores vacio y deja el cursor entre ambos para tipear adentro.
    ///
    /// En todos los casos el cursor queda sobre el MISMO char de contenido que
    /// antes (o entre los marcadores en el caso vacio).
    pub fn toggle_inline(&mut self, kind: InlineKind) {
        let byte_off = self.buffer.char_to_byte(self.cursor_char_idx());
        match crate::markdown::enclosing(&self.text(), byte_off, kind) {
            Some(markers) => self.destoggle_inline(&markers),
            None => self.toggle_inline_word(kind),
        }
        self.clamp_col();
        self.sync_preferred();
        self.dirty = true;
    }

    /// Quita los marcadores `open`/`close` (en bytes) de un enfasis existente.
    /// Borra primero el cierre (offset mayor) para no invalidar el rango de
    /// apertura. Reubica el cursor restando el largo del marcador de apertura si
    /// estaba en/despues del contenido.
    fn destoggle_inline(&mut self, markers: &crate::markdown::Markers) {
        let open_start = self.buffer.byte_to_char(markers.open.start);
        let open_end = self.buffer.byte_to_char(markers.open.end);
        let close_start = self.buffer.byte_to_char(markers.close.start);
        let close_end = self.buffer.byte_to_char(markers.close.end);
        let open_len = open_end - open_start;
        let close_len = close_end - close_start;

        let cursor_old = self.cursor_char_idx();

        // Borrar primero el cierre (mayor), despues la apertura (menor).
        self.buffer.remove(close_start..close_end);
        self.buffer.remove(open_start..open_end);

        // Ajustar el cursor segun de que lado de los marcadores estaba.
        let mut new_idx = cursor_old;
        if cursor_old >= close_end {
            new_idx = cursor_old - open_len - close_len;
        } else if cursor_old >= open_end {
            new_idx = cursor_old - open_len;
        } else if cursor_old >= open_start {
            // Estaba sobre el marcador de apertura: cae al inicio del contenido.
            new_idx = open_start;
        }
        self.set_cursor_char_idx(new_idx);
    }

    /// Envuelve la palabra bajo el cursor (o inserta un par vacio si no hay
    /// palabra) con el marcador de `kind`.
    fn toggle_inline_word(&mut self, kind: InlineKind) {
        let marker = kind.marker();
        let marker_len = kind.marker_len();
        let base = self.buffer.line_to_char(self.line);

        match self.word_under_cursor() {
            Some((ws, we)) => {
                // Insertar primero en la posicion mayor (`we`) para no correr la
                // menor (`ws`).
                self.buffer.insert(base + we, marker);
                self.buffer.insert(base + ws, marker);
                // El cursor estaba en/despues de `ws`: se corre por el marcador
                // de apertura insertado a su izquierda.
                if self.col >= ws {
                    self.col += marker_len;
                }
            }
            None => {
                // Sin palabra: par vacio en el cursor, cursor entre ambos.
                let idx = base + self.col;
                self.buffer.insert(idx, marker);
                self.buffer.insert(idx + marker_len, marker);
                self.col += marker_len;
            }
        }
    }

    /// Corrida maximal de chars `is_alphanumeric()` alrededor del cursor en la
    /// linea actual, como par de char-indices `[ws, we)` (relativos a la linea).
    /// `None` si el cursor no esta sobre/junto a una palabra.
    fn word_under_cursor(&self) -> Option<(usize, usize)> {
        let line: Vec<char> = self.line_text(self.line).chars().collect();
        let len = line.len();

        // Hay palabra solo si el char BAJO el cursor es alfanumerico. Si el
        // cursor esta sobre un espacio (o al final de la linea), no hay palabra
        // y se inserta un par vacio.
        if self.col >= len || !line[self.col].is_alphanumeric() {
            return None;
        }

        // Expandir a izquierda mientras el char previo sea alfanumerico.
        let mut ws = self.col;
        while ws > 0 && line[ws - 1].is_alphanumeric() {
            ws -= 1;
        }
        // Expandir a derecha mientras el char en la posicion sea alfanumerico.
        let mut we = self.col;
        while we < len && line[we].is_alphanumeric() {
            we += 1;
        }
        Some((ws, we))
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;
    use crate::markdown::InlineKind;

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
    fn open_line_below_inserta_y_baja() {
        let mut d = doc_with("ab\ncd");
        d.col = 1; // sobre la primer linea
        d.open_line_below();
        assert_eq!(d.text(), "ab\n\ncd");
        assert_eq!((d.line, d.col), (1, 0));
    }

    // --- Grafemas anchos / multi-char --------------------------------------

    #[test]
    fn backspace_borra_emoji_entero() {
        let familia = "👨\u{200D}👩\u{200D}👧";
        let mut d = doc_with(&format!("a{familia}"));
        d.col = d.line_len_chars(0); // al final
        d.backspace();
        assert_eq!(d.text(), "a"); // el emoji familia se fue completo, no a pedazos
        assert_eq!(d.col, 1);
    }

    #[test]
    fn delete_char_borra_grafema_combinante() {
        // "é" en NFD = 'e' + combinante U+0301: un grafema, dos chars.
        let mut d = doc_with("e\u{301}z");
        d.col = 0;
        d.delete_char(); // borra el grafema completo, no solo la 'e'
        assert_eq!(d.text(), "z");
    }

    // --- Toggle de estilos inline ------------------------------------------

    #[test]
    fn toggle_envuelve_palabra_negrita() {
        // "negro" con el cursor sobre la 'g' (col 2): togglear bold -> "**negro**"
        // y el cursor sigue sobre la 'g'.
        let mut d = doc_with("negro");
        d.col = 2;
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "**negro**");
        assert_eq!((d.line, d.col), (0, 4)); // 'g' corrida 2 chars por "**"
        assert!(d.dirty);
    }

    #[test]
    fn toggle_envuelve_palabra_italica() {
        let mut d = doc_with("negro");
        d.col = 2;
        d.toggle_inline(InlineKind::Italic);
        assert_eq!(d.text(), "*negro*");
        assert_eq!((d.line, d.col), (0, 3)); // corrida 1 char por "*"
    }

    #[test]
    fn toggle_envuelve_palabra_codigo() {
        let mut d = doc_with("negro");
        d.col = 2;
        d.toggle_inline(InlineKind::Code);
        assert_eq!(d.text(), "`negro`");
        assert_eq!((d.line, d.col), (0, 3));
    }

    #[test]
    fn toggle_palabra_desde_inicio() {
        // Cursor sobre la primer letra: la palabra entera se envuelve.
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "**hola** mundo");
        assert_eq!((d.line, d.col), (0, 2)); // sobre la 'h'
    }

    #[test]
    fn destoggle_negrita_con_cursor_adentro() {
        // "**negro**" con el cursor sobre la 'g' (col 4): destogglear -> "negro"
        // con el cursor todavia sobre la 'g' (col 2).
        let mut d = doc_with("**negro**");
        d.col = 4;
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "negro");
        assert_eq!((d.line, d.col), (0, 2));
    }

    #[test]
    fn toggle_es_idempotente_ida_y_vuelta() {
        // Togglear dos veces deja el texto y el cursor como al principio.
        let mut d = doc_with("negro");
        d.col = 2;
        d.toggle_inline(InlineKind::Bold);
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "negro");
        assert_eq!((d.line, d.col), (0, 2));
    }

    #[test]
    fn destoggle_italica() {
        let mut d = doc_with("*x*");
        d.col = 1; // sobre la 'x'
        d.toggle_inline(InlineKind::Italic);
        assert_eq!(d.text(), "x");
        assert_eq!((d.line, d.col), (0, 0));
    }

    #[test]
    fn toggle_sin_palabra_inserta_par_vacio() {
        // Cursor en un espacio: inserta "****" y queda entre los marcadores.
        let mut d = doc_with("a b");
        d.col = 1; // sobre el espacio
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "a**** b");
        assert_eq!((d.line, d.col), (0, 3)); // entre los dos "**"
    }

    #[test]
    fn toggle_en_linea_vacia_inserta_par_vacio() {
        let mut d = doc_with("");
        d.col = 0;
        d.toggle_inline(InlineKind::Code);
        assert_eq!(d.text(), "``");
        assert_eq!((d.line, d.col), (0, 1)); // entre los backticks
    }

    #[test]
    fn toggle_negrita_no_destogglea_italica() {
        // Sobre "*x*" (italica), togglear BOLD no la detecta: envuelve la 'x'.
        let mut d = doc_with("*x*");
        d.col = 1; // sobre la 'x'
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "***x***");
    }
}
