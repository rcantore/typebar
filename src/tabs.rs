//! Barra de tabs de los buffers abiertos: una linea horizontal con un "tab" por
//! buffer (su nombre de archivo), el activo resaltado. `build` devuelve tambien
//! el rango de columnas de cada tab para el hit-testing del click del mouse, asi
//! el render (en `draw`) y el manejo del click (en `run`) coinciden.

use std::ops::Range;

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Un tab dibujado: a que buffer corresponde (`index`) y que columnas ocupa en la
/// fila (`cols`), para mapear un click a un buffer.
pub struct TabHit {
    pub index: usize,
    pub cols: Range<u16>,
}

/// Construye la linea de tabs y los rangos de columnas de cada uno. `titles` son
/// los nombres a mostrar (uno por buffer, en orden) y `active` el indice del
/// enfocado. Cada tab se dibuja como ` titulo ` (un espacio a cada lado); el
/// activo va en negrita sobre el fondo de boton del theme, los demas atenuados.
/// Entre tabs va un separador de 1 columna (sin tab asociado).
pub fn build(titles: &[String], active: usize, theme: &Theme) -> (Line<'static>, Vec<TabHit>) {
    let active_style = Style::default()
        .bg(theme.toolbar_button_bg)
        .add_modifier(Modifier::BOLD);
    let inactive_style = Style::default().add_modifier(Modifier::DIM);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut hits: Vec<TabHit> = Vec::new();
    let mut col: u16 = 0;
    for (i, title) in titles.iter().enumerate() {
        let label = format!(" {title} ");
        let width = label.chars().count() as u16;
        let style = if i == active {
            active_style
        } else {
            inactive_style
        };
        spans.push(Span::styled(label, style));
        hits.push(TabHit {
            index: i,
            cols: col..col + width,
        });
        col += width;
        // Separador entre tabs (no mapea a ningun tab: un click ahi no hace nada).
        if i + 1 < titles.len() {
            spans.push(Span::raw(" "));
            col += 1;
        }
    }
    (Line::from(spans), hits)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn titles(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn los_rangos_cubren_cada_tab_sin_pisarse() {
        let theme = Theme::frappe();
        let (_line, hits) = build(&titles(&["a.md", "bb.md"]), 0, &theme);
        assert_eq!(hits.len(), 2);
        // " a.md " son 6 cols (0..6); luego 1 de separador; " bb.md " son 7 (7..14).
        assert_eq!(hits[0].index, 0);
        assert_eq!(hits[0].cols, 0..6);
        assert_eq!(hits[1].index, 1);
        assert_eq!(hits[1].cols, 7..14);
    }

    #[test]
    fn el_separador_no_mapea_a_ningun_tab() {
        let theme = Theme::frappe();
        let (_line, hits) = build(&titles(&["a.md", "bb.md"]), 0, &theme);
        // La columna 6 (el separador entre los dos tabs) no cae en ningun rango.
        assert!(!hits.iter().any(|t| t.cols.contains(&6)));
    }
}
