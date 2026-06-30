//! Workspace: los buffers abiertos (uno o varios documentos) y cual esta activo.
//!
//! Desacopla el "multi-archivo" del resto del editor: `draw`, las acciones y los
//! overlays siguen operando sobre UN documento (`active`/`active_mut`), sin saber
//! que puede haber varios. Abrir o cambiar de archivo solo mueve el foco aca.

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

    /// Crea un buffer nuevo y vacio ("new file") y lo activa. El nombre es
    /// "untitled.md" (o "untitled-N.md" si ese ya esta abierto, para no chocar con
    /// otro sin titulo); el archivo recien se crea al guardar. Arranca en
    /// `initial_mode` (el del preset activo).
    pub fn new_buffer(&mut self, initial_mode: Mode) {
        let name = self.untitled_name();
        let mut doc = Document::empty(&name);
        doc.mode = initial_mode;
        self.docs.push(doc);
        self.active = self.docs.len() - 1;
    }

    /// Devuelve un nombre "untitled" que no este ya abierto, con sufijo numerico
    /// si hace falta (`untitled.md`, `untitled-2.md`, ...).
    fn untitled_name(&self) -> String {
        let is_open = |name: &str| {
            self.docs
                .iter()
                .any(|d| d.path.as_path() == Path::new(name))
        };
        if !is_open("untitled.md") {
            return "untitled.md".to_string();
        }
        // Empezamos en 2 (el sin sufijo es el "1"); el rango es practico, nunca se
        // agota en un uso real.
        (2..)
            .map(|n| format!("untitled-{n}.md"))
            .find(|name| !is_open(name))
            .expect("siempre hay un nombre libre")
    }

    /// Mueve el foco al buffer `index`. No-op si esta fuera de rango.
    pub fn switch_to(&mut self, index: usize) {
        if index < self.docs.len() {
            self.active = index;
        }
    }

    /// Enfoca el buffer siguiente, con wraparound (del ultimo vuelve al primero).
    pub fn next_buffer(&mut self) {
        self.active = (self.active + 1) % self.docs.len();
    }

    /// Enfoca el buffer anterior, con wraparound (del primero salta al ultimo).
    pub fn prev_buffer(&mut self) {
        let n = self.docs.len();
        self.active = (self.active + n - 1) % n;
    }

    /// Cierra el buffer activo. Mantiene el invariante de que `docs` nunca queda
    /// vacio: si era el unico buffer, lo reemplaza por uno nuevo y vacio
    /// (`untitled.md`) en `initial_mode`. Con varios, el foco se queda en el mismo
    /// indice (que ahora apunta al que era el siguiente) salvo que se cerrara el
    /// ultimo, en cuyo caso baja al previo. No decide sobre cambios sin guardar:
    /// eso lo resuelve `run` (que confirma antes de llamar aca si el doc esta dirty).
    pub fn close_active(&mut self, initial_mode: Mode) {
        self.docs.remove(self.active);
        if self.docs.is_empty() {
            let mut doc = Document::empty("untitled.md");
            doc.mode = initial_mode;
            self.docs.push(doc);
            self.active = 0;
        } else if self.active >= self.docs.len() {
            self.active = self.docs.len() - 1;
        }
    }

    /// Paths de todos los buffers, en orden de apertura.
    pub fn paths(&self) -> impl Iterator<Item = &Path> {
        self.docs.iter().map(|d| d.path.as_path())
    }

    /// Itera los buffers abiertos como `(path, unsaved)`, en orden de apertura. Lo
    /// usa el switcher para listarlos primero y marcar con `[+]` los que no estan
    /// a salvo en disco (cambios sin guardar O untitled/nunca guardado), mismo
    /// criterio que la status bar (ver `Document::unsaved`).
    pub fn buffers(&self) -> impl Iterator<Item = (&Path, bool)> {
        self.docs.iter().map(|d| (d.path.as_path(), d.unsaved()))
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
    fn new_buffer_crea_vacio_lo_enfoca_y_desambigua_el_nombre() {
        let mut ws = Workspace::new(doc_with("hola"));
        ws.new_buffer(Mode::Insert);
        assert_eq!(ws.count(), 2);
        assert_eq!(ws.active_index(), 1);
        assert_eq!(ws.active().text(), ""); // vacio
        assert_eq!(ws.active().path.to_string_lossy(), "untitled.md");
        // Un segundo "new" no pisa el primer untitled: usa un sufijo.
        ws.new_buffer(Mode::Insert);
        assert_eq!(ws.active().path.to_string_lossy(), "untitled-2.md");
    }

    #[test]
    fn next_y_prev_buffer_ciclan_con_wraparound() {
        let mut ws = ws_from(vec![
            doc_at("a.md", ""),
            doc_at("b.md", ""),
            doc_at("c.md", ""),
        ]);
        assert_eq!(ws.active_index(), 0);
        ws.next_buffer();
        assert_eq!(ws.active_index(), 1);
        ws.next_buffer();
        assert_eq!(ws.active_index(), 2);
        ws.next_buffer(); // del ultimo vuelve al primero
        assert_eq!(ws.active_index(), 0);
        ws.prev_buffer(); // del primero salta al ultimo
        assert_eq!(ws.active_index(), 2);
    }

    #[test]
    fn close_active_enfoca_el_siguiente_y_decrece_en_el_ultimo() {
        let mut ws = ws_from(vec![
            doc_at("a.md", "a"),
            doc_at("b.md", "b"),
            doc_at("c.md", "c"),
        ]);
        ws.switch_to(1); // foco en b
        ws.close_active(Mode::Insert);
        // Cerrado b: el indice 1 ahora apunta a c (el que era el siguiente).
        assert_eq!(ws.count(), 2);
        assert_eq!(ws.active_index(), 1);
        assert_eq!(ws.active().text(), "c");
        // Cerrar el ultimo (c): el foco baja al previo (a).
        ws.close_active(Mode::Insert);
        assert_eq!(ws.count(), 1);
        assert_eq!(ws.active_index(), 0);
        assert_eq!(ws.active().text(), "a");
    }

    #[test]
    fn close_active_sobre_el_unico_buffer_lo_reemplaza_por_uno_vacio() {
        let mut ws = Workspace::new(doc_at("solo.md", "contenido"));
        ws.close_active(Mode::Normal);
        // El invariante se mantiene: sigue habiendo un buffer, vacio y untitled,
        // en el modo del preset.
        assert_eq!(ws.count(), 1);
        assert_eq!(ws.active_index(), 0);
        assert_eq!(ws.active().text(), "");
        assert_eq!(ws.active().path.to_string_lossy(), "untitled.md");
        assert_eq!(ws.active().mode, Mode::Normal);
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
