//! Spike read-only del pipeline de rendering "soft WYSIWYG" (Nivel 1).
//!
//! Pipeline: texto crudo -> tree-sitter-md (block + inline) -> mapa de estilos
//! por byte -> `ratatui::Line`s. Los marcadores de sintaxis nunca se ocultan,
//! solo se dimmean (ver docs/ARCHITECTURE.md, modulo Renderer Engine).

use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use tree_sitter_md::MarkdownParser;

// --- Paleta (hardcodeada para el spike; sale de Catppuccin Frappe) ---------
// En el editor real esto viene del Theme Engine (src/theme/).

const HEADING_1: Color = Color::Rgb(0xca, 0x9e, 0xe6); // mauve
const HEADING_2: Color = Color::Rgb(0x99, 0xd1, 0xdb); // sky
const HEADING_N: Color = Color::Rgb(0xa6, 0xd1, 0x89); // green
const CODE_FG: Color = Color::Rgb(0xe7, 0x82, 0x84); // red
const CODE_BG: Color = Color::Rgb(0x41, 0x45, 0x59); // surface0
const MARKER: Color = Color::Rgb(0x73, 0x7a, 0x94); // overlay0 (dimmeado)
const SELECTION_BG: Color = Color::Rgb(0x51, 0x57, 0x6d); // surface1 (resalte sutil)

fn heading_style(level: usize) -> Style {
    let fg = match level {
        1 => HEADING_1,
        2 => HEADING_2,
        _ => HEADING_N,
    };
    Style::default().fg(fg).add_modifier(Modifier::BOLD)
}

fn marker_style() -> Style {
    Style::default().fg(MARKER)
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

/// Recorre el arbol Markdown (block + inline) y junta los tramos estilizados.
fn collect_styles(source: &str) -> Vec<StyleSpan> {
    let mut parser = MarkdownParser::default();
    let tree = parser
        .parse(source.as_bytes(), None)
        .expect("tree-sitter-md no pudo parsear el documento");

    let mut spans: Vec<StyleSpan> = Vec::new();
    let mut cursor = tree.walk();

    // DFS iterativo. `stack` guarda los kinds de los ancestros para saber, por
    // ejemplo, que un nodo `inline` cuelga de un `atx_heading`. La profundidad
    // es `stack.len()`.
    let mut stack: Vec<&str> = Vec::new();
    let mut heading_level: Option<usize> = None;

    loop {
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
            "inline" if parent == Some("atx_heading") => heading_level.map(heading_style),
            "strong_emphasis" => Some(Style::default().add_modifier(Modifier::BOLD)),
            "emphasis" => Some(Style::default().add_modifier(Modifier::ITALIC)),
            "code_span" => Some(Style::default().fg(CODE_FG).bg(CODE_BG)),
            // Todos los marcadores y delimitadores: dimmeados.
            k if k.ends_with("_marker") || k.ends_with("_delimiter") => Some(marker_style()),
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
                None => return spans, // volvimos a la raiz: terminamos
            }
            cursor.goto_parent();
        }
    }
}

// --- Render a ratatui ------------------------------------------------------

/// Convierte el documento Markdown en lineas estilizadas listas para ratatui.
///
/// `selection` es un rango en BYTES del documento (ver `Document::selection_byte_range`):
/// los bytes adentro reciben el `bg` de seleccion, preservando el fg/modifiers
/// del estilo de texto que ya tenian (solo se pisa el fondo).
pub fn render(source: &str, selection: Option<std::ops::Range<usize>>) -> Vec<Line<'static>> {
    let spans = collect_styles(source);

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

    // Resalte de seleccion: solo cambia el `bg`, preservando fg/modifiers del
    // texto. Se aplica al final asi gana sobre cualquier fondo previo.
    if let Some(sel) = selection {
        let end = sel.end.min(source.len());
        if sel.start < end {
            for slot in &mut by_byte[sel.start..end] {
                *slot = slot.bg(SELECTION_BG);
            }
        }
    }

    // Agrupar bytes contiguos con el mismo estilo en Spans, linea por linea.
    let mut lines = Vec::new();
    let mut offset = 0;
    for line_str in source.split('\n') {
        let line_start = offset;
        let line_end = offset + line_str.len();
        offset = line_end + 1; // saltar el '\n'

        let mut line_spans: Vec<Span<'static>> = Vec::new();
        let mut k = line_start;
        while k < line_end {
            let style = by_byte[k];
            let mut j = k;
            while j < line_end && by_byte[j] == style {
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

    /// Codigo de una sola letra para resumir el estilo de un byte. Sirve para
    /// validar el mapeo tree-sitter -> estilo en texto plano, sin terminal.
    fn code(style: &Style) -> char {
        let bold = style.add_modifier.contains(Modifier::BOLD);
        let italic = style.add_modifier.contains(Modifier::ITALIC);
        match (style.fg, style.bg) {
            _ if style.fg == Some(MARKER) => 'm', // marcador dimmeado
            (Some(CODE_FG), _) => 'C',            // inline code
            (Some(HEADING_1), _) | (Some(HEADING_2), _) | (Some(HEADING_N), _) => 'H',
            _ if bold => 'B',
            _ if italic => 'I',
            _ => '.',
        }
    }

    /// Vuelca el documento con una linea de codigos de estilo debajo de cada
    /// linea de texto. Correr con: `cargo test -- --nocapture dump`
    #[test]
    fn dump() {
        let source =
            std::fs::read_to_string("examples/sample.md").expect("falta examples/sample.md");
        let lines = render(&source, None);
        println!(
            "\n--- volcado de estilos (.=plano B=bold I=italic C=code H=heading m=marker) ---"
        );
        for line in &lines {
            let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
            let codes: String = line
                .spans
                .iter()
                .flat_map(|s| {
                    let c = code(&s.style);
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
        let source = "# T\n\nplano **negrita** fin\n";
        let lines = render(source, None);
        // Linea 2 (indice 2): "plano **negrita** fin"
        let line = &lines[2];
        // Reconstruir el estilo por char y chequear que los '*' esten dimmeados
        // y "negrita" en bold.
        let mut per_char: Vec<(char, char)> = Vec::new();
        for span in &line.spans {
            let c = code(&span.style);
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
        let source = "hola mundo";
        let lines = render(source, Some(0..4));
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
                Some(SELECTION_BG),
                "'{}' deberia estar resaltado",
                slot.0
            );
        }
        // El resto sin bg de seleccion.
        for slot in per_char.iter().skip(4) {
            assert_ne!(slot.1, Some(SELECTION_BG));
        }
    }
}
