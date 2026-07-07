//! Theme picker: overlay para elegir el theme EN RUNTIME, con preview EN VIVO
//! (mientras navegas, el editor entero se pinta con el theme resaltado) y un
//! marcador del theme actual. Reusa la mecanica de los otros pickers
//! (`picker_block`/`picker_popup`/`picker_content`/`picker_prompt`, scroll, dim):
//! lo que cambia es el contenido (temas en vez de archivos/comandos).
//!
//! El picker no aplica nada por si mismo: expone el theme resaltado (`highlighted`)
//! para que `run` lo use al dibujar cada frame (preview), y al aceptar devuelve el
//! id elegido para que `run` lo fije como theme base. Al cancelar, `run` vuelve
//! solo al theme que estaba (no guardo estado del anterior aca).

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::switcher::{
    dim_area, picker_block, picker_content, picker_popup, picker_prompt, scroll_window,
};
use crate::theme::{PICKER_THEMES, Theme};
use typebar_core::fuzzy;
use typebar_core::i18n::{self, Key};

/// Que debe hacer `run` tras pasarle una tecla al theme picker.
pub enum ThemeOutcome {
    /// Seguir abierto (siguio tipeando o navegando; `run` sigue previewando).
    Stay,
    /// Cerrar sin elegir (Esc): `run` vuelve al theme que estaba.
    Cancel,
    /// Fijar este theme (por su id) y cerrar (Enter sobre un resultado).
    Accept(&'static str),
}

/// Estado del theme picker: el catalogo de temas, el id del que estaba activo al
/// abrir (para el marcador "actual"), el texto tipeado, los resultados rankeados y
/// el seleccionado.
pub struct ThemePicker {
    /// Catalogo `(id, nombre visible)` de `PICKER_THEMES`.
    entries: &'static [(&'static str, &'static str)],
    /// Nombres visibles, para el fuzzy (paralelo a `entries`).
    labels: Vec<String>,
    /// Id del theme activo al abrir: lleva el marcador "actual".
    current: String,
    /// Texto tipeado.
    query: String,
    /// Resultados rankeados: indices en `entries` con su match (para resaltar).
    results: Vec<(usize, fuzzy::FuzzyMatch)>,
    /// Item seleccionado dentro de `results`.
    selected: usize,
}

impl ThemePicker {
    /// Crea el picker con el catalogo built-in y marca `current` como el activo.
    /// La query arranca vacia (todos matchean) y la seleccion sobre el theme
    /// actual, asi al abrir ya estas parado en el que tenes (y el preview no salta).
    pub fn new(current: &str) -> Self {
        let labels = PICKER_THEMES.iter().map(|(_, d)| d.to_string()).collect();
        let mut p = ThemePicker {
            entries: PICKER_THEMES,
            labels,
            current: current.to_string(),
            query: String::new(),
            results: Vec::new(),
            selected: 0,
        };
        p.recompute();
        // Parar la seleccion sobre el theme actual (si esta entre los resultados).
        if let Some(pos) = p
            .results
            .iter()
            .position(|(ei, _)| p.entries[*ei].0 == p.current)
        {
            p.selected = pos;
        }
        p
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
    pub fn handle_key(&mut self, key: KeyEvent) -> ThemeOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => ThemeOutcome::Cancel,
            KeyCode::Enter => match self.results.get(self.selected) {
                Some(&(ei, _)) => ThemeOutcome::Accept(self.entries[ei].0),
                None => ThemeOutcome::Cancel,
            },
            KeyCode::Down => {
                self.move_selection(1);
                ThemeOutcome::Stay
            }
            KeyCode::Up => {
                self.move_selection(-1);
                ThemeOutcome::Stay
            }
            KeyCode::Char('n') if ctrl => {
                self.move_selection(1);
                ThemeOutcome::Stay
            }
            KeyCode::Char('p') if ctrl => {
                self.move_selection(-1);
                ThemeOutcome::Stay
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute();
                ThemeOutcome::Stay
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.recompute();
                ThemeOutcome::Stay
            }
            _ => ThemeOutcome::Stay,
        }
    }

