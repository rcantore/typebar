//! Pipeline de rendering WYSIWYG. Soporta dos niveles:
//!
//! - **Nivel 1 (soft)**: los marcadores nunca se ocultan, solo se dimmean. El
//!   mapeo cursor->columna queda 1:1 en todas las lineas.
//! - **Nivel 2**: los delimiters inline (`**`, `*`, `` ` ``) se ocultan en
//!   las lineas *inactivas*. La linea con el cursor sigue renderizandose en
//!   Nivel 1, asi que el cursor mapping NO cambia (la columna visual sobre la
//!   linea activa es la misma que en Nivel 1).
//!
//! Heuristica de "linea exenta" (se renderiza como Nivel 1 aunque el nivel sea
//! 2): la linea activa, y todas las lineas si hay seleccion no vacia o
//! coincidencias de busqueda activas, para que el highlight siempre quede
//! sobre celdas reales y no sobre bytes ocultos.
//!
//! Pipeline: texto crudo -> tree-sitter-md (block + inline) -> mapa de estilos
//! por byte + mapa de bytes ocultables -> `ratatui::Line`s.

use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use tree_sitter_md::MarkdownParser;

use crate::theme::Theme;

// La paleta ya no vive aca: la provee el Theme Engine (src/theme/). El renderer
// recibe un `&Theme` y lee sus campos, sin conocer colores concretos.

