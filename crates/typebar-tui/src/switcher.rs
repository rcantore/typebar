//! Switcher de archivos: un fuzzy finder sobre los archivos del proyecto y los
//! buffers abiertos. Vive aparte de los Overlay de busqueda/reemplazo porque
//! opera a nivel workspace (abre o cambia de buffer), no sobre el documento:
//! `run` lo maneja y, al aceptar, abre el path elegido en el `Workspace`.

use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Padding, Paragraph};

use std::ops::Range;
use std::path::PathBuf;

use crate::theme::Theme;
use typebar_core::fuzzy;
use typebar_core::i18n;

/// Padding interno del box: aire entre el borde y el contenido. Es el detalle que
/// separa un popup "apretado" (texto pegado al borde) de uno que respira, como en
/// editxr. `X` a los lados, `Y` arriba/abajo.
pub(crate) const PICKER_PAD_X: u16 = 2;
pub(crate) const PICKER_PAD_Y: u16 = 1;
/// Filas de "chrome" del contenido (fuera de los resultados): prompt + linea en
/// blanco + linea en blanco + footer de atajos. Se usa para calcular el alto del
/// box y para saber cuantas filas quedan para resultados.
pub(crate) const PICKER_CHROME_ROWS: u16 = 4;

/// Geometria del popup del picker. Dado el `area` total y cuantos resultados hay,
/// devuelve `(rect centrado, ancho util del contenido, filas de resultados a
/// mostrar)`. El box se AJUSTA al contenido (como editxr): su alto es el de las
/// filas visibles + chrome + padding + borde, con tope ~80% de la pantalla. Con
/// pocos resultados el box queda chico y elegante; con muchos, scrollea dentro del
/// tope. El ancho es ~70%, acotado a un minimo usable y al ancho disponible.
pub(crate) fn picker_popup(area: Rect, result_count: usize) -> (Rect, usize, usize) {
    let w = (area.width * 7 / 10).clamp(40.min(area.width), area.width);
    // Overhead vertical fijo: borde (2) + padding (2*PAD_Y) + chrome (prompt/
    // blanks/footer). Lo que sobre del box es para resultados.
    let overhead = 2 + 2 * PICKER_PAD_Y + PICKER_CHROME_ROWS;
    // El box no pasa del ~80% del alto; dentro de eso caben tantos resultados como
    // permita el overhead (al menos 1, para el "(sin resultados)").
    let max_h = (area.height * 8 / 10).max(overhead + 1);
    let cap = max_h.saturating_sub(overhead).max(1) as usize;
    let shown = result_count.clamp(1, cap);
    let h = (shown as u16 + overhead).min(area.height);
    let popup = Rect {
        x: area.x + (area.width - w) / 2,
        y: area.y + (area.height - h) / 2,
        width: w,
        height: h,
    };
    let inner_width = w.saturating_sub(2 + 2 * PICKER_PAD_X) as usize;
    (popup, inner_width, shown)
}

/// Box comun de los pickers flotantes (paleta y switcher): borde redondeado con el
/// color de acento del theme y padding interno. Sin titulo ni footer en el borde:
/// el prompt y los atajos van como filas de contenido (ver `picker_content`), asi
/// el borde queda limpio. El acento reusa `heading_2` (el mismo azul/cian que ya
/// resalta el match del fuzzy), asi borde y resaltado leen como una sola
/// identidad; en los themes monocromos (papel) cae al color de tinta y el borde
/// queda sobrio, sin acento fuera de lugar.
pub(crate) fn picker_block(theme: &Theme) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(theme.heading_2))
        .padding(Padding::new(
            PICKER_PAD_X,
            PICKER_PAD_X,
            PICKER_PAD_Y,
            PICKER_PAD_Y,
        ))
}

