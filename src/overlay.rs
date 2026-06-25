//! Overlay de busqueda / reemplazo sobre el editor.
//!
//! Mientras un overlay vive, las teclas las consume el overlay (no el
//! documento): se tipea el termino, se navega entre coincidencias y se confirma
//! o cancela. El tipo es autocontenido: solo depende de `search`, el `Document`
//! y los labels de `i18n`. El render del minibuffer y el calculo de los rangos a
//! resaltar tambien viven aca; `draw` (en `main`) solo los consume.

use std::ops::Range;

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::document::Document;
use crate::i18n;
use crate::search;

/// Estado de un overlay activo sobre el editor. Mientras vive, las teclas las
/// consume el overlay (no el documento): se tipea el termino, se navega entre
/// coincidencias y se confirma o cancela.
pub(crate) enum Overlay {
    /// Busqueda incremental: el cursor salta a la coincidencia a medida que se
    /// tipea; Enter avanza a la siguiente; Esc cierra dejando el cursor ahi.
    Search { query: String },
    /// Buscar y reemplazar: dos campos (buscar / reemplazar) que se alternan con
    /// Tab; Enter reemplaza TODAS las ocurrencias; Esc cancela.
    Replace {
        find: String,
        replacement: String,
        /// Campo con foco (a cual van las teclas tipeadas).
        editing_replacement: bool,
    },
}

impl Overlay {
    /// Crea el overlay de busqueda vacio. Arranca sin termino: el resaltado
    /// aparece al primer caracter tipeado.
    pub(crate) fn new_search(_doc: &Document) -> Self {
        Overlay::Search {
            query: String::new(),
        }
    }

    /// Crea el overlay de reemplazo vacio, con el foco en el campo "buscar".
    pub(crate) fn new_replace() -> Self {
        Overlay::Replace {
            find: String::new(),
            replacement: String::new(),
            editing_replacement: false,
        }
    }

    /// Procesa una tecla. Devuelve `true` si el overlay debe cerrarse.
    pub(crate) fn handle_key(&mut self, doc: &mut Document, key: KeyEvent) -> bool {
        match self {
            Overlay::Search { query } => Self::handle_search_key(query, doc, key),
            Overlay::Replace {
                find,
                replacement,
                editing_replacement,
            } => Self::handle_replace_key(find, replacement, editing_replacement, doc, key),
        }
    }

    /// Teclas del overlay de busqueda. Tipear/borrar recomputa y salta a la
    /// coincidencia mas cercana; Enter avanza; Esc cierra.
    fn handle_search_key(query: &mut String, doc: &mut Document, key: KeyEvent) -> bool {
        match key.code {
            KeyCode::Esc => return true,
            KeyCode::Enter => {
                // Avanzar a la coincidencia siguiente a la posicion actual + 1,
                // para no quedar clavado en la misma.
                let matches = search::find_all(&doc.text(), query);
                if !matches.is_empty()
                    && let Some(idx) = search::next_match_from(&matches, doc.cursor_byte() + 1)
                {
                    doc.move_cursor_to_byte(matches[idx].start);
                }
                return false;
            }
            KeyCode::Backspace => {
                query.pop();
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                query.push(c);
            }
            _ => return false,
        }
        // Tras editar el termino, saltar a la primer coincidencia desde el cursor.
        let matches = search::find_all(&doc.text(), query);
        if !matches.is_empty()
            && let Some(idx) = search::next_match_from(&matches, doc.cursor_byte())
        {
            doc.move_cursor_to_byte(matches[idx].start);
        }
        false
    }

    /// Teclas del overlay de reemplazo. Tab alterna campo, Enter reemplaza todo,
    /// Esc cancela.
    fn handle_replace_key(
        find: &mut String,
        replacement: &mut String,
        editing_replacement: &mut bool,
        doc: &mut Document,
        key: KeyEvent,
    ) -> bool {
        match key.code {
            KeyCode::Esc => true,
            KeyCode::Tab => {
                *editing_replacement = !*editing_replacement;
                false
            }
            KeyCode::Enter => {
                if !find.is_empty() {
                    doc.replace_all(find, replacement);
                }
                true
            }
            KeyCode::Backspace => {
                if *editing_replacement {
                    replacement.pop();
                } else {
                    find.pop();
                }
                false
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                if *editing_replacement {
                    replacement.push(c);
                } else {
                    find.push(c);
                }
                false
            }
            _ => false,
        }
    }

