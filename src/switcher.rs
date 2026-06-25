//! Switcher de archivos: un fuzzy finder sobre los archivos del proyecto y los
//! buffers abiertos. Vive aparte de los Overlay de busqueda/reemplazo porque
//! opera a nivel workspace (abre o cambia de buffer), no sobre el documento:
//! `run` lo maneja y, al aceptar, abre el path elegido en el `Workspace`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use std::path::PathBuf;

use crate::fuzzy;
use crate::i18n;
use crate::theme::Theme;

/// Que debe hacer `run` tras pasarle una tecla al switcher.
pub enum SwitcherOutcome {
    /// Seguir abierto (siguio tipeando o navegando).
    Stay,
    /// Cerrar sin elegir (Esc).
    Cancel,
    /// Abrir/cambiar a este path y cerrar (Enter sobre un resultado).
    Accept(PathBuf),
}

/// Estado del switcher: los candidatos, el texto tipeado, los resultados
/// rankeados y cual esta seleccionado.
pub struct Switcher {
    /// Candidatos a filtrar (archivos del proyecto + buffers abiertos, dedup).
    candidates: Vec<PathBuf>,
    /// Representacion string de cada candidato (cacheada para el fuzzy y el render).
    labels: Vec<String>,
    /// Texto tipeado.
    query: String,
    /// Resultados rankeados: indices en `candidates` con su match (para resaltar).
    results: Vec<(usize, fuzzy::FuzzyMatch)>,
    /// Item seleccionado dentro de `results`.
    selected: usize,
}

impl Switcher {
    /// Crea el switcher con sus candidatos y la query vacia (todos matchean).
    pub fn new(candidates: Vec<PathBuf>) -> Self {
        let labels = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut s = Switcher {
            candidates,
            labels,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
        };
        s.recompute();
        s
    }

    /// Recalcula el ranking contra la query y resetea la seleccion al tope.
    fn recompute(&mut self) {
        let refs: Vec<&str> = self.labels.iter().map(|s| s.as_str()).collect();
        self.results = fuzzy::rank(&self.query, &refs);
        self.selected = 0;
    }

    /// Mueve la seleccion `delta` filas, con clamp a los limites.
    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let last = (self.results.len() - 1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Procesa una tecla y dice que hacer. Tipear/borrar refiltra; flechas (o
    /// Ctrl-N/Ctrl-P) navegan; Enter elige; Esc cancela.
    pub fn handle_key(&mut self, key: KeyEvent) -> SwitcherOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => SwitcherOutcome::Cancel,
            KeyCode::Enter => match self.results.get(self.selected) {
                Some(&(ci, _)) => SwitcherOutcome::Accept(self.candidates[ci].clone()),
                None => SwitcherOutcome::Cancel,
            },
            KeyCode::Down => {
                self.move_selection(1);
                SwitcherOutcome::Stay
            }
            KeyCode::Up => {
                self.move_selection(-1);
                SwitcherOutcome::Stay
            }
            KeyCode::Char('n') if ctrl => {
                self.move_selection(1);
                SwitcherOutcome::Stay
            }
            KeyCode::Char('p') if ctrl => {
                self.move_selection(-1);
                SwitcherOutcome::Stay
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute();
                SwitcherOutcome::Stay
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.recompute();
                SwitcherOutcome::Stay
            }
            _ => SwitcherOutcome::Stay,
        }
    }

    /// El texto tipeado (para el prompt del box).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Cantidad de resultados que matchean la query actual.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    /// Lineas de resultados a dibujar (hasta `max_rows`), con scroll para mantener
    /// visible el seleccionado, los chars matcheados acentuados y la fila
    /// seleccionada marcada (prefijo `>` y reverse).
    pub fn result_lines(&self, theme: &Theme, max_rows: usize) -> Vec<Line<'static>> {
        if max_rows == 0 || self.results.is_empty() {
            return Vec::new();
        }
        // Ventana de scroll: que el seleccionado siempre entre en `max_rows`.
        let start = if self.selected >= max_rows {
            self.selected + 1 - max_rows
        } else {
            0
        };
        let end = (start + max_rows).min(self.results.len());