/// Fila de prompt del picker: la etiqueta en acento, lo tipeado en normal con un
/// cursor `_`, y el conteo de resultados atenuado a la derecha. La comparten los
/// dos pickers para que el encabezado se vea igual (solo cambia el `label`).
pub(crate) fn picker_prompt(
    theme: &Theme,
    label: &str,
    query: &str,
    count: usize,
) -> Line<'static> {
    let accent = Style::default()
        .fg(theme.heading_2)
        .add_modifier(Modifier::BOLD);
    let dim = Style::default().add_modifier(Modifier::DIM);
    Line::from(vec![
        Span::styled(format!("{label} "), accent),
        Span::raw(query.to_string()),
        Span::styled("_", dim),
        Span::styled(format!("   ({count})"), dim),
    ])
}

/// Ensambla el contenido del box a partir del `prompt` (ya estilado) y las `rows`
/// de resultados: prompt · blank · resultados · blank · footer de atajos (tenue).
/// Lo comparten paleta y switcher para tener el mismo ritmo vertical. El caller
/// dimensiona `rows` para que el total llene el box (footer al pie).
pub(crate) fn picker_content(
    prompt: Line<'static>,
    mut rows: Vec<Line<'static>>,
) -> Vec<Line<'static>> {
    let hint = Line::from(Span::styled(
        i18n::t(i18n::Key::PickerHints).to_string(),
        Style::default().add_modifier(Modifier::DIM),
    ));
    let mut out = Vec::with_capacity(rows.len() + 3);
    out.push(prompt);
    out.push(Line::from(""));
    out.append(&mut rows);
    out.push(Line::from(""));
    out.push(hint);
    out
}

/// Atenua (DIM) todas las celdas de `area` YA dibujadas en `buf`. Lo usan los
/// pickers para dejar el documento visible pero apagado detras de su popup, en
/// vez de borrarlo a negro con `Clear`. `set_style` con solo `add_modifier`
/// mergea: agrega el DIM sin tocar el fg/bg ni el simbolo de cada celda.
pub(crate) fn dim_area(buf: &mut Buffer, area: Rect) {
    let dim = Style::default().add_modifier(Modifier::DIM);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_style(dim);
        }
    }
}

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
    /// Candidatos a filtrar (buffers abiertos primero + archivos del proyecto, dedup).
    candidates: Vec<PathBuf>,
    /// Representacion string de cada candidato (cacheada para el fuzzy y el render).
    labels: Vec<String>,
    /// Marca "no a salvo en disco" por candidato, paralela a `candidates`: `true`
    /// si el candidato es un buffer abierto con cambios sin guardar o todavia no
    /// guardado en disco (untitled). Los archivos del disco que no esten abiertos
    /// van en `false`.
    unsaved: Vec<bool>,
    /// Texto tipeado.
    query: String,
    /// Resultados rankeados: indices en `candidates` con su match (para resaltar).
    results: Vec<(usize, fuzzy::FuzzyMatch)>,
    /// Item seleccionado dentro de `results`.
    selected: usize,
}

