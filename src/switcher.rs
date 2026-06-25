//! Switcher de archivos: un fuzzy finder sobre los archivos del proyecto y los
//! buffers abiertos. Vive aparte de los Overlay de busqueda/reemplazo porque
//! opera a nivel workspace (abre o cambia de buffer), no sobre el documento:
//! `run` lo maneja y, al aceptar, abre el path elegido en el `Workspace`.

use ratatui::Frame;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph};

use std::ops::Range;
use std::path::PathBuf;

use crate::fuzzy;
use crate::i18n;
use crate::theme::Theme;

/// Ventana de scroll de un picker: el rango `[start, end)` de filas a dibujar
/// para que el item `selected` siempre entre dentro de `max_rows`, dado un total
/// de `len` resultados. Se comparte entre el switcher y la paleta porque la
/// logica de scroll es identica en ambos (lo que difiere es como pintan cada
/// fila, que queda en cada uno). Con `selected` por debajo de `max_rows` la
/// ventana arranca en 0; pasado ese punto se corre para dejar el seleccionado en
/// la ultima fila visible. `end` se clampea a `len`.
pub(crate) fn scroll_window(selected: usize, len: usize, max_rows: usize) -> Range<usize> {
    if max_rows == 0 || len == 0 {
        return 0..0;
    }
    let start = if selected >= max_rows {
        selected + 1 - max_rows
    } else {
        0
    };
    let end = (start + max_rows).min(len);
    start..end
}

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
    /// visible el seleccionado. `width` es el ancho interno del box (menos los
    /// bordes), al que se padea la fila seleccionada.
    ///
    /// Cada fila pinta el path con jerarquia: el directorio (todo hasta el ultimo
    /// `/`, inclusive) va atenuado y el nombre del archivo resaltado, para que la
    /// vista lea como una lista de archivos y no como rutas planas. Encima de eso
    /// se acentuan los chars que matchearon el fuzzy (mapeando los indices, que
    /// son sobre el label completo con el directorio incluido). La fila
    /// seleccionada lleva una barra de fondo a todo el ancho: se rellena con
    /// espacios hasta `width` para que el resalte cubra la fila entera, no solo el
    /// texto (si `width` es 0, no se padea).
    fn result_lines_padded(
        &self,
        theme: &Theme,
        max_rows: usize,
        width: usize,
    ) -> Vec<Line<'static>> {
        // Ventana de scroll: que el seleccionado siempre entre en `max_rows`
        // (logica compartida con la paleta).
        let Range { start, end } = scroll_window(self.selected, self.results.len(), max_rows);
        if start == end {
            return Vec::new();
        }

        // Acento del match (lo que el usuario tipeo): se pinta ENCIMA de la
        // jerarquia dir/archivo.
        let accent = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::BOLD);
        // Directorio: atenuado (DIM + color de marker) para que pese menos que el
        // nombre del archivo.
        let dir_style = Style::default()
            .fg(theme.marker)
            .add_modifier(Modifier::DIM);
        // Nombre del archivo: brillante y en negrita, es lo que mas importa.
        let file_style = Style::default().add_modifier(Modifier::BOLD);
        // Barra de seleccion: fondo del color de seleccion del theme, a todo ancho.
        let sel_bg = Style::default().bg(theme.selection_bg);

        let mut lines = Vec::with_capacity(end - start);
        for ri in start..end {
            let (ci, m) = &self.results[ri];
            let label = &self.labels[*ci];
            let selected = ri == self.selected;

            // Largo en chars del path (para mapear indices y padear la barra). Se
            // parte por el ULTIMO `/`: lo de antes (inclusive) es directorio, lo
            // de despues es el nombre del archivo. Sin `/`, todo es nombre.
            let total_chars = label.chars().count();
            let dir_chars = match label.rfind('/') {
                // `+1` para incluir la barra dentro del directorio.
                Some(pos) => label[..=pos].chars().count(),
                None => 0,
            };

            // Un prefijo marca el seleccionado (se nota aunque el terminal no
            // pinte el fondo).
            let marker = if selected { "> " } else { "  " };
            let mut row = Style::default();
            if selected {
                row = sel_bg;
            }
            let mut spans: Vec<Span<'static>> = vec![Span::styled(marker.to_string(), row)];

            // Spans char a char. Base segun jerarquia (dir atenuado / archivo
            // brillante), acento si el char matcheo, y el fondo de la barra si la
            // fila esta seleccionada (preservando fg/modificadores).
            for (i, ch) in label.chars().enumerate() {
                let mut style = if m.indices.contains(&i) {
                    accent
                } else if i < dir_chars {
                    dir_style
                } else {
                    file_style
                };
                if selected {
                    style = style.bg(theme.selection_bg);
                }
                spans.push(Span::styled(ch.to_string(), style));
            }

            // Barra a todo el ancho: rellenar con espacios (con el fondo de la
            // seleccion) hasta `width`, contando el marker y el path ya dibujados.
            if selected {
                let used = marker.chars().count() + total_chars;
                if width > used {
                    let pad = " ".repeat(width - used);
                    spans.push(Span::styled(pad, sel_bg));
                }
            }

            lines.push(Line::from(spans));
        }
        lines
    }

    /// Dibuja el switcher como un popup CENTRADO flotante: limpia toda la pantalla
    /// (afuera del box queda en blanco) y pinta un box con borde en el centro, con
    /// el prompt + lo tipeado (con `_` de cursor) + el conteo en el titulo y la
    /// lista rankeada adentro. Al no setear cursor, ratatui lo oculta; el `_` del
    /// prompt marca donde se tipea.
    pub fn render(&self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        // Popup centrado: ~70% del ancho/alto, acotado a la terminal (y a un minimo
        // usable). En pantallas chicas cae al tamano disponible.
        let w = (area.width * 7 / 10).clamp(40.min(area.width), area.width);
        let h = (area.height * 7 / 10).clamp(3.min(area.height), area.height);
        let popup = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };

        let prompt = i18n::t(i18n::Key::SwitcherPrompt);
        let title = format!(" {prompt} {}_   ({}) ", self.query(), self.result_count());
        let block = Block::bordered().title(title);
        // Alto/ancho utiles dentro del borde del box (restan 2: ambos lados).
        let rows = popup.height.saturating_sub(2) as usize;
        let inner_width = popup.width.saturating_sub(2) as usize;
        // Sin matches: una linea atenuada en vez de un box vacio.
        let lines = if self.result_count() == 0 {
            vec![Line::from(Span::styled(
                i18n::t(i18n::Key::SwitcherEmpty).to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ))]
        } else {
            // Ancho real del box (no el del contenido) para que la barra de
            // seleccion llegue al borde derecho.
            self.result_lines_padded(theme, rows, inner_width)
        };
        // `Clear` borra TODA la pantalla (el editor de fondo); despues el box.
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).block(block), popup);
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

    /// Texto plano de una `Line` (concatena el contenido de todos los spans).
    fn line_text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn el_path_se_parte_en_directorio_y_archivo() {
        // Con un path "src/main.rs" el directorio ("src/") debe ir atenuado y el
        // nombre ("main.rs") brillante. Chequeamos via los estilos de los spans:
        // los spans del directorio llevan el modificador DIM, los del archivo BOLD
        // (y sin DIM). El marker inicial (2 spans/celdas) se saltea.
        let theme = Theme::frappe();
        let s = Switcher::new(paths(&["src/main.rs"]));
        let lines = s.result_lines_padded(&theme, 10, 20);
        let line = &lines[0];
        // spans[0] es el marker; los chars del path arrancan en spans[1].
        // "src/" son 4 chars -> spans[1..=4] son el directorio.
        for span in &line.spans[1..=4] {
            assert!(
                span.style.add_modifier.contains(Modifier::DIM),
                "el directorio deberia ir atenuado (DIM)"
            );
        }
        // "main.rs" empieza en spans[5]; deberia ir en BOLD y sin DIM.
        assert!(
            line.spans[5].style.add_modifier.contains(Modifier::BOLD)
                && !line.spans[5].style.add_modifier.contains(Modifier::DIM),
            "el nombre del archivo deberia ir brillante (BOLD, sin DIM)"
        );
    }

    #[test]
    fn la_barra_de_seleccion_padea_al_ancho() {
        // La fila seleccionada se rellena con espacios hasta el ancho dado, asi el
        // fondo de la barra cubre toda la fila y no solo el texto. Con width=20 y
        // "a.md" (marker 2 + 4 chars = 6 usados), el texto plano de la fila debe
        // medir 20 celdas, y el padding final debe llevar el fondo de seleccion.
        let theme = Theme::frappe();
        let s = Switcher::new(paths(&["a.md"]));
        let lines = s.result_lines_padded(&theme, 10, 20);
        let line = &lines[0];
        assert_eq!(
            line_text(line).chars().count(),
            20,
            "la fila no llego al ancho"
        );
        let last = line.spans.last().unwrap();
        assert!(
            last.content.chars().all(|c| c == ' '),
            "el relleno final deberian ser espacios"
        );
        assert_eq!(
            last.style.bg,
            Some(theme.selection_bg),
            "el relleno deberia llevar el fondo de la barra"
        );
    }

    #[test]
    fn la_fila_no_seleccionada_no_padea() {
        // Solo la fila seleccionada lleva barra/padding; las demas quedan al largo
        // de su contenido (marker + path), sin relleno.
        let theme = Theme::frappe();
        let s = Switcher::new(paths(&["a.md", "bb.md"]));
        let lines = s.result_lines_padded(&theme, 10, 30);
        // La fila 0 esta seleccionada (padea a 30); la fila 1 no (mide 2 + 5 = 7).
        assert_eq!(line_text(&lines[0]).chars().count(), 30);
        assert_eq!(
            line_text(&lines[1]).chars().count(),
            2 + "bb.md".chars().count()
        );
    }

    #[test]
    fn scroll_window_lista_mas_corta_que_max_rows() {
        // Con menos resultados que filas disponibles se muestran todos desde 0,
        // sin scroll (end clampeado a `len`).
        assert_eq!(scroll_window(0, 3, 10), 0..3);
        // Aun con el seleccionado en el ultimo, no hay que correr la ventana.
        assert_eq!(scroll_window(2, 3, 10), 0..3);
    }

    #[test]
    fn scroll_window_seleccionado_al_final_corre_la_ventana() {
        // 100 resultados, 5 filas: el seleccionado al final deja la ventana
        // pegada al fondo, con el seleccionado en la ultima fila visible.
        assert_eq!(scroll_window(99, 100, 5), 95..100);
        // Justo en el borde: selected == max_rows ya corre una fila.
        assert_eq!(scroll_window(5, 100, 5), 1..6);
        // Por debajo del borde sigue arrancando en 0.
        assert_eq!(scroll_window(4, 100, 5), 0..5);
    }

    #[test]
    fn scroll_window_casos_vacios() {
        // Sin filas o sin resultados, rango vacio.
        assert_eq!(scroll_window(0, 0, 5), 0..0);
        assert_eq!(scroll_window(3, 10, 0), 0..0);
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
