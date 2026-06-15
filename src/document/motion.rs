//! Movimiento del cursor: horizontal por grafema, vertical preservando la
//! columna visual, y saltos al inicio/fin de linea o documento. Nada de esto
//! modifica el buffer ni marca `dirty`.
//!
//! Cada movimiento tiene un nucleo `*_core` que SOLO mueve el cursor (sin tocar
//! la seleccion). Los `move_*` publicos colapsan la seleccion antes de moverse;
//! los `extend_*` fijan el ancla antes de moverse (para extender el rango). Asi
//! la logica de movimiento (grapheme-based) no se duplica.

use super::Document;

impl Document {
    fn move_left_core(&mut self) {
        // Un movimiento corta la corrida de tipeo: el proximo insert arranca un
        // grupo de undo nuevo.
        self.last_was_insert = false;
        self.col = self.graphemes(self.line).prev_boundary(self.col);
        self.sync_preferred();
    }

    fn move_right_core(&mut self) {
        self.last_was_insert = false;
        let g = self.graphemes(self.line);
        if self.col < g.len_chars() {
            self.col = g.next_boundary(self.col);
        }
        self.sync_preferred();
    }

    fn move_up_core(&mut self) {
        self.last_was_insert = false;
        if self.line > 0 {
            self.line -= 1;
            self.col = self
                .graphemes(self.line)
                .col_for_display(self.preferred_display_col);
        }
    }

    fn move_down_core(&mut self) {
        self.last_was_insert = false;
        // Ultima linea valida: len_lines()-1, pero si el buffer termina en '\n'
        // ropey cuenta una linea extra vacia; la permitimos como destino valido.
        if self.line + 1 < self.buffer.len_lines() {
            self.line += 1;
            self.col = self
                .graphemes(self.line)
                .col_for_display(self.preferred_display_col);
        }
    }

    pub fn move_left(&mut self) {
        self.clear_selection();
        self.move_left_core();
    }

    pub fn move_right(&mut self) {
        self.clear_selection();
        self.move_right_core();
    }

    pub fn move_up(&mut self) {
        self.clear_selection();
        self.move_up_core();
    }

    pub fn move_down(&mut self) {
        self.clear_selection();
        self.move_down_core();
    }

    /// Fija el ancla (si no la habia) y mueve el cursor a la izquierda,
    /// extendiendo la seleccion.
    pub fn extend_left(&mut self) {
        self.start_selection();
        self.move_left_core();
    }

    pub fn extend_right(&mut self) {
        self.start_selection();
        self.move_right_core();
    }

    pub fn extend_up(&mut self) {
        self.start_selection();
        self.move_up_core();
    }

    pub fn extend_down(&mut self) {
        self.start_selection();
        self.move_down_core();
    }

    /// Entra a Insert *despues* del cursor (la 'a' de Vim): avanza un grafema.
    pub fn move_right_for_append(&mut self) {
        self.move_right();
    }

    /// Mueve el cursor al inicio de la linea actual (col 0).
    pub fn move_to_line_start(&mut self) {
        self.last_was_insert = false;
        self.clear_selection();
        self.col = 0;
        self.sync_preferred();
    }

    /// Mueve el cursor al fin de la linea actual (despues del ultimo char).
    pub fn move_to_line_end(&mut self) {
        self.last_was_insert = false;
        self.clear_selection();
        self.col = self.line_len_chars(self.line);
        self.sync_preferred();
    }

    /// Mueve el cursor al inicio del documento (linea 0, col 0).
    pub fn move_to_doc_start(&mut self) {
        self.last_was_insert = false;
        self.clear_selection();
        self.line = 0;
        self.col = 0;
        self.sync_preferred();
    }

