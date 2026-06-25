//! Workspace: los buffers abiertos (uno o varios documentos) y cual esta activo.
//!
//! Desacopla el "multi-archivo" del resto del editor: `draw`, las acciones y los
//! overlays siguen operando sobre UN documento (`active`/`active_mut`), sin saber
//! que puede haber varios. Abrir o cambiar de archivo solo mueve el foco aca.

// `switch_to`/`count`/`active_index` todavia no tienen consumidor en la UI
// (quedan para la navegacion entre buffers y un indicador en la status bar); se
// quita este allow cuando se cableen. El resto (new/active/open_or_switch/paths)
// ya lo usa `run`.
#![allow(dead_code)]

use std::io;
use std::path::Path;

use crate::document::{Document, Mode};

/// Conjunto de documentos abiertos con un foco activo. Invariante: `docs` nunca
/// esta vacio y `active` siempre es un indice valido.
pub struct Workspace {
    docs: Vec<Document>,
    active: usize,
}

impl Workspace {
    /// Crea el workspace con el documento inicial ya abierto y enfocado.
    pub fn new(initial: Document) -> Self {
        Workspace {
            docs: vec![initial],
            active: 0,
        }
    }

    /// El documento activo (lectura).
    pub fn active(&self) -> &Document {
        &self.docs[self.active]
    }

    /// El documento activo (mutable): por aca pasan la edicion y los overlays.
    pub fn active_mut(&mut self) -> &mut Document {
        &mut self.docs[self.active]
    }

    /// Cantidad de buffers abiertos (siempre >= 1).
    pub fn count(&self) -> usize {
        self.docs.len()
    }

    /// Indice (0-based) del buffer activo.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Abre `path` como buffer nuevo y lo activa. Si ya hay un buffer con ese
    /// path, solo mueve el foco a ese (no reabre ni descarta cambios). El buffer
    /// nuevo arranca en `initial_mode` (el del preset activo). Devuelve error de
    /// IO solo cuando hay que abrir un archivo nuevo y falla.
    pub fn open_or_switch(&mut self, path: impl AsRef<Path>, initial_mode: Mode) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(i) = self.docs.iter().position(|d| d.path.as_path() == path) {
            self.active = i;
            return Ok(());
        }
        let mut doc = Document::open(path)?;
        doc.mode = initial_mode;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
        Ok(())
    }

    /// Mueve el foco al buffer `index`. No-op si esta fuera de rango.
    pub fn switch_to(&mut self, index: usize) {
        if index < self.docs.len() {
            self.active = index;
        }
    }

    /// Paths de todos los buffers, en orden de apertura.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.docs.iter().map(|d| d.path.as_path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::doc_with;
    use std::path::PathBuf;

    /// Construye un Document en memoria con un path concreto (sin tocar el disco).
    fn doc_at(path: &str, text: &str) -> Document {
        let mut d = doc_with(text);
        d.path = PathBuf::from(path);
        d
    }

    /// Constructor de test con varios docs (el de produccion arranca con uno y
    /// suma via `open_or_switch`, que toca el disco).
    fn ws_from(docs: Vec<Document>) -> Workspace {
        Workspace { docs, active: 0 }
    }

    #[test]
    fn arranca_con_un_buffer_enfocado() {
        let ws = Workspace::new(doc_with("hola"));
        assert_eq!(ws.count(), 1);
        assert_eq!(ws.active_index(), 0);
        assert_eq!(ws.active().text(), "hola");
    }

    #[test]
    fn switch_to_respeta_los_limites() {
        let mut ws = ws_from(vec![doc_at("a.md", "a"), doc_at("b.md", "b")]);
        ws.switch_to(1);
        assert_eq!(ws.active_index(), 1);
        // Fuera de rango: no-op (no rompe el invariante).
        ws.switch_to(99);
        assert_eq!(ws.active_index(), 1);
    }

    #[test]
    fn open_or_switch_sobre_path_abierto_dedupea_y_mueve_foco() {
        let mut ws = ws_from(vec![doc_at("a.md", "a"), doc_at("b.md", "b")]);
        ws.switch_to(1); // foco en b
        // Reabrir "a.md" (ya abierto) no agrega buffer: solo cambia el foco.
        ws.open_or_switch("a.md", Mode::Insert).unwrap();
        assert_eq!(ws.count(), 2);
        assert_eq!(ws.active_index(), 0);
    }

    #[test]
    fn paths_lista_en_orden() {
        let ws = ws_from(vec![doc_at("a.md", "a"), doc_at("b.md", "b")]);
        let paths: Vec<_> = ws
            .paths()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        assert_eq!(paths, vec!["a.md", "b.md"]);
    }

    #[test]
    fn active_mut_edita_el_buffer_enfocado() {
        let mut ws = ws_from(vec![doc_at("a.md", ""), doc_at("b.md", "")]);
        ws.switch_to(0);
        ws.active_mut().insert_char('x');
        assert_eq!(ws.active().text(), "x");
        // El otro buffer queda intacto.
        ws.switch_to(1);
        assert_eq!(ws.active().text(), "");
    }
}
