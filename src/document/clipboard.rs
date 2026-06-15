//! Portapapeles INTERNO del editor (no el del SO): copiar (`yank`) y pegar
//! (`paste`) texto via el campo `clipboard: Option<String>` del `Document`.
//!
//! `yank` lee la seleccion y la guarda; NO es una mutacion del buffer (no toma
//! snapshot ni marca `dirty`). `paste` SI es una mutacion: inserta en el cursor,
//! toma snapshot al tope (asi es undoable) y marca `dirty`.
//!
//! Decision de diseno: si hay una seleccion activa al pegar, por simplicidad NO
//! se reemplaza; `paste` solo inserta en la posicion del cursor (igual que
//! cualquier otra insercion). Reemplazar la seleccion al pegar queda fuera del
//! scope de este milestone.

use super::Document;

impl Document {
    /// Copia el rango seleccionado al portapapeles interno y limpia la
    /// seleccion. No hace nada si no hay seleccion. NO es una mutacion del
    /// buffer: no toma snapshot ni toca `dirty`.
    pub fn yank(&mut self) {
        let Some(range) = self.selection_range() else {
            return;
        };
        self.clipboard = Some(self.buffer.slice(range).to_string());
        self.clear_selection();
    }

    /// Pega el texto del portapapeles interno en la posicion del cursor y deja
    /// el cursor al final del texto pegado. No hace nada si el portapapeles esta
    /// vacio. Es una MUTACION: toma snapshot (es undoable) y marca `dirty`.
    pub fn paste(&mut self) {
        let Some(text) = self.clipboard.clone() else {
            return;
        };
        if text.is_empty() {
            return;
        }
        self.snapshot();
        self.last_was_insert = false;
        let idx = self.cursor_char_idx();
        self.buffer.insert(idx, &text);
        // Mover el cursor al final del texto pegado: avanza tantos CHARS como
        // tiene el texto (no bytes), respetando la invariante char-index.
        self.set_cursor_char_idx(idx + text.chars().count());
        self.sync_preferred();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;

    #[test]
    fn yank_con_seleccion_copia_y_limpia() {
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.start_selection();
        d.col = 4; // seleccion [0, 4) = "hola"
        d.yank();
        assert_eq!(d.clipboard.as_deref(), Some("hola"));
        assert!(d.selection_range().is_none()); // se limpio la seleccion
        assert_eq!(d.text(), "hola mundo"); // no muta el buffer
        assert!(!d.dirty); // no marca dirty
    }

    #[test]
    fn yank_sin_seleccion_no_hace_nada() {
        let mut d = doc_with("hola");
        d.col = 2;
        d.yank();
        assert!(d.clipboard.is_none());
    }

    #[test]
    fn paste_inserta_en_el_cursor_y_lo_deja_al_final() {
        let mut d = doc_with("hola mundo");
        // Copiar "hola".
        d.col = 0;
        d.start_selection();
        d.col = 4;
        d.yank();
        // Pegar al final del documento.
        d.col = 10;
        d.paste();
        assert_eq!(d.text(), "hola mundohola");
        assert_eq!(d.col, 14); // cursor al final del texto pegado
        assert!(d.dirty);
    }

    #[test]
    fn paste_con_clipboard_vacio_no_hace_nada() {
        let mut d = doc_with("hola");
        d.col = 2;
        d.paste();
        assert_eq!(d.text(), "hola");
        assert_eq!(d.col, 2);
        assert!(!d.dirty);
    }

    #[test]
    fn paste_es_undoable() {
        // Pegar y despues deshacer vuelve al estado previo (verifica que paste
        // tomo un snapshot al tope).
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.start_selection();
        d.col = 4;
        d.yank(); // clipboard = "hola"
        d.col = 10;
        d.paste();
        assert_eq!(d.text(), "hola mundohola");
        d.undo();
        assert_eq!(d.text(), "hola mundo");
        assert_eq!(d.col, 10); // cursor donde estaba al pegar
    }

    #[test]
    fn ciclo_yank_mover_paste_reubica_el_texto() {
        // Copiar "hola ", mover el cursor al final y pegar: el texto se reubica.
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.start_selection();
        d.col = 5; // seleccion [0, 5) = "hola "
        d.yank();
        d.col = 10; // final del documento
        d.paste();
        assert_eq!(d.text(), "hola mundohola ");
        assert_eq!(d.col, 15);
    }

    #[test]
    fn paste_con_grafema_ancho_avanza_por_chars() {
        // Pegar un texto con un grafema multi-char deja el cursor por chars, no
        // por bytes ni celdas.
        let mut d = doc_with("a中b");
        d.col = 0;
        d.start_selection();
        d.col = 2; // seleccion [0, 2) = "a中"
        d.yank();
        d.col = 0;
        d.paste(); // inserta "a中" al inicio
        assert_eq!(d.text(), "a中a中b");
        assert_eq!(d.col, 2); // avanzo 2 chars (no 4 bytes del CJK)
    }
}