fn heading_style(theme: &Theme, level: usize) -> Style {
    let fg = match level {
        1 => theme.heading_1,
        2 => theme.heading_2,
        _ => theme.heading_n,
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}

fn marker_style(theme: &Theme) -> Style {
    Style::default().fg(theme.marker)
}

// --- Mapa de estilos por byte ----------------------------------------------

/// Un tramo de bytes del documento que recibe un estilo. `depth` es la
/// profundidad del nodo en el arbol: a mayor profundidad, mas especifico, y
/// por eso gana al pintar (un delimitador `**` esta mas adentro que el
/// `strong_emphasis` que lo contiene, asi que sobrescribe a dimmeado).
struct StyleSpan {
    start: usize,
    end: usize,
    style: Style,
    depth: usize,
}

/// Recorre el arbol Markdown (block + inline) y junta:
/// - los tramos estilizados (`StyleSpan`s),
/// - `hide_byte`: bytes que se omiten en Nivel 2 en lineas inactivas.
///   Cubre delimitadores inline (`**`/`*`/`` ` ``), markers de heading con su
///   whitespace siguiente, y todo lo que rodea al texto de un link o imagen
///   (corchetes, parentesis, URL).
/// - `replace_byte`: bytes que se sustituyen por un char distinto en Nivel 2.
///   Cubre el primer byte de los list markers bullet (`-`/`*`/`+` -> `•`) y
///   el `>` de blockquotes (-> `│`).
///
/// El render decide por linea si aplicar `hide_byte`/`replace_byte` (ver
/// `render`). Los colores salen del `theme`.
fn collect_styles(source: &str, theme: &Theme) -> (Vec<StyleSpan>, Vec<bool>, Vec<Option<char>>) {
    let mut parser = MarkdownParser::default();
    let tree = parser
        .parse(source.as_bytes(), None)
        .expect("tree-sitter-md no pudo parsear el documento");

    let mut spans: Vec<StyleSpan> = Vec::new();
    let mut hide_byte: Vec<bool> = vec![false; source.len()];
    let mut replace_byte: Vec<Option<char>> = vec![None; source.len()];
    let mut cursor = tree.walk();

    // DFS iterativo. `stack` guarda los kinds de los ancestros para saber, por
    // ejemplo, que un nodo `inline` cuelga de un `atx_heading`. La profundidad
    // es `stack.len()`.
    let mut stack: Vec<&str> = Vec::new();
    let mut heading_level: Option<usize> = None;

    'dfs: loop {
        let node = cursor.node();
        let kind = node.kind();
        let range = node.byte_range();
        let depth = stack.len();
        let parent = stack.last().copied();

        let style = match kind {
            // Headings: el texto (nodo `inline` hijo del heading) va bold+color.
            // El marcador `#`/`##` se dimmea aparte (caso *_marker de abajo).
            "atx_heading" => {
                // Nivel = cantidad de '#' al inicio del nodo.
                let level = source.as_bytes()[range.start..]
                    .iter()
                    .take_while(|&&b| b == b'#')
                    .count();
                heading_level = Some(level);
                None
            }
            "inline" if parent == Some("atx_heading") => {
                heading_level.map(|level| heading_style(theme, level))
            }
            "strong_emphasis" => Some(Style::default().add_modifier(Modifier::BOLD)),
            "emphasis" => Some(Style::default().add_modifier(Modifier::ITALIC)),
            "code_span" => Some(Style::default().fg(theme.code_fg).bg(theme.code_bg)),
            // Todos los marcadores y delimitadores: dimmeados.
            k if k.ends_with("_marker") || k.ends_with("_delimiter") => Some(marker_style(theme)),
            _ => None,
        };

        if let Some(style) = style {
            spans.push(StyleSpan {
                start: range.start,
                end: range.end,
                style,
                depth,
            });
        }

        // ---- Bytes ocultables / sustituibles (Nivel 2) ------------------
        // Las reglas se aplican por kind del nodo. Cada caso documenta su
        // razon. Para un mapa visual completo ver docs/ARCHITECTURE.md.

        // (a) Delimitadores inline (`**`/`*`/`` ` ``) cuyo padre es el nodo de
        //     enfasis correspondiente. Se valida el parent inmediato para no
        //     esconder marcadores de bloques que tambien terminan en _marker.
        if kind.ends_with("_delimiter")
            && matches!(parent, Some("strong_emphasis" | "emphasis" | "code_span"))
        {
            for slot in &mut hide_byte[range.start..range.end] {
                *slot = true;
            }
        }

        // (b) Marcador ATX de heading (`#`, `##`, ...): ocultar bytes del
        //     marker mas el whitespace ASCII inmediato hasta el inicio del
        //     texto. tree-sitter-md a veces incluye el espacio en el rango,
        //     a veces no, asi que extendemos por las dudas.
        if kind.starts_with("atx_") && kind.ends_with("_marker") {
            let mut end = range.end;
            while end < source.len() && matches!(source.as_bytes()[end], b' ' | b'\t') {
                end += 1;
            }
            for slot in &mut hide_byte[range.start..end] {
                *slot = true;
            }
        }

        // (c) List markers de bullet (`-`, `*`, `+`): reemplazar el primer
        //     byte por `•`, preservando el resto del rango (que incluye el
        //     espacio). Los ordenados (`1.` / `1)`) NO se tocan: el numero es
        //     informativo.
        if matches!(
            kind,
            "list_marker_minus" | "list_marker_star" | "list_marker_plus"
        ) && range.start < source.len()
        {
            replace_byte[range.start] = Some('•');
        }

        // (d) Block quote primera linea: el `block_quote_marker` aparece en
        //     el block tree. Las lineas de continuacion (`block_continuation`)
        //     NO se visitan por este DFS porque `MarkdownCursor::walk()` salta
        //     al inline subtree al entrar al nodo `inline`. Se procesan en un
        //     segundo pase debajo (ver "Pase extra: block_continuations").
        if kind == "block_quote_marker" && range.start < source.len() {
            replace_byte[range.start] = Some('│');
        }

        // (e) Dentro de un `inline_link` o `image`: ocultar todo lo que NO sea
        //     el texto visible del link (`link_text`) o el alt de la imagen
        //     (`image_description`). Esto cubre `[`, `]`, `(`, `)`, `!` y la
        //     URL (`link_destination`/`link_title`).
        if matches!(parent, Some("inline_link" | "image"))
            && !matches!(kind, "link_text" | "image_description")
        {
            for slot in &mut hide_byte[range.start..range.end] {
                *slot = true;
            }
        }

        // Descender; si no hay hijos, avanzar a hermano o subir.
        if cursor.goto_first_child() {
            stack.push(kind);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            match stack.pop() {
                Some("atx_heading") => heading_level = None,
                Some(_) => {}
                None => break 'dfs, // volvimos a la raiz del walk principal: terminamos el DFS
            }
            cursor.goto_parent();
        }
    }

    // Pase extra: block_continuations. El DFS de `MarkdownCursor::walk()` no
    // los visita porque al entrar al nodo `inline` cambia al subtree inline.
    // Para detectar el `>` de las lineas 2+ de un block_quote walkeamos el
    // block tree puro y reemplazamos el primer byte de cada `block_continuation`
    // cuando es `>`. Hacemos lo mismo aunque ya este reemplazado por el caso
    // (d) del pase principal: es idempotente.
    let mut block_cursor = tree.block_tree().walk();
    loop {
        let node = block_cursor.node();
        if node.kind() == "block_continuation" {
            let r = node.byte_range();
            if source.as_bytes().get(r.start) == Some(&b'>') {
                replace_byte[r.start] = Some('│');
            }
        }
        if block_cursor.goto_first_child() {
            continue;
        }
        loop {
            if block_cursor.goto_next_sibling() {
                break;
            }
            if !block_cursor.goto_parent() {
                return (spans, hide_byte, replace_byte);
            }
        }
    }
}