        let accent = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::BOLD);
        let normal = Style::default();

        let mut lines = Vec::with_capacity(end - start);
        for ri in start..end {
            let (ci, m) = &self.results[ri];
            let label = &self.labels[*ci];
            let selected = ri == self.selected;

            // Un prefijo marca el seleccionado (se nota aunque el terminal no
            // pinte el reverse).
            let marker = if selected { "> " } else { "  " };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(marker.to_string(), normal)];
            // Spans char a char: acento si el indice matcheo, normal si no; toda
            // la fila en reverse si esta seleccionada.
            for (i, ch) in label.chars().enumerate() {
                let mut style = if m.indices.contains(&i) {
                    accent
                } else {
                    normal
                };
                if selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(ch.to_string(), style));
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    /// Dibuja el switcher a pantalla completa: un box con borde cuyo titulo es el
    /// prompt + lo tipeado (con `_` de cursor) + el conteo, y adentro la lista
    /// rankeada (con scroll y resaltado del match). Al no setear cursor, ratatui
    /// lo oculta; el `_` del prompt marca donde se tipea.
    pub fn render(&self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        let prompt = i18n::t(i18n::Key::SwitcherPrompt);
        let title = format!(" {prompt} {}_   ({}) ", self.query(), self.result_count());
        let block = Block::bordered().title(title);
        // Alto util dentro del borde (resta 2: arriba y abajo).
        let rows = area.height.saturating_sub(2) as usize;
        // Sin matches: una linea atenuada en vez de un box vacio.
        let lines = if self.result_count() == 0 {
            vec![Line::from(Span::styled(
                i18n::t(i18n::Key::SwitcherEmpty).to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ))]
        } else {
            self.result_lines(theme, rows)
        };
        // `Clear` borra lo que hubiera debajo (el editor) antes de pintar el box.
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).block(block), area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn arranca_con_todos_los_candidatos() {
        let s = Switcher::new(paths(&["src/main.rs", "README.md", "src/fuzzy.rs"]));
        assert_eq!(s.result_count(), 3);
    }

    #[test]
    fn tipear_filtra() {
        let mut s = Switcher::new(paths(&["src/main.rs", "README.md", "src/fuzzy.rs"]));
        s.handle_key(key(KeyCode::Char('f')));
        s.handle_key(key(KeyCode::Char('z')));
        // "fz" solo matchea fuzzy.rs.
        assert_eq!(s.result_count(), 1);
    }

    #[test]
    fn enter_acepta_el_seleccionado() {
        let mut s = Switcher::new(paths(&["a.md", "b.md"]));
        // Bajar a "b.md" y aceptar.
        s.handle_key(key(KeyCode::Down));
        match s.handle_key(key(KeyCode::Enter)) {
            SwitcherOutcome::Accept(p) => assert_eq!(p, PathBuf::from("b.md")),
            _ => panic!("esperaba Accept"),
        }
    }

    #[test]
    fn esc_cancela() {
        let mut s = Switcher::new(paths(&["a.md"]));
        assert!(matches!(
            s.handle_key(key(KeyCode::Esc)),
            SwitcherOutcome::Cancel
        ));
    }

    #[test]
    fn enter_sin_resultados_cancela() {
        let mut s = Switcher::new(paths(&["a.md"]));
        for c in "zzz".chars() {
            s.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(s.result_count(), 0);
        assert!(matches!(
            s.handle_key(key(KeyCode::Enter)),
            SwitcherOutcome::Cancel
        ));
    }

    #[test]
    fn la_seleccion_clampea() {
        let mut s = Switcher::new(paths(&["a.md", "b.md"]));
        // Subir de mas no pasa de 0; bajar de mas no pasa del ultimo.
        s.handle_key(key(KeyCode::Up));
        match s.handle_key(key(KeyCode::Enter)) {
            SwitcherOutcome::Accept(p) => assert_eq!(p, PathBuf::from("a.md")),
            _ => panic!("esperaba Accept"),
        }
        for _ in 0..5 {
            s.handle_key(key(KeyCode::Down));
        }
        match s.handle_key(key(KeyCode::Enter)) {
            SwitcherOutcome::Accept(p) => assert_eq!(p, PathBuf::from("b.md")),
            _ => panic!("esperaba Accept"),
        }
    }
}
