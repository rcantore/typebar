//! Undo/redo por snapshots. Aprovecha que `ropey::Rope` clona barato (estructura
//! persistente con sharing): un snapshot guarda una copia del buffer entero mas
//! la posicion del cursor y el ancla de seleccion, sin tener que modelar diffs.
//!
//! El modelo es un **stack lineal clasico** (sin ramas): `undo_stack` guarda los
//! estados previos y `redo_stack` los deshechos. Cualquier edicion nueva vacia
//! el `redo_stack` (la rama deshecha se descarta).
//!
//! Coalescing del tipeo: para que `undo` no sea letra por letra, los
//! `insert_char` consecutivos comparten un solo snapshot (ver `snapshot` y el
//! flag `last_was_insert` en `mod`). Un movimiento o cualquier otra mutacion
//! corta la corrida: el siguiente insert arranca un grupo nuevo.

use ropey::Rope;

use super::Document;

/// Maximo de snapshots en `undo_stack`. Al pasarse se descarta el mas viejo
/// (el del fondo), para acotar la memoria: cada snapshot clona el `Rope`, que
/// comparte nodos pero igual tiene un costo. 500 alcanza de sobra para una
/// sesion de edicion y no es notable en memoria.
const MAX_UNDO: usize = 500;

/// Estado restaurable del documento en un punto del tiempo. El `Rope` se clona
/// (barato en ropey); no guardamos `dirty` ni `mode` a proposito (deshacer
/// siempre deja el documento como modificado y no cambia el modo).
pub struct Snapshot {
    buffer: Rope,
    line: usize,
    col: usize,
    selection_anchor: Option<usize>,
}

impl Document {
    /// Captura el estado actual como `Snapshot` (clona el buffer).
    fn capture(&self) -> Snapshot {
        Snapshot {
            buffer: self.buffer.clone(),
            line: self.line,
            col: self.col,
            selection_anchor: self.selection_anchor,
        }
    }

    /// Restaura el documento desde un `Snapshot`: buffer, cursor y seleccion.
    /// Resincroniza la columna visual deseada y marca `dirty`.
    fn restore(&mut self, snap: Snapshot) {
        self.buffer = snap.buffer;
        self.line = snap.line;
        self.col = snap.col;
        self.selection_anchor = snap.selection_anchor;
        self.sync_preferred();
        self.dirty = true;
    }

    /// Toma un snapshot del estado ACTUAL antes de una mutacion y limpia el
    /// `redo_stack` (cualquier edicion nueva invalida lo deshecho). Lo llaman
    /// todas las mutaciones del documento al tope del metodo.
    pub(super) fn snapshot(&mut self) {
        self.undo_stack.push(self.capture());
        // Capear la pila descartando el mas viejo (el del fondo).
        if self.undo_stack.len() > MAX_UNDO {
            self.undo_stack.remove(0);
        }
        self.redo_stack.clear();
    }

    /// Deshace la ultima edicion: empuja el estado actual a `redo_stack`, saca el
    /// tope de `undo_stack` y lo restaura. No hace nada si no hay historia.
    /// Corta la corrida de tipeo (`last_was_insert = false`).
    pub fn undo(&mut self) {
        self.last_was_insert = false;
        let Some(snap) = self.undo_stack.pop() else {
            return;
        };
        self.redo_stack.push(self.capture());
        self.restore(snap);
    }

    /// Rehace la ultima edicion deshecha: simetrico de `undo` (saca de
    /// `redo_stack`, empuja el actual a `undo_stack`). No hace nada si no hay
    /// nada que rehacer.
    pub fn redo(&mut self) {
        self.last_was_insert = false;
        let Some(snap) = self.redo_stack.pop() else {
            return;
        };
        self.undo_stack.push(self.capture());
        self.restore(snap);
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_support::doc_with;

    #[test]
    fn undo_vuelve_al_estado_previo() {
        let mut d = doc_with("hola");
        d.col = 4;
        d.insert_char('!');
        assert_eq!(d.text(), "hola!");
        d.undo();
        assert_eq!(d.text(), "hola");
        assert_eq!(d.col, 4);
    }

    #[test]
    fn redo_rehace_lo_deshecho() {
        let mut d = doc_with("hola");
        d.col = 4;
        d.insert_char('!');
        d.undo();
        assert_eq!(d.text(), "hola");
        d.redo();
        assert_eq!(d.text(), "hola!");
        assert_eq!(d.col, 5);
    }

    #[test]
    fn varias_ediciones_deshacen_en_orden() {
        // Cada mutacion no-insert es su propio grupo: insert "a", luego un
        // delete_char, luego insert "b" (cortado por el delete). Tres undos
        // vuelven al inicio paso a paso.
        let mut d = doc_with("xyz");
        d.col = 0;
        d.insert_char('a'); // "axyz"
        d.delete_char(); // "ayz" (borra la 'x')
        d.insert_char('b'); // "abyz"
        assert_eq!(d.text(), "abyz");
        d.undo();
        assert_eq!(d.text(), "ayz");
        d.undo();
        assert_eq!(d.text(), "axyz");
        d.undo();
        assert_eq!(d.text(), "xyz");
    }

    #[test]
    fn coalescing_tipeo_un_solo_undo_borra_la_corrida() {
        // Tipear "hola" (4 insert_char seguidos) deshace de una sola, no letra
        // por letra.
        let mut d = doc_with("");
        d.insert_char('h');
        d.insert_char('o');
        d.insert_char('l');
        d.insert_char('a');
        assert_eq!(d.text(), "hola");
        d.undo();
        assert_eq!(d.text(), "");
    }

    #[test]
    fn movimiento_corta_la_corrida_de_tipeo() {
        // Un movimiento entre dos inserts arranca un grupo nuevo: hacen falta
        // dos undos.
        let mut d = doc_with("");
        d.insert_char('a');
        d.insert_char('b');
        d.move_left(); // corta la corrida
        d.move_right();
        d.insert_char('c');
        assert_eq!(d.text(), "abc");
        d.undo();
        assert_eq!(d.text(), "ab");
        d.undo();
        assert_eq!(d.text(), "");
    }

    #[test]
    fn edicion_nueva_limpia_el_redo_stack() {
        let mut d = doc_with("");
        d.insert_char('a');
        d.undo(); // ""
        d.redo(); // "a" (hay redo)
        d.undo(); // "" otra vez
        d.insert_char('b'); // edicion nueva: invalida el redo
        d.redo(); // no debe hacer nada
        assert_eq!(d.text(), "b");
    }

    #[test]
    fn undo_redo_sin_historia_no_rompen() {
        let mut d = doc_with("hola");
        d.undo(); // sin historia: no pasa nada
        assert_eq!(d.text(), "hola");
        d.redo(); // idem
        assert_eq!(d.text(), "hola");
    }

    #[test]
    fn undo_restaura_cursor_y_seleccion() {
        // El borrado de una seleccion se deshace restaurando texto, cursor Y la
        // seleccion que habia antes de borrar (el snapshot se toma con el ancla
        // y el cursor en su sitio).
        let mut d = doc_with("hola mundo");
        d.col = 0;
        d.start_selection(); // ancla en 0
        d.col = 5; // seleccion [0, 5) = "hola "
        d.delete_selection();
        assert_eq!(d.text(), "mundo");
        d.undo();
        assert_eq!(d.text(), "hola mundo");
        assert_eq!(d.col, 5); // cursor donde estaba al borrar
        assert_eq!(d.selection_range(), Some(0..5)); // y la seleccion vuelve
    }
}