// --- Render a ratatui ------------------------------------------------------

/// Convierte el documento Markdown en lineas estilizadas listas para ratatui.
///
/// `selection` es un rango en BYTES del documento (ver `Document::selection_byte_range`):
/// los bytes adentro reciben el `bg` de seleccion, preservando el fg/modifiers
/// del estilo de texto que ya tenian (solo se pisa el fondo).
///
/// `matches` son rangos en bytes de coincidencias de busqueda a resaltar; el
/// indice `current` (si hay) marca la coincidencia activa con un color mas vivo.
///
/// `theme` provee la paleta de colores (ver `crate::theme::Theme`).
///
/// `active_line` es el indice de linea con el cursor (0-based). En Nivel 2 esa
/// linea se renderiza como Nivel 1 (markers visibles) para que el mapeo cursor->columna
/// no cambie. `None` desactiva la excepcion (se usa en algunos tests).
///
/// `level` es el modo WYSIWYG: `1` = soft (markers dimmeados pero visibles en
/// todas las lineas), `2` = markers inline ocultos fuera de la linea activa. Si
/// `selection` esta presente o `matches` no esta vacio, forzamos Nivel 1 global
/// para que el highlight siempre caiga sobre celdas visibles.
// Los argumentos son todos contexto de render (texto, theme, estado del
// overlay/seleccion, modo WYSIWYG); agrupar en una struct intermedia anadiria
// indireccion sin claridad.
#[allow(clippy::too_many_arguments)]
pub fn render(
    source: &str,
    selection: Option<std::ops::Range<usize>>,
    matches: &[std::ops::Range<usize>],
    current: Option<usize>,
    theme: &Theme,
    active_line: Option<usize>,
    level: u8,
) -> Vec<Line<'static>> {
    let (spans, hide_byte, replace_byte) = collect_styles(source, theme);

    // Estilo por byte. Pintamos los tramos de menor a mayor profundidad, asi
    // el mas profundo (mas especifico) queda arriba.
    let mut by_byte = vec![Style::default(); source.len()];
    let mut ordered: Vec<&StyleSpan> = spans.iter().collect();
    ordered.sort_by_key(|s| s.depth);
    for span in ordered {
        for slot in &mut by_byte[span.start..span.end] {
            *slot = span.style;
        }
    }

    // Resalte de coincidencias de busqueda (solo pisa el `bg`). La coincidencia
    // actual va con un color mas vivo que el resto.
    for (i, m) in matches.iter().enumerate() {
        let end = m.end.min(source.len());
        if m.start >= end {
            continue;
        }
        let bg = if current == Some(i) {
            theme.search_current_bg
        } else {
            theme.search_match_bg
        };
        for slot in &mut by_byte[m.start..end] {
            *slot = slot.bg(bg);
        }
    }

    // Resalte de seleccion: solo cambia el `bg`, preservando fg/modifiers del
    // texto. Se aplica al final asi gana sobre cualquier fondo previo.
    let has_selection = selection.is_some();
    if let Some(sel) = selection {
        let end = sel.end.min(source.len());
        if sel.start < end {
            for slot in &mut by_byte[sel.start..end] {
                *slot = slot.bg(theme.selection_bg);
            }
        }
    }

    // Nivel 1 forzado: si hay seleccion no vacia o coincidencias activas, o si
    // el caller pidio Nivel 1 explicito, ninguna linea se "contrae".
    let force_full = level <= 1 || has_selection || !matches.is_empty();

    // Agrupar bytes contiguos con el mismo estilo en Spans, linea por linea.
    // En Nivel 2, en las lineas inactivas:
    //   - los bytes en `hide_byte` se omiten (y rompen la run actual),
    //   - los bytes en `replace_byte` se emiten como un Span propio con el
    //     char de reemplazo y el estilo del byte original.
    let mut lines = Vec::new();
    let mut offset = 0;
    for (line_idx, line_str) in source.split('\n').enumerate() {
        let line_start = offset;
        let line_end = offset + line_str.len();
        offset = line_end + 1; // saltar el '\n'

        let show_markers = force_full || active_line == Some(line_idx);

        let mut line_spans: Vec<Span<'static>> = Vec::new();
        let mut k = line_start;
        while k < line_end {
            if !show_markers && hide_byte[k] {
                k += 1;
                continue;
            }
            if !show_markers && let Some(c) = replace_byte[k] {
                // Span aislado con el char sustituido, preservando el estilo
                // (dimming/color del marker original).
                line_spans.push(Span::styled(c.to_string(), by_byte[k]));
                k += 1;
                continue;
            }
            let style = by_byte[k];
            let mut j = k;
            while j < line_end
                && by_byte[j] == style
                && (show_markers || (!hide_byte[j] && replace_byte[j].is_none()))
            {
                j += 1;
            }
            // Los limites de tramo son limites de nodo => limites de char UTF-8.
            line_spans.push(Span::styled(source[k..j].to_string(), style));
            k = j;
        }
        lines.push(Line::from(line_spans));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    /// Theme de referencia para los tests: el default (`frappe`), que conserva
    /// la paleta historica. Los codigos de estilo se comparan contra sus campos.
    fn test_theme() -> Theme {
        Theme::by_name("frappe")
    }

    /// Codigo de una sola letra para resumir el estilo de un byte. Sirve para
    /// validar el mapeo tree-sitter -> estilo en texto plano, sin terminal.
    /// Compara contra los campos de `theme` (antes contra las constantes de
    /// paleta, que ahora viven en el Theme Engine).
    fn code(theme: &Theme, style: &Style) -> char {
        let bold = style.add_modifier.contains(Modifier::BOLD);
        let italic = style.add_modifier.contains(Modifier::ITALIC);
        let is_heading = |c: Option<Color>| {
            c == Some(theme.heading_1) || c == Some(theme.heading_2) || c == Some(theme.heading_n)
        };
        match (style.fg, style.bg) {
            _ if style.fg == Some(theme.marker) => 'm', // marcador dimmeado
            (Some(fg), _) if fg == theme.code_fg => 'C', // inline code
            (fg, _) if is_heading(fg) => 'H',
            _ if bold => 'B',
            _ if italic => 'I',
            _ => '.',
        }
    }

    /// EXPLORACION TEMPORAL: verifica si el DFS de `MarkdownCursor::walk()`
    /// visita los `block_continuation` del block tree (que estan anidados
    /// dentro del paragraph hijo del block_quote).
    #[test]
    fn explora_dfs_visita_block_continuation() {
        let src = "> a\n> b\n> c\n";
        let mut p = tree_sitter_md::MarkdownParser::default();
        let tree = p.parse(src.as_bytes(), None).unwrap();
        let mut cursor = tree.walk();
        let mut visited: Vec<(String, std::ops::Range<usize>)> = Vec::new();
        loop {
            let node = cursor.node();
            visited.push((node.kind().to_string(), node.byte_range()));
            if cursor.goto_first_child() {
                continue;
            }
            loop {
                if cursor.goto_next_sibling() {
                    break;
                }
                if !cursor.goto_parent() {
                    for (k, r) in &visited {
                        println!("VISIT {} [{}..{}]", k, r.start, r.end);
                    }
                    return;
                }
            }
        }
    }

    /// EXPLORACION TEMPORAL: ver el AST de un blockquote de 3+ lineas, que
    /// es donde el render se rompe (el `>` de la 3ra linea no se transforma).
    #[test]
    fn explora_blockquote_3_lineas() {
        fn walk(node: tree_sitter::Node, src: &str, d: usize) {
            let r = node.byte_range();
            let snip = &src[r.start..r.end.min(src.len())];
            let snip = if snip.len() > 50 { &snip[..50] } else { snip };
            println!(
                "{}{} [{}..{}] {:?}",
                "  ".repeat(d),
                node.kind(),
                r.start,
                r.end,
                snip
            );
            let mut c = node.walk();
            for child in node.children(&mut c) {
                walk(child, src, d + 1);
            }
        }
        let src = "> a\n> b\n> c\n";
        let mut p = tree_sitter_md::MarkdownParser::default();
        let tree = p.parse(src.as_bytes(), None).unwrap();
        println!("\n=== BLOCK ===");
        walk(tree.block_tree().root_node(), src, 0);
        println!("\n=== INLINE TREES ===");
        for it in tree.inline_trees() {
            walk(it.root_node(), src, 0);
        }
    }

    /// Vuelca el documento con una linea de codigos de estilo debajo de cada
    /// linea de texto. Correr con: `cargo test -- --nocapture dump`
    #[test]
    fn dump() {
        let theme = test_theme();
        let source =
            std::fs::read_to_string("examples/sample.md").expect("falta examples/sample.md");
        // Nivel 1 (volcado simple, sin ocultar markers).
        let lines = render(&source, None, &[], None, &theme, None, 1);
        println!(
            "\n--- volcado de estilos (.=plano B=bold I=italic C=code H=heading m=marker) ---"
        );
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let codes: String = line
                .spans
                .iter()
                .flat_map(|s| {
                    let c = code(&theme, &s.style);
                    s.content.chars().map(move |_| c)
                })
                .collect();
            println!("{text}");
            if !codes.trim().is_empty() {
                println!("{codes}");
            }
        }
    }

    /// El supuesto critico del spike: los offsets del arbol *inline* tienen que
    /// ser absolutos al documento. Si no lo fueran, el `**` de la negrita en una
    /// linea tardia no caeria sobre los asteriscos reales.
    #[test]
    fn inline_offsets_son_absolutos() {
        let theme = test_theme();
        let source = "# T\n\nplano **negrita** fin\n";
        // Nivel 1 explicito: el test verifica que los `**` esten dimmeados pero
        // visibles (mapeo cursor 1:1 en todas las lineas).
        let lines = render(source, None, &[], None, &theme, None, 1);
        // Linea 2 (indice 2): "plano **negrita** fin"
        let line = &lines[2];
        // Reconstruir el estilo por char y chequear que los '*' esten dimmeados
        // y "negrita" en bold.
        let mut per_char: Vec<(char, char)> = Vec::new();
        for span in &line.spans {
            let c = code(&theme, &span.style);
            for ch in span.content.chars() {
                per_char.push((ch, c));
            }
        }
        let text: String = per_char.iter().map(|(ch, _)| *ch).collect();
        assert_eq!(text, "plano **negrita** fin");
        // los asteriscos: posiciones 6,7 y 15,16 -> 'm'
        for i in [6, 7, 15, 16] {
            assert_eq!(
                per_char[i].1, 'm',
                "asterisco en {i} deberia estar dimmeado"
            );
        }
        // "negrita": posiciones 8..15 -> 'B'
        for (i, slot) in per_char.iter().enumerate().take(15).skip(8) {
            assert_eq!(slot.1, 'B', "char {i} deberia estar en bold");
        }
        // "plano " y " fin" -> '.'
        assert_eq!(per_char[0].1, '.');
    }

    #[test]
    fn seleccion_pinta_el_bg_del_rango() {
        // "hola mundo": seleccionar bytes [0,4) = "hola". Esos chars deben tener
        // el bg de seleccion; el resto no.
        let theme = test_theme();
        let source = "hola mundo";
        let lines = render(source, Some(0..4), &[], None, &theme, None, 1);
        // Reconstruir el bg por char.
        let mut per_char: Vec<(char, Option<Color>)> = Vec::new();
        for span in &lines[0].spans {
            for ch in span.content.chars() {
                per_char.push((ch, span.style.bg));
            }
        }
        // "hola" (0..4) con bg de seleccion.
        for slot in per_char.iter().take(4) {
            assert_eq!(
                slot.1,
                Some(theme.selection_bg),
                "'{}' deberia estar resaltado",
                slot.0
            );
        }
        // El resto sin bg de seleccion.
        for slot in per_char.iter().skip(4) {
            assert_ne!(slot.1, Some(theme.selection_bg));
        }
    }

    #[test]
    fn busqueda_resalta_matches_y_el_actual_distinto() {
        // "ab ab ab": dos matches de "ab" (0..2 y 3..5), el segundo es el actual.
        let theme = test_theme();
        let source = "ab ab ab";
        let matches = vec![0..2, 3..5, 6..8];
        let lines = render(source, None, &matches, Some(1), &theme, None, 1);
        let mut per_char: Vec<(char, Option<Color>)> = Vec::new();
        for span in &lines[0].spans {
            for ch in span.content.chars() {
                per_char.push((ch, span.style.bg));
            }
        }
        // Match 0 (0..2): color de match comun.
        assert_eq!(per_char[0].1, Some(theme.search_match_bg));
        // Match actual (3..5): color vivo.
        assert_eq!(per_char[3].1, Some(theme.search_current_bg));
        assert_eq!(per_char[4].1, Some(theme.search_current_bg));
        // Espacio entre matches: sin resalte.
        assert_eq!(per_char[2].1, None);
    }

    // --- Nivel 2: markers inline ocultos fuera de la linea activa ---------

    /// Junta el texto visible de una linea (sin estilos), ignorando spans vacios.
    fn line_text(lines: &[Line<'static>], idx: usize) -> String {
        lines[idx]
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect()
    }

    #[test]
    fn nivel2_oculta_asteriscos_en_linea_inactiva() {
        // Dos lineas: la activa (0) y otra con negrita (1). En Nivel 2, los
        // `**` de la linea 1 no se renderean.
        let theme = test_theme();
        let source = "linea activa\nplano **negrita** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 0), "linea activa");
        assert_eq!(line_text(&lines, 1), "plano negrita fin");
    }

    #[test]
    fn nivel2_preserva_markers_en_la_linea_activa() {
        // La linea con el cursor mantiene los `**`: el mapeo cursor->columna
        // no cambia respecto a Nivel 1.
        let theme = test_theme();
        let source = "linea activa\nplano **negrita** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2);
        assert_eq!(line_text(&lines, 1), "plano **negrita** fin");
    }

    #[test]
    fn nivel2_oculta_italica_y_code_inline() {
        // `*it*` y `` `cod` `` ambos ocultan sus delimitadores en lineas
        // inactivas.
        let theme = test_theme();
        let source = "cursor\nun *it* y `cod` aca\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "un it y cod aca");
    }

    #[test]
    fn nivel2_oculta_marker_de_heading_en_linea_inactiva() {
        // `# Titular` en una linea inactiva: el `#` + espacio se ocultan, queda
        // solo el texto del heading (que mantiene su estilo bold+color via el
        // `inline` hijo del `atx_heading`).
        let theme = test_theme();
        let source = "# Titular\n\notra linea\n";
        let lines = render(source, None, &[], None, &theme, Some(2), 2);
        assert_eq!(line_text(&lines, 0), "Titular");
    }

    #[test]
    fn nivel2_oculta_h2_y_h3() {
        // Tambien funciona para `##` y `###` (extiende el hide al whitespace
        // posterior aunque tree-sitter no lo incluya en el rango del marker).
        let theme = test_theme();
        let source = "linea\n## H2\n### H3\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "H2");
        assert_eq!(line_text(&lines, 2), "H3");
    }

    #[test]
    fn nivel2_preserva_heading_en_linea_activa() {
        // En la linea activa el `#` se ve, como en Nivel 1.
        let theme = test_theme();
        let source = "antes\n# Activa\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2);
        assert_eq!(line_text(&lines, 1), "# Activa");
    }

    #[test]
    fn nivel2_reemplaza_bullet_de_lista_no_ordenada() {
        // `- item` se renderiza como `• item` en linea inactiva (el guion se
        // sustituye por bullet, el espacio se mantiene). Igual para `*` y `+`.
        let theme = test_theme();
        let source = "cursor\n- uno\n* dos\n+ tres\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "• uno");
        assert_eq!(line_text(&lines, 2), "• dos");
        assert_eq!(line_text(&lines, 3), "• tres");
    }

    #[test]
    fn nivel2_no_toca_listas_ordenadas() {
        // `1. ordn` queda igual: el numero es informativo, no se reemplaza.
        let theme = test_theme();
        let source = "cursor\n1. ordn\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "1. ordn");
    }

    #[test]
    fn nivel2_preserva_bullet_en_linea_activa() {
        // En la linea activa el bullet original (`-`) se mantiene para no
        // sorprender al usuario que esta editando.
        let theme = test_theme();
        let source = "antes\n- activo\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2);
        assert_eq!(line_text(&lines, 1), "- activo");
    }

    #[test]
    fn nivel2_reemplaza_blockquote_con_barra_vertical() {
        // `> cita` -> `│ cita` en linea inactiva. La continuacion en la linea
        // siguiente (`> linea 2`) tambien se sustituye.
        let theme = test_theme();
        let source = "antes\n> cita\n> linea 2\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "│ cita");
        assert_eq!(line_text(&lines, 2), "│ linea 2");
    }

    #[test]
    fn nivel2_oculta_marcadores_de_link() {
        // `[texto](url)` queda como `texto` en linea inactiva.
        let theme = test_theme();
        let source = "cursor\nun [hola](https://x.com) link\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "un hola link");
    }

    #[test]
    fn nivel2_oculta_url_y_parentesis_de_imagen_mostrando_alt() {
        // `![alt](pic.png)` queda como `alt` en linea inactiva.
        let theme = test_theme();
        let source = "cursor\nver ![logo](pic.png) abajo\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "ver logo abajo");
    }

    #[test]
    fn nivel2_preserva_link_completo_en_linea_activa() {
        // En la linea activa el link se ve crudo (la activa renderiza Nivel 1).
        let theme = test_theme();
        let source = "antes\n[ver](https://x.com)\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2);
        assert_eq!(line_text(&lines, 1), "[ver](https://x.com)");
    }

    #[test]
    fn nivel1_explicito_preserva_comportamiento_historico() {
        // Con `level = 1`, ninguna linea oculta markers, exista o no
        // `active_line`.
        let theme = test_theme();
        let source = "cursor\nplano **neg** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 1);
        assert_eq!(line_text(&lines, 1), "plano **neg** fin");
    }

    #[test]
    fn nivel2_con_seleccion_no_vacia_fuerza_nivel1() {
        // Si hay una seleccion no vacia, todas las lineas se renderean en
        // Nivel 1 (markers visibles) para que el highlight caiga sobre
        // celdas reales.
        let theme = test_theme();
        let source = "cursor\nplano **neg** fin\n";
        // Seleccion sobre la linea 0 ("cur" => bytes 0..3).
        let lines = render(source, Some(0..3), &[], None, &theme, Some(0), 2);
        // La linea 1, que seria inactiva, conserva los `**`.
        assert_eq!(line_text(&lines, 1), "plano **neg** fin");
    }

    #[test]
    fn nivel2_con_matches_fuerza_nivel1() {
        // Con matches activos, igual: Nivel 1 global, los markers no se
        // ocultan en ninguna linea.
        let theme = test_theme();
        let source = "cursor\nplano **neg** fin\n";
        // Match arbitrario fuera de la negrita.
        // Un solo rango: pasamos el slice tipado para evitar la ambiguedad
        // de clippy sobre arrays de un elemento.
        let matches: Vec<std::ops::Range<usize>> = vec![0..3, 5..6];
        let lines = render(source, None, &matches, Some(0), &theme, Some(0), 2);
        assert_eq!(line_text(&lines, 1), "plano **neg** fin");
    }

    #[test]
    fn nivel2_oculta_aunque_active_line_sea_none() {
        // `active_line = None` significa "no hay linea exenta": en Nivel 2
        // todas las lineas ocultan delimitadores. Es el modo usado por tests
        // o por un futuro modo preview.
        let theme = test_theme();
        let source = "uno **dos** tres\n";
        let lines = render(source, None, &[], None, &theme, None, 2);
        assert_eq!(line_text(&lines, 0), "uno dos tres");
    }
}