    /// Mueve el cursor al fin del documento: ultima linea con contenido (mismo
    /// criterio que `move_down`, que ignora la linea extra vacia que ropey
    /// cuenta cuando el buffer termina en '\n'), col al final de esa linea.
    pub fn move_to_doc_end(&mut self) {
        self.last_was_insert = false;
        self.clear_selection();
        // len_lines()-1 es la ultima linea; si el buffer termina en '\n' esa es
        // la linea extra vacia, asi que retrocedemos a la anterior.
        let last = self.buffer.len_lines().saturating_sub(1);
        self.line = if last > 0 && self.line_len_chars(last) == 0 {
            last - 1
        } else {
            last
        };
        self.col = self.line_len_chars(self.line);
        self.sync_preferred();
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;

    #[test]
    fn movimiento_clampa_a_lineas_cortas() {
        let mut d = doc_with("largo\nx\notra");
        d.col = 5; // fin de "largo"
        d.preferred_display_col = 5;
        d.move_down(); // a "x" (len 1): col se clampa a 1
        assert_eq!((d.line, d.col), (1, 1));
        d.move_down(); // a "otra" (len 4): vuelve a preferred_col min 4
        assert_eq!((d.line, d.col), (2, 4));
    }

    #[test]
    fn extend_fija_ancla_y_movimiento_normal_colapsa() {
        let mut d = doc_with("hola mundo");
        d.col = 2;
        d.extend_right(); // ancla en 2, cursor a 3
        assert_eq!(d.selection_range(), Some(2..3));
        d.extend_right(); // mantiene ancla, cursor a 4
        assert_eq!(d.selection_range(), Some(2..4));
        d.move_left(); // movimiento normal: colapsa la seleccion
        assert_eq!(d.selection_range(), None);
    }

    #[test]
    fn extend_left_extiende_hacia_atras() {
        let mut d = doc_with("hola");
        d.col = 3;
        d.extend_left();
        assert_eq!(d.selection_range(), Some(2..3));
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
    fn move_to_line_start_va_a_col_cero() {
        let mut d = doc_with("hola");
        d.col = 3;
        d.move_to_line_start();
        assert_eq!(d.col, 0);
    }

    #[test]
    fn move_to_line_end_va_al_final() {
        let mut d = doc_with("hola\nmundo");
        d.line = 0;
        d.col = 1;
        d.move_to_line_end();
        assert_eq!((d.line, d.col), (0, 4)); // "hola" tiene 4 chars
    }

    #[test]
    fn move_to_doc_start_va_al_origen() {
        let mut d = doc_with("ab\ncd");
        d.line = 1;
        d.col = 2;
        d.move_to_doc_start();
        assert_eq!((d.line, d.col), (0, 0));
    }

    #[test]
    fn move_to_doc_end_ultima_linea_con_contenido() {
        let mut d = doc_with("ab\ncd");
        d.move_to_doc_end();
        assert_eq!((d.line, d.col), (1, 2)); // fin de "cd"
    }

    #[test]
    fn move_to_doc_end_ignora_newline_final() {
        // El '\n' final hace que ropey cuente una linea extra vacia; el destino
        // debe ser la ultima linea con contenido, igual que move_down.
        let mut d = doc_with("ab\ncd\n");
        d.move_to_doc_end();
        assert_eq!((d.line, d.col), (1, 2)); // fin de "cd", no la linea vacia
    }

    // --- Grafemas anchos / multi-char --------------------------------------

    #[test]
    fn move_right_salta_emoji_completo() {
        // Familia con ZWJ: un solo grafema de varios chars.
        let familia = "👨\u{200D}👩\u{200D}👧";
        let mut d = doc_with(&format!("{familia}x"));
        let fam_chars = familia.chars().count();
        d.move_right(); // del inicio: salta TODO el cluster de una
        assert_eq!(d.col, fam_chars);
        assert_eq!(d.display_col(), 2); // el cluster ocupa 2 celdas
    }

    #[test]
    fn vertical_preserva_columna_visual_con_cjk() {
        // Linea 0 con CJK; bajar debe respetar la COLUMNA (celdas), no el char.
        let mut d = doc_with("中中中\nabcdef");
        d.col = 2; // tras dos CJK -> columna visual 4
        d.preferred_display_col = d.display_col();
        assert_eq!(d.preferred_display_col, 4);
        d.move_down(); // en "abcdef" la columna 4 es el char-index 4
        assert_eq!(d.col, 4);
    }
}