impl Switcher {
    /// Crea el switcher con sus candidatos (y su marca de sin-guardar paralela) y
    /// la query vacia (todos matchean). `unsaved[i]` corresponde a `candidates[i]`.
    pub fn new(candidates: Vec<PathBuf>, unsaved: Vec<bool>) -> Self {
        debug_assert_eq!(
            candidates.len(),
            unsaved.len(),
            "unsaved debe ser paralelo a candidates"
        );
        let labels = candidates
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let mut s = Switcher {
            candidates,
            labels,
            unsaved,
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

            // Marca "sin guardar": el mismo `[+]` que usa la status bar, en columna
            // fija de 4 celdas para que los nombres queden alineados (dirty o no).
            // Los candidatos limpios reservan el mismo ancho con espacios.
            let dirty_mark = if self.unsaved[*ci] { "[+] " } else { "    " };
            let mut dirty_style = Style::default()
                .fg(theme.heading_1)
                .add_modifier(Modifier::BOLD);
            if selected {
                dirty_style = dirty_style.bg(theme.selection_bg);
            }
            spans.push(Span::styled(dirty_mark.to_string(), dirty_style));

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
                let used = marker.chars().count() + dirty_mark.chars().count() + total_chars;
                if width > used {
                    let pad = " ".repeat(width - used);
                    spans.push(Span::styled(pad, sel_bg));
                }
            }

            lines.push(Line::from(spans));
        }
        lines
    }

    /// Dibuja el switcher como un popup CENTRADO flotante montado SOBRE el editor
    /// ya dibujado: atenua (DIM) todo el fondo para que el documento se vea apagado
    /// detras, y pinta en el centro un box con borde redondeado (acento del theme)
    /// con el prompt + lo tipeado (con `_` de cursor) + el conteo en el titulo, la
    /// lista rankeada adentro y un footer de atajos al pie. Al no setear cursor,
    /// ratatui lo oculta; el `_` del prompt marca donde se tipea.
    pub fn render(&self, frame: &mut Frame, theme: &Theme) {
        let area = frame.area();
        // Geometria compartida: el box se ajusta al contenido (ancho ~70%, alto
        // segun cuantos resultados entren), centrado.
        let (popup, inner_width, shown) = picker_popup(area, self.result_count());
        // Filas de resultados (exactamente `shown`): sin matches, una linea tenue.
        let rows = if self.result_count() == 0 {
            vec![Line::from(Span::styled(
                i18n::t(i18n::Key::SwitcherEmpty).to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ))]
        } else {
            // `inner_width` es el ancho util (descontando borde y padding), para
            // que la barra de seleccion llegue justo al borde interno.
            self.result_lines_padded(theme, shown, inner_width)
        };
        let prompt = picker_prompt(
            theme,
            i18n::t(i18n::Key::SwitcherPrompt),
            self.query(),
            self.result_count(),
        );
        let content = picker_content(prompt, rows);
        // Atenuar el editor de fondo (ya dibujado) en vez de borrarlo; despues
        // limpiar SOLO el rect del popup para que el box quede nitido encima.
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

    fn paths(items: &[&str]) -> Vec<PathBuf> {
        items.iter().map(PathBuf::from).collect()
    }

    /// Switcher con todos los candidatos "limpios" (sin marca): el caso de los
    /// tests que no ejercitan la marca de sin-guardar.
    fn sw_clean(candidates: Vec<PathBuf>) -> Switcher {
        let unsaved = vec![false; candidates.len()];
        Switcher::new(candidates, unsaved)
    }

    #[test]
    fn arranca_con_todos_los_candidatos() {
        let s = sw_clean(paths(&["src/main.rs", "README.md", "src/fuzzy.rs"]));
        assert_eq!(s.result_count(), 3);
    }

    #[test]
    fn tipear_filtra() {
        let mut s = sw_clean(paths(&["src/main.rs", "README.md", "src/fuzzy.rs"]));
        s.handle_key(key(KeyCode::Char('f')));
        s.handle_key(key(KeyCode::Char('z')));
        // "fz" solo matchea fuzzy.rs.
        assert_eq!(s.result_count(), 1);
    }

    #[test]
    fn enter_acepta_el_seleccionado() {
        let mut s = sw_clean(paths(&["a.md", "b.md"]));
        // Bajar a "b.md" y aceptar.
        s.handle_key(key(KeyCode::Down));
        match s.handle_key(key(KeyCode::Enter)) {
            SwitcherOutcome::Accept(p) => assert_eq!(p, PathBuf::from("b.md")),
            _ => panic!("esperaba Accept"),
        }
    }

    #[test]
    fn esc_cancela() {
        let mut s = sw_clean(paths(&["a.md"]));
        assert!(matches!(
            s.handle_key(key(KeyCode::Esc)),
            SwitcherOutcome::Cancel
        ));
    }

    #[test]
    fn enter_sin_resultados_cancela() {
        let mut s = sw_clean(paths(&["a.md"]));
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
        // (y sin DIM). spans[0] es el marker de seleccion y spans[1] la columna
        // dirty; el path arranca en spans[2].
        let theme = Theme::frappe();
        let s = sw_clean(paths(&["src/main.rs"]));
        let lines = s.result_lines_padded(&theme, 10, 20);
        let line = &lines[0];
        // "src/" son 4 chars -> spans[2..=5] son el directorio.
        for span in &line.spans[2..=5] {
            assert!(
                span.style.add_modifier.contains(Modifier::DIM),
                "el directorio deberia ir atenuado (DIM)"
            );
        }
        // "main.rs" empieza en spans[6]; deberia ir en BOLD y sin DIM.
        assert!(
            line.spans[6].style.add_modifier.contains(Modifier::BOLD)
                && !line.spans[6].style.add_modifier.contains(Modifier::DIM),
            "el nombre del archivo deberia ir brillante (BOLD, sin DIM)"
        );
    }

    #[test]
    fn la_barra_de_seleccion_padea_al_ancho() {
        // La fila seleccionada se rellena con espacios hasta el ancho dado, asi el
        // fondo de la barra cubre toda la fila y no solo el texto. Con width=20 y
        // "a.md" (marker 2 + columna dirty 4 + 4 chars = 10 usados), el texto plano
        // de la fila debe medir 20 celdas, y el padding final lleva el fondo de seleccion.
        let theme = Theme::frappe();
        let s = sw_clean(paths(&["a.md"]));
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
        // de su contenido (marker de seleccion + columna dirty + path), sin relleno.
        let theme = Theme::frappe();
        let s = sw_clean(paths(&["a.md", "bb.md"]));
        let lines = s.result_lines_padded(&theme, 10, 30);
        // La fila 0 esta seleccionada (padea a 30); la fila 1 no (mide 2 + 4 + 5).
        assert_eq!(line_text(&lines[0]).chars().count(), 30);
        assert_eq!(
            line_text(&lines[1]).chars().count(),
            2 + 4 + "bb.md".chars().count()
        );
    }

    #[test]
    fn marca_dirty_solo_en_los_candidatos_sin_guardar() {
        // El candidato dirty muestra `[+]` en su columna; el limpio, espacios. La
        // columna es de ancho fijo, asi los nombres quedan alineados en ambos.
        let theme = Theme::frappe();
        // dos buffers: el primero sin guardar, el segundo limpio.
        let s = Switcher::new(paths(&["untitled.md", "notes.md"]), vec![true, false]);
        let lines = s.result_lines_padded(&theme, 10, 40);
        // spans[1] es la columna dirty en cada fila.
        assert_eq!(lines[0].spans[1].content.as_ref(), "[+] ");
        assert_eq!(lines[1].spans[1].content.as_ref(), "    ");
        // El `[+]` lleva color de acento (heading_1), no es texto plano.
        assert_eq!(lines[0].spans[1].style.fg, Some(theme.heading_1));
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
    fn dim_area_atenua_sin_borrar_el_contenido() {
        // `dim_area` agrega DIM a cada celda del area PERO no borra: el simbolo
        // (el editor de fondo ya dibujado) sigue ahi, solo apagado. Asi el popup
        // deja ver el documento atenuado en vez de un fondo negro.
        let area = Rect::new(0, 0, 3, 2);
        let mut buf = Buffer::empty(area);
        buf[(1, 1)].set_symbol("x");
        dim_area(&mut buf, area);
        for y in 0..2 {
            for x in 0..3 {
                assert!(
                    buf[(x, y)].modifier.contains(Modifier::DIM),
                    "toda celda del area deberia quedar atenuada"
                );
            }
        }
        assert_eq!(buf[(1, 1)].symbol(), "x", "no deberia borrar el contenido");
    }

    #[test]
    fn picker_block_borde_redondeado_con_acento() {
        // El box de los pickers usa borde REDONDEADO (esquina `╭`) y lo pinta con
        // el color de acento del theme (heading_2), no el fg plano.
        use ratatui::widgets::Widget;
        let theme = Theme::frappe();
        let area = Rect::new(0, 0, 12, 3);
        let mut buf = Buffer::empty(area);
        picker_block(&theme).render(area, &mut buf);
        assert_eq!(
            buf[(0, 0)].symbol(),
            "╭",
            "la esquina deberia ser redondeada"
        );
        assert_eq!(
            buf[(0, 0)].style().fg,
            Some(theme.heading_2),
            "el borde deberia llevar el color de acento"
        );
    }

    #[test]
    fn la_seleccion_clampea() {
        let mut s = sw_clean(paths(&["a.md", "b.md"]));
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