    /// El texto tipeado (para el prompt del box).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Cantidad de temas que matchean la query actual.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    /// Id del theme resaltado ahora mismo (el seleccionado), o `None` si nada
    /// matchea. `run` lo usa para el preview en vivo.
    pub fn highlighted_id(&self) -> Option<&'static str> {
        self.results
            .get(self.selected)
            .map(|&(ei, _)| self.entries[ei].0)
    }

    /// El theme resaltado ya construido, para que `run` dibuje el frame con el
    /// preview. `None` si nada matchea (ahi `run` deja el theme que estaba).
    pub fn highlighted_theme(&self) -> Option<Theme> {
        self.highlighted_id().map(Theme::by_name)
    }

    /// Lineas de resultados a dibujar (hasta `max_rows`), con scroll para mantener
    /// visible el seleccionado: el nombre con los chars del match acentuados, un
    /// marcador "actual" a la derecha en el theme activo, y la fila seleccionada en
    /// reverse. `width` es el ancho util del box para alinear el marcador.
    pub fn result_lines(&self, theme: &Theme, max_rows: usize, width: usize) -> Vec<Line<'static>> {
        let std::ops::Range { start, end } =
            scroll_window(self.selected, self.results.len(), max_rows);
        if start == end {
            return Vec::new();
        }

        let accent = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::BOLD);
        let normal = Style::default();
        let current_style = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::DIM);
        let current_tag = format!("● {}", i18n::t(Key::ThemeCurrent));

        let mut lines = Vec::with_capacity(end - start);
        for ri in start..end {
            let (ei, m) = &self.results[ri];
            let (id, name) = self.entries[*ei];
            let selected = ri == self.selected;

            let marker = if selected { "> " } else { "  " };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(marker.to_string(), normal)];
            for (i, ch) in name.chars().enumerate() {
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
            // Marcador "actual" alineado a la derecha en el theme activo.
            if id == self.current {
                let used = 2 + name.chars().count();
                let tag_len = current_tag.chars().count();
                let pad = width.saturating_sub(used + tag_len).max(1);
                let (mut gap_style, mut tag_style) = (normal, current_style);
                if selected {
                    gap_style = gap_style.add_modifier(Modifier::REVERSED);
                    tag_style = tag_style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(" ".repeat(pad), gap_style));
                spans.push(Span::styled(current_tag.clone(), tag_style));
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    /// Dibuja el theme picker como popup centrado flotante, con el mismo look que
    /// la paleta y el switcher (borde redondeado con acento, padding, prompt y
    /// footer internos, fondo atenuado). El `theme` que recibe es el del PREVIEW
    /// (el resaltado), asi el propio box se pinta con el theme que estas mirando.
    pub fn render(&self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        let (popup, inner_width, shown) = picker_popup(area, self.result_count());
        let rows = if self.result_count() == 0 {
            vec![Line::from(Span::styled(
                i18n::t(Key::SwitcherEmpty).to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ))]
        } else {
            self.result_lines(theme, shown, inner_width)
        };
        let prompt = picker_prompt(
            theme,
            i18n::t(Key::ThemePickerPrompt),
            self.query(),
            self.result_count(),
        );
        let content = picker_content(prompt, rows);
        dim_area(frame.buffer_mut(), area);
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(content).block(picker_block(theme)), popup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn arranca_parado_en_el_theme_actual() {
        // Al abrir, la seleccion cae sobre el theme activo (no en el tope), asi el
        // preview no salta apenas se abre.
        let p = ThemePicker::new("nord");
        assert_eq!(p.highlighted_id(), Some("nord"));
    }

    #[test]
    fn arranca_con_todos_los_temas() {
        let p = ThemePicker::new("frappe");
        assert_eq!(p.result_count(), PICKER_THEMES.len());
    }

    #[test]
    fn tipear_filtra_y_preview_sigue_al_seleccionado() {
        let mut p = ThemePicker::new("frappe");
        for c in "drac".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        // "drac" deberia dejar solo Dracula, y el preview apuntar a el.
        assert_eq!(p.result_count(), 1);
        assert_eq!(p.highlighted_id(), Some("dracula"));
    }

    #[test]
    fn enter_acepta_el_id_resaltado() {
        let mut p = ThemePicker::new("frappe");
        for c in "tokyo".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        match p.handle_key(key(KeyCode::Enter)) {
            ThemeOutcome::Accept(id) => assert_eq!(id, "tokyo-night"),
            _ => panic!("esperaba Accept(tokyo-night)"),
        }
    }

    #[test]
    fn esc_cancela() {
        let mut p = ThemePicker::new("frappe");
        assert!(matches!(
            p.handle_key(key(KeyCode::Esc)),
            ThemeOutcome::Cancel
        ));
    }

    #[test]
    fn el_marcador_actual_va_en_el_theme_activo() {
        // La fila del theme activo lleva el tag "actual"; otra no. Chequeamos via el
        // texto plano de las lineas.
        let theme = Theme::frappe();
        let p = ThemePicker::new("gruvbox");
        let lines = p.result_lines(&theme, 20, 40);
        let text: Vec<String> = lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect();
        let tag = i18n::t(Key::ThemeCurrent);
        let gruvbox_line = text.iter().find(|t| t.contains("Gruvbox")).unwrap();
        assert!(
            gruvbox_line.contains(tag),
            "Gruvbox deberia marcar el actual"
        );
        let dracula_line = text.iter().find(|t| t.contains("Dracula")).unwrap();
        assert!(!dracula_line.contains(tag), "Dracula no es el actual");
    }
}
