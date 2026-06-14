//! Modelo de seleccion del documento. La seleccion se guarda como un *ancla*
//! (char-index ABSOLUTO en el buffer) opcional: el rango seleccionado va del
//! min al max entre el ancla y el cursor. `None` = sin seleccion.
//!
//! Esto es agnostico al preset de teclado: en Vim lo maneja el modo Visual, en
//! los presets modeless (standard/wordstar) lo manejan las Shift+flechas. En
//! ambos casos la geometria es la misma (character-wise, no por palabra/linea).

use std::ops::Range;

use crate::markdown::InlineKind;

use super::Document;

impl Document {
    /// Rango seleccionado en chars `[start, end)`, ordenado. `None` si no hay
    /// ancla o si la seleccion esta vacia (`start == end`).
    pub fn selection_range(&self) -> Option<Range<usize>> {
        let anchor = self.selection_anchor?;
        let cursor = self.cursor_char_idx();
        let start = anchor.min(cursor);
        let end = anchor.max(cursor);
        if start == end {
            return None;
        }
        Some(start..end)
    }

    /// Mismo rango que `selection_range` pero convertido a BYTES, para pasarselo
    /// al render. No expone el buffer: la conversion char->byte se hace aca.
    pub fn selection_byte_range(&self) -> Option<Range<usize>> {
        let r = self.selection_range()?;
        let start = self.buffer.char_to_byte(r.start);
        let end = self.buffer.char_to_byte(r.end);
        Some(start..end)
    }

    /// Fija el ancla en la posicion actual del cursor si todavia no hay una. Si
    /// ya hay ancla no la toca (para que extender no la reinicie en cada paso).
    pub fn start_selection(&mut self) {
        if self.selection_anchor.is_none() {
            self.selection_anchor = Some(self.cursor_char_idx());
        }
    }

    /// Borra el ancla: deja de haber seleccion.
    pub fn clear_selection(&mut self) {
        self.selection_anchor = None;
    }

    /// Envuelve el rango seleccionado `[start, end)` (char-index ABSOLUTO) con el
    /// marcador de `kind`. Reubica el cursor por el marcador de apertura si
    /// estaba en/despues de `start` (y por el de cierre si estaba en/despues de
    /// `end`).
    ///
    /// Limitacion conocida: a diferencia del toggle sin seleccion, esto NO
    /// detecta un enfasis ya existente, asi que SIEMPRE envuelve (nunca
    /// destogglea). Para una seleccion que ya esta envuelta, esto agrega un par
    /// extra de marcadores.
    pub(super) fn toggle_inline_selection(&mut self, kind: InlineKind, range: Range<usize>) {
        let marker = kind.marker();
        let marker_len = kind.marker_len();
        let cursor = self.cursor_char_idx();

        // Insertar primero en la posicion mayor (`end`) para no correr la menor.
        self.buffer.insert(range.end, marker);
        self.buffer.insert(range.start, marker);

        // Ajustar el cursor: se corre por la apertura si estaba en/despues de
        // `start`, y otra vez por el cierre si estaba en/despues de `end`.
        let mut new_idx = cursor;
        if cursor >= range.start {
            new_idx += marker_len;
        }
        if cursor >= range.end {
            new_idx += marker_len;
        }
        self.set_cursor_char_idx(new_idx);
    }

    /// Borra el rango seleccionado del buffer, deja el cursor en `start` y limpia
    /// la seleccion. No hace nada si no hay seleccion.
    pub fn delete_selection(&mut self) {
        let Some(range) = self.selection_range() else {
            return;
        };
        self.buffer.remove(range.clone());
        self.set_cursor_char_idx(range.start);
        self.clear_selection();
        self.sync_preferred();
        self.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;
    use crate::markdown::InlineKind;

    #[test]
    fn toggle_con_seleccion_envuelve_el_rango() {
        // Seleccionar "ola mu" en "hola mundo" y togglear bold envuelve ese
        // rango exacto, no la palabra bajo el cursor.
        let mut d = doc_with("hola mundo");
        d.col = 1; // ancla en 1
        d.start_selection();
        d.col = 7; // cursor en 7 -> seleccion [1, 7) = "ola mu"
        d.toggle_inline(InlineKind::Bold);
        assert_eq!(d.text(), "h**ola mu**ndo");
        assert!(d.selection_range().is_none()); // se consumio la seleccion
        assert!(d.dirty);
    }

    #[test]
    fn delete_selection_borra_y_reubica() {
        let mut d = doc_with("hola mundo");
        d.col = 4; // ancla en 4
        d.start_selection();
        d.col = 9; // seleccion [4, 9) = " mund"
        d.delete_selection();
        assert_eq!(d.text(), "holao");
        assert_eq!(d.col, 4); // cursor en el inicio del rango borrado
        assert!(d.selection_range().is_none());
        assert!(d.dirty);
    }

    #[test]
    fn selection_range_ordenado() {
        let mut d = doc_with("hola mundo");
        d.col = 6; // ancla en 6
        d.start_selection();
        d.col = 2; // cursor a la izquierda del ancla
        // El rango debe salir ordenado [2, 6), no [6, 2).
        assert_eq!(d.selection_range(), Some(2..6));
    }

    #[test]
    fn selection_range_none_sin_ancla() {
        let d = doc_with("hola");
        assert_eq!(d.selection_range(), None);
    }

    #[test]
    fn selection_range_none_si_vacia() {
        // Ancla y cursor en la misma posicion: rango vacio -> None.
        let mut d = doc_with("hola");
        d.col = 2;
        d.start_selection();
        assert_eq!(d.selection_range(), None);
    }

    #[test]
    fn start_selection_no_reinicia_el_ancla() {
        let mut d = doc_with("hola mundo");
        d.col = 1;
        d.start_selection(); // ancla en 1
        d.col = 4;
        d.start_selection(); // no debe mover el ancla a 4
        assert_eq!(d.selection_range(), Some(1..4));
    }

    #[test]
    fn clear_selection_borra_el_ancla() {
        let mut d = doc_with("hola");
        d.col = 1;
        d.start_selection();
        d.col = 3;
        d.clear_selection();
        assert_eq!(d.selection_range(), None);
    }
}
