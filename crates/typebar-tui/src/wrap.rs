//! Motor puro de soft wrap: dado un set de `Line`s ya renderizadas (una por
//! linea del documento, ver `render.rs`) y un ancho de viewport, calcula donde
//! se corta cada linea en filas visuales y parte las `Line`s para dibujarlas.
//!
//! Es deliberadamente "puro": no sabe nada de scroll, viewport rects ni del
//! loop principal. La integracion con `draw`/el cursor vive en otra tarea; acá
//! solo se resuelve el layout y el corte de texto+estilos.
//!
//! Unidad de trabajo: celdas de pantalla ("display cols"), no bytes ni chars,
//! via los helpers de `typebar_core::text` (que replican `cell_width` de
//! ratatui). El corte SIEMPRE cae en un limite de grafema: un CJK/emoji ancho
//! nunca se parte ni se cuenta a medias.
//!
//! Algoritmo de wrap (greedy, por linea, independiente entre lineas):
//! - Se recorren los grafemas de la linea acumulando ancho.
//! - Si el proximo grafema no entra en la fila: cortar despues del ULTIMO
//!   espacio (`' '`) que haya entrado en la fila (el espacio se queda en esa
//!   fila, no se recorta ni se descarta: los rangos deben ser contiguos), asi
//!   la palabra completa baja a la fila siguiente.
//! - Si no hubo ningun espacio en la fila (una palabra sola mas larga que el
//!   ancho), corte duro por grafema en el punto exacto donde dejo de entrar.
//! - Si ni el primer grafema de la fila entra (grafema mas ancho que el
//!   viewport), se fuerza igual (para garantizar progreso): nunca queda una
//!   fila vacia.
//!
//! `no_wrap[i] == true` o `width == 0` desactivan el wrap para esa linea (o
//! para todas): queda en una sola fila, que ratatui clipea al dibujar.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

use typebar_core::text::graphemes_with_width;

/// Layout de wrap resultante de `layout()`: para cada linea del documento,
/// los limites (en display cols) donde arranca cada fila visual.
///
/// Representacion interna: por linea, un vector de limites
/// `[0, b1, b2, ..., ancho_total]` (longitud = cantidad de filas + 1), igual
/// al patron de `boundaries` en `LineGraphemes` del core. La fila `k` ocupa
/// `[boundaries[k], boundaries[k+1])`. Como los limites son contiguos y
/// cubren toda la linea, no hace falta guardar el ancho total aparte: es el
/// ultimo elemento.
pub struct WrapLayout {
    /// `boundaries[i]` = limites de display-col de las filas de la linea `i`.
    boundaries: Vec<Vec<usize>>,
    /// `first_row[i]` = indice de fila visual absoluta donde arranca la linea `i`.
    first_row: Vec<usize>,
    /// Cantidad total de filas visuales (suma de filas de todas las lineas).
    total_rows: usize,
}

impl WrapLayout {
    /// Cantidad total de filas visuales.
    pub fn total_rows(&self) -> usize {
        self.total_rows
    }

    /// (fila visual absoluta, x en celdas dentro de esa fila) para el cursor
    /// parado en `display_col` (columna visual sobre la linea COMPLETA del
    /// documento, no sobre una fila individual).
    ///
    /// Regla del limite exacto: si `display_col` cae justo en el limite entre
    /// la fila `k` y la `k+1`, pertenece a la fila `k+1` (ahi aparece lo
    /// proximo que se tipee) -- salvo que sea el fin de la ULTIMA fila de la
    /// linea (fin de linea), en cuyo caso se queda en esa ultima fila.
    ///
    /// Se resuelve buscando el arranque de fila (`boundaries[k]`, para
    /// `k` en `0..filas`, el sentinela final excluido) mas grande que sea
    /// `<= display_col`: como el sentinela de fin de linea no es un arranque
    /// de fila valido, la busqueda cae naturalmente en la ultima fila cuando
    /// `display_col` es el fin de linea, sin caso especial.
    pub fn row_and_x(&self, line: usize, display_col: usize) -> (usize, usize) {
        let b = &self.boundaries[line];
        let n_rows = b.len() - 1;
        // Cuenta cuantos arranques de fila (b[0..n_rows]) son <= display_col;
        // ese conteo - 1 es la fila. b[0] == 0 siempre es <= display_col, asi
        // que el conteo nunca es 0.
        let row = b[..n_rows].partition_point(|&start| start <= display_col) - 1;
        (self.first_row[line] + row, display_col - b[row])
    }
}

