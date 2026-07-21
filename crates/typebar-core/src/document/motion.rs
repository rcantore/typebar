//! Movimiento del cursor: horizontal por grafema, vertical preservando la
//! columna visual, y saltos al inicio/fin de linea o documento. Nada de esto
//! modifica el buffer ni marca `dirty`.
//!
//! Cada movimiento tiene un nucleo `*_core` que SOLO mueve el cursor (sin tocar
//! la seleccion). Los `move_*` publicos colapsan la seleccion antes de moverse;
//! los `extend_*` fijan el ancla antes de moverse (para extender el rango). Asi
//! la logica de movimiento (grapheme-based) no se duplica.

use super::Document;
use crate::text::LineGraphemes;

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

    /// Filas visuales de una linea (limites en columnas de pantalla, ver
    /// `text::wrap_boundaries`) junto con su analisis de grafemas. Sin soft
    /// wrap (`wrap_width == 0`) da siempre una sola fila, y todo el movimiento
    /// vertical de abajo colapsa al de siempre: linea a linea.
    fn line_rows(&self, line: usize) -> (LineGraphemes, Vec<usize>) {
        let g = self.graphemes(line);
        let rows = g.row_boundaries(self.wrap_width);
        (g, rows)
    }

    /// Columna (en chars) a la que aterriza el cursor al caer en la fila `row`
    /// de `line`, respetando la columna visual deseada.
    ///
    /// El limite derecho de una fila que NO es la ultima de su linea pertenece
    /// ya a la fila siguiente (es donde se dibujaria el proximo caracter), asi
    /// que ahi se retrocede un grafema: sin eso, bajar una fila podria dejar el
    /// cursor visualmente en la fila de mas abajo, como si la tecla se hubiera
    /// comido un renglon.
    fn col_in_row(&self, g: &LineGraphemes, rows: &[usize], row: usize) -> usize {
        let end = rows[row + 1];
        let target = (rows[row] + self.preferred_display_col).min(end);
        let col = g.col_for_display(target);
        let is_last_row = row + 2 == rows.len();
        if !is_last_row && g.display_col(col) == end {
            // Toda fila tiene al menos un grafema, asi que el limite anterior
            // nunca se pasa al arranque de la fila.
            g.prev_boundary(col)
        } else {
            col
        }
    }

    fn move_up_core(&mut self) {
        self.last_was_insert = false;
        let (g, rows) = self.line_rows(self.line);
        let row = crate::text::row_at(&rows, g.display_col(self.col));
        if row > 0 {
            // Hay una fila visual arriba dentro de la MISMA linea del doc.
            self.col = self.col_in_row(&g, &rows, row - 1);
        } else if self.line > 0 {
            // Primera fila de la linea: se pasa a la ULTIMA fila de la anterior.
            self.line -= 1;
            let (g, rows) = self.line_rows(self.line);
            self.col = self.col_in_row(&g, &rows, rows.len() - 2);
        }
    }

    fn move_down_core(&mut self) {
        self.last_was_insert = false;
        let (g, rows) = self.line_rows(self.line);
        let row = crate::text::row_at(&rows, g.display_col(self.col));
        if row + 2 < rows.len() {
            // Hay otra fila visual abajo dentro de la MISMA linea del doc.
            self.col = self.col_in_row(&g, &rows, row + 1);
        } else if self.line + 1 < self.buffer.len_lines() {
            // Ultima fila de la linea: se pasa a la PRIMERA de la siguiente.
            // Ultima linea valida: len_lines()-1, pero si el buffer termina en
            // '\n' ropey cuenta una linea extra vacia; la permitimos como
            // destino valido.
            self.line += 1;
            let (g, rows) = self.line_rows(self.line);
            self.col = self.col_in_row(&g, &rows, 0);
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

    /// Mueve el cursor `page_size` FILAS visuales hacia arriba, preservando la
    /// columna visual deseada (es un `move_up` repetido, asi que con soft wrap
    /// pagina lo mismo que se ve en pantalla y no la linea logica, que puede
    /// valer varias filas). Si `page_size` excede la posicion actual, queda al
    /// principio del documento. `page_size` lo decide el caller a partir del
    /// alto del viewport (ver `main::draw`).
    pub fn move_page_up(&mut self, page_size: usize) {
        self.clear_selection();
        for _ in 0..page_size {
            let before = (self.line, self.col);
            self.move_up_core();
            if (self.line, self.col) == before {
                break; // ya esta arriba de todo
            }
        }
    }

    /// Mueve el cursor `page_size` FILAS visuales hacia abajo, preservando la
    /// columna visual deseada. Si excede el documento, queda en la ultima linea
    /// valida (igual criterio que `move_to_doc_end`: ignora la linea extra
    /// vacia que ropey cuenta cuando el buffer termina en '\n').
    pub fn move_page_down(&mut self, page_size: usize) {
        self.clear_selection();
        for _ in 0..page_size {
            let before = (self.line, self.col);
            self.move_down_core();
            if (self.line, self.col) == before {
                break; // ya esta abajo de todo
            }
        }
        // Paginar hasta el fondo no deberia aterrizar en la linea vacia que
        // ropey cuenta despues del '\n' final: se vuelve a la ultima con
        // contenido (una fila visual arriba).
        let last_idx = self.buffer.len_lines().saturating_sub(1);
        if self.line == last_idx && last_idx > 0 && self.line_len_chars(last_idx) == 0 {
            self.move_up_core();
        }
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
    fn move_page_up_no_pasa_de_linea_cero() {
        let mut d = doc_with("a\nb\nc\nd\ne\nf");
        d.line = 2;
        d.move_page_up(5);
        assert_eq!(d.line, 0);
    }

    #[test]
    fn move_page_down_clampa_a_ultima_linea_con_contenido() {
        // El '\n' final cuenta una linea extra vacia que ignoramos.
        let mut d = doc_with("a\nb\nc\n");
        d.move_page_down(10);
        assert_eq!(d.line, 2); // "c", no la linea vacia despues del '\n'
    }

    #[test]
    fn move_page_up_y_down_preservan_preferred_col() {
        // El movimiento vertical preserva la columna deseada (sin tocar el
        // preferred), igual que move_up/move_down.
        let mut d = doc_with("largo\nx\notrolargo\ny\nz");
        d.col = 5;
        d.preferred_display_col = 5;
        d.move_page_down(2); // baja 2 lineas: a "otrolargo" (len 9), col=5
        assert_eq!((d.line, d.col), (2, 5));
        d.move_page_up(2); // sube 2: a "largo" (len 5)
        assert_eq!((d.line, d.col), (0, 5));
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

    // --- Movimiento vertical con soft wrap ---------------------------------
    //
    // Todos usan la misma linea envuelta a 8 celdas: "aaa bbb ccc ddd" corta
    // tras el espacio, o sea filas [0,8) = "aaa bbb " y [8,15) = "ccc ddd".

    #[test]
    fn bajar_en_una_linea_envuelta_va_a_la_fila_de_abajo_no_a_la_linea_siguiente() {
        // El bug que motivo esto: con soft wrap, un parrafo largo es UNA linea
        // del documento y varias filas en pantalla; bajar tiene que avanzar una
        // fila, no saltarse el parrafo entero.
        let mut d = doc_with("aaa bbb ccc ddd\nsegunda");
        d.set_wrap_width(8);
        d.move_down();
        assert_eq!((d.line, d.col), (0, 8));
        d.move_down(); // recien ahora se pasa a la linea siguiente
        assert_eq!((d.line, d.col), (1, 0));
    }

    #[test]
    fn subir_entra_a_la_ultima_fila_de_la_linea_anterior() {
        let mut d = doc_with("aaa bbb ccc ddd\nsegunda");
        d.set_wrap_width(8);
        d.line = 1;
        d.move_up();
        assert_eq!((d.line, d.col), (0, 8)); // arranque de la ULTIMA fila
        d.move_up();
        assert_eq!((d.line, d.col), (0, 0));
    }

    #[test]
    fn la_columna_deseada_es_relativa_a_la_fila_visual() {
        let mut d = doc_with("aaa bbb ccc ddd");
        d.set_wrap_width(8);
        d.col = 10; // 3ra celda de la fila de abajo ("ccc ddd" -> la 2da 'c')
        d.move_right(); // fija la columna deseada (x=3 dentro de la fila)
        d.move_left(); // y vuelve a la 10 sin perderla
        d.move_up();
        assert_eq!((d.line, d.col), (0, 2)); // misma x, fila de arriba
        d.move_down();
        assert_eq!((d.line, d.col), (0, 10)); // y de vuelta
    }

    #[test]
    fn subir_a_una_fila_llena_no_aterriza_en_el_limite_de_la_siguiente() {
        // "aaa bbb cccccccc": filas [0,8) y [8,16), las dos de ancho 8. Desde el
        // fin de la de abajo (x=8) la de arriba no tiene celda 8: la columna 8 ya
        // es el arranque de la fila de abajo y el cursor se dibujaria ahi mismo
        // (parece que subir no hizo nada), asi que se retrocede un grafema.
        let mut d = doc_with("aaa bbb cccccccc");
        d.set_wrap_width(8);
        d.move_to_line_end();
        assert_eq!(d.col, 16);
        d.move_up();
        assert_eq!(d.col, 7); // ultima celda de la fila de arriba
    }

    #[test]
    fn paginar_cuenta_filas_visuales() {
        // Dos "paginas" de 2 filas: la primera se consume dentro de la linea
        // envuelta (2 filas), no saltando 2 lineas del documento.
        let mut d = doc_with("aaa bbb ccc ddd\nsegunda\ntercera");
        d.set_wrap_width(8);
        d.move_page_down(2);
        assert_eq!((d.line, d.col), (1, 0));
        d.move_page_up(2);
        assert_eq!((d.line, d.col), (0, 0));
    }

    #[test]
    fn sin_wrap_width_el_movimiento_vertical_es_por_linea() {
        // `wrap_width == 0` (default, y lo que ve cualquier consumidor del core
        // que no dibuje) deja el comportamiento de siempre: linea a linea.
        let mut d = doc_with("aaa bbb ccc ddd\nsegunda");
        d.move_down();
        assert_eq!((d.line, d.col), (1, 0));
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