    /// Rangos (en bytes) a resaltar segun el overlay: las coincidencias del
    /// termino de busqueda, o del campo "buscar" en reemplazo. Cual es la
    /// coincidencia "actual" lo decide `draw` segun la posicion del cursor.
    pub(crate) fn highlights(&self, text: &str) -> Vec<Range<usize>> {
        match self {
            Overlay::Search { query } => search::find_all(text, query),
            Overlay::Replace { find, .. } => search::find_all(text, find),
        }
    }

    /// Linea del minibuffer mostrada en lugar de la status bar.
    pub(crate) fn minibuffer(&self) -> Line<'static> {
        let style = Style::default().add_modifier(Modifier::REVERSED);
        match self {
            Overlay::Search { query } => Line::from(Span::styled(
                format!(" {} {query}_ ", i18n::t(i18n::Key::MinibufferSearchPrompt)),
                style,
            )),
            Overlay::Replace {
                find,
                replacement,
                editing_replacement,
            } => {
                // Un marcador `_` en el campo con foco indica donde se tipea.
                let (f, r) = if *editing_replacement {
                    (find.clone(), format!("{replacement}_"))
                } else {
                    (format!("{find}_"), replacement.clone())
                };
                Line::from(Span::styled(
                    format!(
                        " {} {f} → {r}  ({}) ",
                        i18n::t(i18n::Key::MinibufferReplacePrompt),
                        i18n::t(i18n::Key::MinibufferReplaceHelp),
                    ),
                    style,
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::test_support::doc_with;

    /// KeyEvent simple para los tests del overlay.
    fn k(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    /// Tipea una cadena en el overlay, tecla por tecla.
    fn type_str(ov: &mut Overlay, doc: &mut Document, s: &str) {
        for c in s.chars() {
            ov.handle_key(doc, k(KeyCode::Char(c)));
        }
    }

    #[test]
    fn overlay_busqueda_salta_a_la_coincidencia() {
        let mut doc = doc_with("x foo y foo");
        let mut ov = Overlay::new_search(&doc);
        type_str(&mut ov, &mut doc, "foo");
        // El cursor salta al primer "foo" (byte 2).
        assert_eq!(doc.cursor_byte(), 2);
    }

    #[test]
    fn overlay_busqueda_enter_avanza_y_esc_cierra() {
        let mut doc = doc_with("x foo y foo");
        let mut ov = Overlay::new_search(&doc);
        type_str(&mut ov, &mut doc, "foo");
        assert_eq!(doc.cursor_byte(), 2);
        // Enter avanza a la coincidencia siguiente (byte 8).
        assert!(!ov.handle_key(&mut doc, k(KeyCode::Enter)));
        assert_eq!(doc.cursor_byte(), 8);
        // Esc cierra el overlay.
        assert!(ov.handle_key(&mut doc, k(KeyCode::Esc)));
    }

    #[test]
    fn overlay_busqueda_backspace_recomputa() {
        let mut doc = doc_with("abc abx");
        let mut ov = Overlay::new_search(&doc);
        type_str(&mut ov, &mut doc, "abx"); // matchea solo "abx" (1 coincidencia)
        assert_eq!(ov.highlights(&doc.text()).len(), 1);
        // Borrar la 'x' ensancha el termino a "ab": ahora matchea en ambas
        // palabras (2 coincidencias), probando que backspace recomputa.
        ov.handle_key(&mut doc, k(KeyCode::Backspace));
        assert_eq!(ov.highlights(&doc.text()).len(), 2);
    }

    #[test]
    fn overlay_reemplazo_tab_y_enter_reemplaza_todo() {
        let mut doc = doc_with("a a a");
        let mut ov = Overlay::new_replace();
        type_str(&mut ov, &mut doc, "a"); // campo "buscar"
        ov.handle_key(&mut doc, k(KeyCode::Tab)); // foco -> "reemplazar"
        type_str(&mut ov, &mut doc, "bb");
        // Enter reemplaza todo y cierra.
        assert!(ov.handle_key(&mut doc, k(KeyCode::Enter)));
        assert_eq!(doc.text(), "bb bb bb");
    }

    #[test]
    fn overlay_reemplazo_esc_cancela_sin_tocar_el_texto() {
        let mut doc = doc_with("a a a");
        let mut ov = Overlay::new_replace();
        type_str(&mut ov, &mut doc, "a");
        assert!(ov.handle_key(&mut doc, k(KeyCode::Esc)));
        assert_eq!(doc.text(), "a a a"); // intacto
    }
}