/// Calcula el layout de wrap. `lines` son las `Line`s ya renderizadas (una por
/// linea del documento); `no_wrap[i]` marca que esa linea no debe envolverse
/// (ej. la grilla de una tabla: se deja en 1 fila y ratatui la clipea si no
/// entra); `width` es el ancho disponible en celdas (`0` => todo en 1 fila,
/// sin wrap, para cualquier linea).
pub fn layout(lines: &[Line<'static>], no_wrap: &[bool], width: usize) -> WrapLayout {
    let mut boundaries = Vec::with_capacity(lines.len());
    let mut first_row = Vec::with_capacity(lines.len());
    let mut total_rows = 0;

    for (i, line) in lines.iter().enumerate() {
        let graphemes = line_grapheme_meta(line);
        let total_width: usize = graphemes.iter().map(|&(w, _)| w).sum();
        let skip_wrap = width == 0 || no_wrap.get(i).copied().unwrap_or(false);

        let b = if skip_wrap {
            vec![0, total_width]
        } else {
            let widths: Vec<usize> = graphemes.iter().map(|&(w, _)| w).collect();
            let spaces: Vec<bool> = graphemes.iter().map(|&(_, s)| s).collect();
            wrap_boundaries(&widths, &spaces, width)
        };

        first_row.push(total_rows);
        total_rows += b.len() - 1;
        boundaries.push(b);
    }

    WrapLayout {
        boundaries,
        first_row,
        total_rows,
    }
}

/// Parte las `Line`s en filas visuales segun `layout`, preservando spans y
/// estilos: un span puede partirse justo en el limite de una fila, y ambas
/// mitades conservan el estilo original.
pub fn visual_lines(lines: Vec<Line<'static>>, layout: &WrapLayout) -> Vec<Line<'static>> {
    let mut out = Vec::with_capacity(layout.total_rows);
    for (i, line) in lines.into_iter().enumerate() {
        out.extend(split_line(line, &layout.boundaries[i]));
    }
    out
}

/// Ancho (celdas) y si-es-espacio de cada grafema de una `Line`, concatenando
/// el contenido de todos sus spans en orden.
fn line_grapheme_meta(line: &Line<'static>) -> Vec<(usize, bool)> {
    let mut out = Vec::new();
    for span in &line.spans {
        for (g, w) in graphemes_with_width(span.content.as_ref()) {
            out.push((w, g == " "));
        }
    }
    out
}

/// Greedy word-wrap sobre una secuencia de grafemas (ancho + flag de
/// espacio), ya separada de la linea. Devuelve los limites en display cols
/// (`[0, .., ancho_total]`, longitud = filas + 1). Precondicion: se llama
/// solo cuando el wrap esta activo (`width > 0` y no `no_wrap`); el ancho
/// total mayor al `width` es justamente el caso que produce mas de una fila.
fn wrap_boundaries(widths: &[usize], is_space: &[bool], width: usize) -> Vec<usize> {
    let total: usize = widths.iter().sum();
    if total <= width {
        // Entra entera (incluida la linea vacia): una sola fila.
        return vec![0, total];
    }

    let n = widths.len();
    let mut boundaries = vec![0];
    let mut idx = 0; // indice de grafema donde arranca la fila actual
    let mut start_col = 0; // display col donde arranca la fila actual

    while idx < n {
        let mut col = start_col;
        // Ultimo punto (indice de grafema siguiente, col) justo despues de un
        // espacio que entro en esta fila: preferimos cortar ahi.
        let mut last_space: Option<(usize, usize)> = None;
        let mut j = idx;

        loop {
            if j >= n {
                // El resto de la linea entero cabe en esta fila: ultima fila.
                boundaries.push(total);
                idx = n;
                break;
            }
            let w = widths[j];
            if col - start_col + w <= width {
                col += w;
                if is_space[j] {
                    last_space = Some((j + 1, col));
                }
                j += 1;
                continue;
            }
            // El grafema `j` no entra en la fila actual.
            if col == start_col {
                // Fila vacia: forzar este grafema igual, para garantizar
                // progreso (nunca partirlo, pero tampoco dejar la fila vacia).
                col += w;
                j += 1;
            } else if let Some((next_idx, break_col)) = last_space {
                // Cortar despues del ultimo espacio: la palabra entera que no
                // entraba baja completa a la fila siguiente.
                col = break_col;
                j = next_idx;
            }
            // (si no hay espacio y la fila no esta vacia: corte duro,
            // `col`/`j` ya son el punto de corte tal cual quedaron)
            boundaries.push(col);
            idx = j;
            start_col = col;
            break;
        }
    }

    boundaries
}

/// Parte una `Line` en filas visuales segun `boundaries` (limites en display
/// cols, formato `WrapLayout`). Como el estilo es constante dentro de un
/// mismo `Span` de origen, partir un span en un limite de fila solo requiere
/// cortar su texto en el grafema correspondiente: ambos pedazos heredan el
/// estilo original.
fn split_line(line: Line<'static>, boundaries: &[usize]) -> Vec<Line<'static>> {
    let n_rows = boundaries.len() - 1;
    let mut rows: Vec<Vec<Span<'static>>> = (0..n_rows).map(|_| Vec::new()).collect();
    let mut row_idx = 0;
    let mut col = 0usize;

    for span in line.spans {
        let style: Style = span.style;
        let mut piece = String::new();
        for (g, w) in graphemes_with_width(span.content.as_ref()) {
            while row_idx + 1 < boundaries.len() && col >= boundaries[row_idx + 1] {
                if !piece.is_empty() {
                    rows[row_idx].push(Span::styled(std::mem::take(&mut piece), style));
                }
                row_idx += 1;
            }
            piece.push_str(g);
            col += w;
        }
        if !piece.is_empty() {
            rows[row_idx].push(Span::styled(piece, style));
        }
    }

    rows.into_iter().map(Line::from).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::{Color, Modifier};

    /// Construye una `Line` de un unico span sin estilo particular, atajo
    /// para los tests que no necesitan mezclar estilos.
    fn plain(s: &str) -> Line<'static> {
        Line::from(s.to_string())
    }

    /// Texto visible completo de una fila (concatena sus spans).
    fn row_text(line: &Line<'static>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn corte_exacto_ascii_en_el_ancho() {
        // "1234567890" con ancho 5: entra exacto en dos filas de 5.
        let lines = vec![plain("1234567890")];
        let layout = layout(&lines, &[false], 5);
        assert_eq!(layout.total_rows(), 2);
        let rows = visual_lines(lines, &layout);
        assert_eq!(row_text(&rows[0]), "12345");
        assert_eq!(row_text(&rows[1]), "67890");
    }

    #[test]
    fn prefiere_cortar_en_el_espacio() {
        // "hola mundo bien" con ancho 8: "mundo" (5) no entra despues de
        // "hola " (5) porque 5+5=10>8, asi que corta tras el espacio y
        // "mundo" baja entera a la fila siguiente (no "hola mun"/"do...").
        let lines = vec![plain("hola mundo bien")];
        let layout = layout(&lines, &[false], 8);
        let rows = visual_lines(lines, &layout);
        let texts: Vec<String> = rows.iter().map(row_text).collect();
        // El espacio de corte queda en la fila anterior (no se recorta).
        assert_eq!(texts[0], "hola ");
        assert_eq!(texts[1], "mundo ");
        assert_eq!(texts[2], "bien");
    }

    #[test]
    fn palabra_mas_larga_que_el_ancho_corta_duro() {
        // "abcdefghij" (10, sin espacios) con ancho 4: no hay donde cortar en
        // espacio, asi que corta duro cada 4 grafemas.
        let lines = vec![plain("abcdefghij")];
        let layout = layout(&lines, &[false], 4);
        let rows = visual_lines(lines, &layout);
        let texts: Vec<String> = rows.iter().map(row_text).collect();
        assert_eq!(texts, vec!["abcd", "efgh", "ij"]);
    }

    #[test]
    fn cjk_ancho_no_se_parte_fila_mas_corta() {
        // "a中b" con ancho 2: 'a'(1) entra, '中'(2) no entra (1+2=3>2) ->
        // fila 1 = "a" (mas corta que el ancho, no se parte el CJK). Fila 2
        // arranca en '中'(2), 'b'(1) no entra (2+1=3>2) -> fila 2 = "中".
        // Fila 3 = "b".
        let lines = vec![plain("a中b")];
        let layout = layout(&lines, &[false], 2);
        let rows = visual_lines(lines, &layout);
        let texts: Vec<String> = rows.iter().map(row_text).collect();
        assert_eq!(texts, vec!["a", "中", "b"]);
    }

    #[test]
    fn emoji_ancho_no_se_parte() {
        // "x😀y" con ancho 2: igual que el CJK, el emoji (ancho 2) fuerza su
        // propia fila corta.
        let lines = vec![plain("x😀y")];
        let layout = layout(&lines, &[false], 2);
        let rows = visual_lines(lines, &layout);
        let texts: Vec<String> = rows.iter().map(row_text).collect();
        assert_eq!(texts, vec!["x", "😀", "y"]);
    }

    #[test]
    fn rangos_contiguos_cubren_toda_la_linea() {
        // La suma de anchos de las filas visuales = ancho total de la linea,
        // sin huecos ni solapamientos (el espacio de corte cuenta para la
        // fila anterior).
        let text = "una linea con varias palabras para envolver de verdad";
        let lines = vec![plain(text)];
        let total_width = typebar_core::text::display_width(text);
        let layout = layout(&lines, &[false], 10);
        let rows = visual_lines(lines, &layout);
        let sum: usize = rows
            .iter()
            .map(|l| {
                l.spans
                    .iter()
                    .map(|s| typebar_core::text::display_width(s.content.as_ref()))
                    .sum::<usize>()
            })
            .sum();
        assert_eq!(sum, total_width);
        // Reconstruyendo el texto de todas las filas (sin separador) se
        // recupera exactamente el original.
        let joined: String = rows.iter().map(row_text).collect();
        assert_eq!(joined, text);
    }

    #[test]
    fn row_and_x_cursor_en_cero() {
        let lines = vec![plain("hola mundo bien")];
        let layout = layout(&lines, &[false], 8);
        assert_eq!(layout.row_and_x(0, 0), (0, 0));
    }

    #[test]
    fn row_and_x_cursor_en_medio_de_una_fila() {
        let lines = vec![plain("hola mundo bien")];
        let layout = layout(&lines, &[false], 8);
        // "hola " ocupa cols [0,5). Col 2 (la 'l') sigue en la fila 0.
        assert_eq!(layout.row_and_x(0, 2), (0, 2));
    }

    #[test]
    fn row_and_x_en_el_limite_exacto_entre_filas_va_a_la_siguiente() {
        let lines = vec![plain("hola mundo bien")];
        let layout = layout(&lines, &[false], 8);
        // Fila 0 = "hola " (cols [0,5)); fila 1 = "mundo " (cols [5,11)).
        // display_col == 5 es el limite exacto -> pertenece a la fila 1, x=0.
        assert_eq!(layout.row_and_x(0, 5), (1, 0));
    }

    #[test]
    fn row_and_x_en_fin_de_linea_se_queda_en_la_ultima_fila() {
        let lines = vec![plain("hola mundo bien")];
        let layout = layout(&lines, &[false], 8);
        let total = typebar_core::text::display_width("hola mundo bien");
        let (row, x) = layout.row_and_x(0, total);
        // Ultima fila es "bien" (cols [11,15)); fin de linea (15) se queda
        // ahi, no "crea" una fila fantasma despues.
        assert_eq!(row, layout.total_rows() - 1);
        assert_eq!(x, total - 11);
    }

    #[test]
    fn linea_vacia_es_una_fila() {
        let lines = vec![plain("")];
        let layout = layout(&lines, &[false], 8);
        assert_eq!(layout.total_rows(), 1);
        let rows = visual_lines(lines, &layout);
        assert_eq!(rows.len(), 1);
        assert_eq!(row_text(&rows[0]), "");
        // El cursor en la unica posicion valida (col 0) mapea a la fila 0.
        assert_eq!(layout.row_and_x(0, 0), (0, 0));
    }

    #[test]
    fn linea_que_entra_justo_es_una_fila() {
        let lines = vec![plain("12345")];
        let layout = layout(&lines, &[false], 5);
        assert_eq!(layout.total_rows(), 1);
    }

    #[test]
    fn no_wrap_deja_una_fila_aunque_exceda() {
        let lines = vec![plain("esta linea es mucho mas larga que el ancho")];
        let layout = layout(&lines, &[true], 10);
        assert_eq!(layout.total_rows(), 1);
        let rows = visual_lines(lines, &layout);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            row_text(&rows[0]),
            "esta linea es mucho mas larga que el ancho"
        );
    }

    #[test]
    fn ancho_cero_deja_todo_en_una_fila() {
        let lines = vec![plain("cualquier cosa, sin wrap")];
        let layout = layout(&lines, &[false], 0);
        assert_eq!(layout.total_rows(), 1);
    }

    #[test]
    fn visual_lines_preserva_estilos_al_partir_un_span_en_el_medio() {
        // Un solo span en negrita de 10 chars, ancho 6: se parte en dos
        // filas, y AMBAS mitades conservan el estilo bold.
        let style = Style::default().add_modifier(Modifier::BOLD).fg(Color::Red);
        let line = Line::from(Span::styled("abcdefghij", style));
        let lines = vec![line];
        let layout = layout(&lines, &[false], 6);
        let rows = visual_lines(lines, &layout);
        assert_eq!(rows.len(), 2);
        assert_eq!(row_text(&rows[0]), "abcdef");
        assert_eq!(row_text(&rows[1]), "ghij");
        for row in &rows {
            for span in &row.spans {
                assert_eq!(
                    span.style, style,
                    "el estilo debe preservarse en ambas mitades"
                );
            }
        }
    }

    #[test]
    fn visual_lines_preserva_multiples_spans_con_estilos_distintos() {
        // "AA" bold + "BBBB" plano, ancho 3: el corte cae dentro del segundo
        // span ("BBBB"), partiendolo; el primero queda intacto.
        let bold = Style::default().add_modifier(Modifier::BOLD);
        let plain_style = Style::default();
        let line = Line::from(vec![
            Span::styled("AA", bold),
            Span::styled("BBBB", plain_style),
        ]);
        let lines = vec![line];
        let layout = layout(&lines, &[false], 3);
        let rows = visual_lines(lines, &layout);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].spans.len(), 2);
        assert_eq!(rows[0].spans[0].content.as_ref(), "AA");
        assert_eq!(rows[0].spans[0].style, bold);
        assert_eq!(rows[0].spans[1].content.as_ref(), "B");
        assert_eq!(rows[0].spans[1].style, plain_style);
        assert_eq!(rows[1].spans.len(), 1);
        assert_eq!(rows[1].spans[0].content.as_ref(), "BBB");
        assert_eq!(rows[1].spans[0].style, plain_style);
    }

    #[test]
    fn total_rows_consistente_con_filas_por_linea() {
        // Linea 0: corta (1 fila). Linea 1: larga, envuelve en 3 filas.
        // Linea 2: no_wrap (1 fila aunque exceda el ancho).
        // No hay getter publico del arranque de fila por linea (ver
        // `row_and_x`): se infiere pidiendo `row_and_x(linea, 0)`, que por
        // construccion cae siempre en la primera fila de esa linea.
        let lines = vec![
            plain("corta"),
            plain("una linea bastante mas larga que el ancho dado"),
            plain("linea de tabla que no se envuelve nunca jamas"),
        ];
        let layout = layout(&lines, &[false, false, true], 10);

        let first_row_0 = layout.row_and_x(0, 0).0;
        let first_row_1 = layout.row_and_x(1, 0).0;
        let first_row_2 = layout.row_and_x(2, 0).0;
        assert_eq!(first_row_0, 0);

        let rows_line0 = first_row_1 - first_row_0;
        assert_eq!(rows_line0, 1);

        let rows_line1 = first_row_2 - first_row_1;
        assert!(
            rows_line1 > 1,
            "la linea larga debe envolver en varias filas"
        );

        let rows_line2 = layout.total_rows() - first_row_2;
        assert_eq!(rows_line2, 1, "no_wrap deja 1 sola fila");

        // La suma de filas por linea = total_rows.
        assert_eq!(rows_line0 + rows_line1 + rows_line2, layout.total_rows());
    }
}
