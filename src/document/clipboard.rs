//! Portapapeles del editor: copiar (`yank`) y pegar (`paste`).
//!
//! Por defecto usa el portapapeles del SISTEMA OPERATIVO via `arboard` (asi se
//! puede copiar/pegar contra otras apps), con FALLBACK al buffer interno
//! `clipboard: Option<String>` del `Document` cuando el clipboard del SO no esta
//! disponible (headless/CI) o cuando una operacion de get/set falla. Nunca se
//! paniquea por el clipboard: cualquier error cae al buffer interno.
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
    /// Escribe `text` al clipboard del SO (si hay handle) y SIEMPRE tambien al
    /// buffer interno como fallback. Si el SO falla, el buffer interno alcanza
    /// para que `paste` siga funcionando en esta sesion. No paniquea.
    fn clipboard_set(&mut self, text: String) {
        if let Some(cb) = self.sys_clipboard.as_mut() {
            // Ignoramos el error a proposito: si el SO no acepta el set, igual
            // queda guardado en el buffer interno de abajo.
            let _ = cb.set_text(text.clone());
        }
        self.clipboard = Some(text);
    }

    /// Lee el texto a pegar: primero intenta el clipboard del SO; si no hay
    /// handle o la lectura falla, cae al buffer interno. `None` = no hay nada
    /// para pegar. No paniquea.
    fn clipboard_get(&mut self) -> Option<String> {
        if let Some(cb) = self.sys_clipboard.as_mut()
            && let Ok(text) = cb.get_text()
        {
            return Some(text);
        }
        self.clipboard.clone()
    }

    /// Copia el rango seleccionado al portapapeles (SO + fallback interno) y
    /// limpia la seleccion. No hace nada si no hay seleccion. NO es una mutacion
    /// del buffer: no toma snapshot ni toca `dirty`.
    pub fn yank(&mut self) {
        let Some(range) = self.selection_range() else {
            return;
        };
        let text = self.buffer.slice(range).to_string();
        self.clipboard_set(text);
        self.clear_selection();
    }

    /// Pega el texto del portapapeles (SO o fallback interno) en la posicion del
    /// cursor y deja el cursor al final del texto pegado. No hace nada si el
    /// portapapeles esta vacio. Es una MUTACION: toma snapshot (es undoable) y
    /// marca `dirty`.
    pub fn paste(&mut self) {
        let Some(text) = self.clipboard_get() else {
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

    // Nota: `doc_with` arranca con `sys_clipboard: None`, asi que TODOS estos
    // tests ejercitan el path de FALLBACK al buffer interno, sin tocar el
    // clipboard global del SO (que en CI no existe y seria estado compartido).

    #[test]
    fn yank_sin_clipboard_del_so_usa_el_buffer_interno() {
        // Con el clipboard del SO ausente, yank igual deja el texto en el
        // fallback interno (verifica que `clipboard_set` no depende del SO).
        let mut d = doc_with("hola mundo");
        assert!(d.sys_clipboard.is_none()); // precondicion: no hay clipboard del SO
        d.col = 0;
        d.start_selection();
        d.col = 4; // seleccion [0, 4) = "hola"
        d.yank();
        assert_eq!(d.clipboard.as_deref(), Some("hola"));
    }

    #[test]
    fn ciclo_yank_paste_funciona_solo_con_fallback() {
        // Sin clipboard del SO, el ciclo completo yank -> paste sigue andando
        // 100% via el buffer interno (es el contrato del fallback).
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.start_selection();
        d.col = 5; // "hola "
        d.yank();
        d.col = 10; // final
        d.paste();
        assert_eq!(d.text(), "hola mundohola ");
    }

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
