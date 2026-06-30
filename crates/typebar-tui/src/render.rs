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
use tree_sitter::Node;
use tree_sitter_md::MarkdownParser;
use unicode_width::UnicodeWidthStr;

use crate::theme::Theme;

/// Margen a la derecha (en celdas) que se agrega al ancho del contenido al
/// dibujar la "caja" de un bloque de codigo, para que el texto no quede pegado
/// al borde derecho del fondo.
const CODE_BOX_PAD: usize = 2;

/// Margen a la izquierda (en celdas) dentro de la "caja" de codigo: el contenido
/// se corre a la derecha esta cantidad para que no quede pegado al borde
/// izquierdo del fondo. El cursor lo compensa en `main` (ver `code_line_flags`).
pub const CODE_BOX_LEFT_PAD: usize = 1;

/// Margen a la derecha (en celdas) que `main` deja libre entre la caja de codigo
/// y el borde del area de texto: la caja se extiende casi de lado a lado pero se
/// detiene esta cantidad antes del borde, para que "respire" dentro de un margen
/// en vez de pegarse al filo. Lo usa `main` al calcular el `code_box_width` que
/// le pasa a `render`.
pub const CODE_BOX_RIGHT_MARGIN: usize = 4;

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
fn collect_styles(
    source: &str,
    theme: &Theme,
) -> (Vec<StyleSpan>, Vec<bool>, Vec<Option<char>>, Vec<usize>) {
    let mut parser = MarkdownParser::default();
    let tree = parser
        .parse(source.as_bytes(), None)
        .expect("tree-sitter-md no pudo parsear el documento");

    let mut spans: Vec<StyleSpan> = Vec::new();
    let mut hide_byte: Vec<bool> = vec![false; source.len()];
    let mut replace_byte: Vec<Option<char>> = vec![None; source.len()];
    // Rangos (en bytes) de los bloques de codigo, para calcular el relleno de
    // "caja" por linea una vez terminado el DFS.
    let mut code_blocks: Vec<std::ops::Range<usize>> = Vec::new();
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
            // Bloques de codigo (fenced e indentado): pintamos TODO el rango
            // del bloque con el estilo de codigo (fg+bg) para que se lea como
            // una "caja", no como prosa. Es un nodo de bloque (poca
            // profundidad), asi que los hijos mas profundos pueden refinar (ver
            // la cerca abajo). El contenido (`code_fence_content`, `info_string`
            // y los tokens internos) no tiene brazo propio, asi que hereda este
            // estilo.
            "fenced_code_block" | "indented_code_block" => {
                Some(Style::default().fg(theme.code_fg).bg(theme.code_bg))
            }
            // La cerca ``` de apertura/cierre: dimmeada como marcador pero sobre
            // el fondo de codigo, asi la linea forma parte de la caja. Debe ir
            // antes del brazo generico `_delimiter` (que no lleva fondo).
            "fenced_code_block_delimiter" => Some(marker_style(theme).bg(theme.code_bg)),
            // Tablas (estilo-only, sin re-alinear): NO se inserta padding, asi el
            // mapeo 1:1 y el cursor quedan intactos; solo se da jerarquia visual.
            // El header en negrita; los pipes `|` y la fila delimitadora (`|---|`)
            // atenuados como estructura. Los `:` de alignment se acentuan aparte
            // (ver el pase de bytes mas abajo).
            "pipe_table_cell" if parent == Some("pipe_table_header") => {
                Some(Style::default().add_modifier(Modifier::BOLD))
            }
            "pipe_table_delimiter_row" => Some(marker_style(theme)),
            "|" if matches!(
                parent,
                Some("pipe_table_header" | "pipe_table_row" | "pipe_table_delimiter_row")
            ) =>
            {
                Some(marker_style(theme))
            }
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

        // (f) Bloques de codigo: registrar el rango para el relleno de "caja"
        //     (ver el calculo de `code_pad` al final).
        if matches!(kind, "fenced_code_block" | "indented_code_block") {
            code_blocks.push(range.clone());
        }

        // (f.2) Tablas: en la fila delimitadora, acentuar los `:` de alignment
        //       (`:--`, `--:`, `:-:`) con el color de heading, por encima del dim
        //       de la fila. Se hace a nivel byte (robusto ante como tree-sitter-md
        //       modele el alignment) con `depth + 1` para ganarle al span de la
        //       fila (mismo o menor depth).
        if kind == "pipe_table_delimiter_row" {
            for (i, &b) in source.as_bytes()[range.start..range.end].iter().enumerate() {
                if b == b':' {
                    let at = range.start + i;
                    spans.push(StyleSpan {
                        start: at,
                        end: at + 1,
                        style: Style::default().fg(theme.heading_1),
                        depth: depth + 1,
                    });
                }
            }
        }

        // (g) Cercas de un bloque fenced (``` de apertura/cierre) y su info
        //     string (el lenguaje tras ```): se ocultan en lineas inactivas, asi
        //     la linea queda como banda limpia del fondo de codigo (sin los
        //     backticks) formando el borde superior/inferior de la caja. En la
        //     linea activa reaparecen (show_markers) para poder editarlas. El
        //     relleno de "caja" mantiene el ancho aunque la linea quede vacia.
        if matches!(kind, "fenced_code_block_delimiter" | "info_string") {
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

    // Relleno de "caja" de los bloques de codigo: `code_pad[i]` es el ancho
    // visual objetivo de la linea `i` si pertenece a un bloque de codigo (0 si
    // no). El render rellena cada linea de codigo a la derecha hasta ese ancho
    // con el fondo de codigo, formando un rectangulo en vez de pegar el fondo al
    // texto. El ancho es la linea mas ancha del bloque + `CODE_BOX_PAD`.
    let line_count = source.split('\n').count();
    let mut code_pad: Vec<usize> = vec![0; line_count];
    if !code_blocks.is_empty() {
        // Offsets de inicio de cada linea, para mapear byte -> indice de linea.
        let mut line_starts: Vec<usize> = vec![0];
        for (i, b) in source.bytes().enumerate() {
            if b == b'\n' {
                line_starts.push(i + 1);
            }
        }
        let lines: Vec<&str> = source.split('\n').collect();
        let line_of = |byte: usize| match line_starts.binary_search(&byte) {
            Ok(i) => i,
            Err(i) => i - 1,
        };
        for r in &code_blocks {
            if r.start >= r.end {
                continue;
            }
            let first = line_of(r.start);
            let last = line_of(r.end - 1).min(line_count - 1);
            let width = (first..=last)
                .map(|i| UnicodeWidthStr::width(lines[i]))
                .max()
                .unwrap_or(0)
                + CODE_BOX_LEFT_PAD
                + CODE_BOX_PAD;
            for slot in &mut code_pad[first..=last] {
                *slot = (*slot).max(width);
            }
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
                return (spans, hide_byte, replace_byte, code_pad);
            }
        }
    }
}

// --- Tablas: layout de grilla para las filas inactivas ----------------------

/// Alineacion de una columna, leida de la fila delimitadora (`:--`, `--:`, `:-:`).
#[derive(Clone, Copy, PartialEq)]
enum TableAlign {
    Left,
    Center,
    Right,
}

/// Tipo de fila dentro de una tabla.
#[derive(Clone, Copy, PartialEq)]
enum TableRowKind {
    Header,
    Delimiter,
    Body,
}

/// Una linea de tabla lista para dibujarse como grilla: su tipo, el texto ya
/// trimmeado de cada celda (vacio en la delimitadora) y -compartidos por toda la
/// tabla- el ancho y la alineacion de cada columna.
struct TableLine {
    kind: TableRowKind,
    cells: Vec<String>,
    widths: Vec<usize>,
    aligns: Vec<TableAlign>,
}

/// Devuelve, por linea del documento, su `TableLine` si esa linea es parte de una
/// tabla (`None` si no). El render dibuja las filas INACTIVAS como grilla alineada
/// (la activa va cruda, ver `render`). Mantiene el mapeo 1:1 linea-source ->
/// linea-pantalla: cada fila (header, delimitadora, cuerpo) ocupa una sola linea,
/// igual que en el markdown. No hay borde superior/inferior: no existe una linea de
/// source para ellos y meterlos romperia el mapeo del cursor (estilo GitHub).
fn collect_tables(source: &str) -> Vec<Option<TableLine>> {
    let line_count = source.split('\n').count();
    let mut out: Vec<Option<TableLine>> = (0..line_count).map(|_| None).collect();

    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return out;
    };

    let mut cursor = tree.block_tree().walk();
    loop {
        let node = cursor.node();
        if node.kind() == "pipe_table" {
            process_table(&node, source, &mut out);
            // No descendemos: `process_table` ya recorrio toda la tabla.
        } else if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return out;
            }
        }
    }
}

/// Procesa un nodo `pipe_table`: calcula anchos/alineaciones por columna y escribe
/// un `TableLine` en `out` por cada fila (indexado por su linea de source).
fn process_table(table: &Node, source: &str, out: &mut [Option<TableLine>]) {
    let mut tc = table.walk();
    let row_nodes: Vec<Node> = table.children(&mut tc).collect();

    let mut rows: Vec<(usize, TableRowKind, Vec<String>)> = Vec::new();
    let mut aligns: Vec<TableAlign> = Vec::new();
    for row in &row_nodes {
        let line = row.start_position().row;
        match row.kind() {
            "pipe_table_header" => rows.push((line, TableRowKind::Header, row_cells(row, source))),
            "pipe_table_row" => rows.push((line, TableRowKind::Body, row_cells(row, source))),
            "pipe_table_delimiter_row" => {
                aligns = row_aligns(row, source);
                rows.push((line, TableRowKind::Delimiter, Vec::new()));
            }
            _ => {}
        }
    }

    let ncols = rows.iter().map(|(_, _, c)| c.len()).max().unwrap_or(0);
    if ncols == 0 {
        return;
    }

    // Ancho de columna = max ancho VISUAL del contenido (header + cuerpo; la fila
    // delimitadora no cuenta).
    let mut widths = vec![0usize; ncols];
    for (_, kind, cells) in &rows {
        if *kind == TableRowKind::Delimiter {
            continue;
        }
        for (i, w) in widths.iter_mut().enumerate() {
            let cell_w = cells
                .get(i)
                .map(|s| UnicodeWidthStr::width(s.as_str()))
                .unwrap_or(0);
            *w = (*w).max(cell_w);
        }
    }
    aligns.resize(ncols, TableAlign::Left);

    for (line, kind, mut cells) in rows {
        cells.resize(ncols, String::new());
        if let Some(slot) = out.get_mut(line) {
            *slot = Some(TableLine {
                kind,
                cells,
                widths: widths.clone(),
                aligns: aligns.clone(),
            });
        }
    }
}

/// Texto trimmeado de cada `pipe_table_cell` de una fila, en orden.
fn row_cells(row: &Node, source: &str) -> Vec<String> {
    let mut c = row.walk();
    row.children(&mut c)
        .filter(|n| n.kind() == "pipe_table_cell")
        .map(|n| source[n.byte_range()].trim().to_string())
        .collect()
}

/// Alineacion de cada `pipe_table_delimiter_cell`, leida de los `:` (`:--` izq,
/// `--:` der, `:-:` centro, default izq).
fn row_aligns(row: &Node, source: &str) -> Vec<TableAlign> {
    let mut c = row.walk();
    row.children(&mut c)
        .filter(|n| n.kind() == "pipe_table_delimiter_cell")
        .map(|n| {
            let t = source[n.byte_range()].trim();
            match (t.starts_with(':'), t.ends_with(':')) {
                (true, true) => TableAlign::Center,
                (false, true) => TableAlign::Right,
                _ => TableAlign::Left,
            }
        })
        .collect()
}

/// Spans de una linea de tabla como grilla alineada (box-drawing). El header va en
/// negrita; los bordes (`│ ├ ┼ ┤ ─`) atenuados. Estilo GitHub: sin borde
/// superior/inferior (no hay linea de source para ellos), solo verticales y el
/// separador del header (que sale de la fila delimitadora). Cada columna ocupa
/// `width + 2` celdas (un espacio de padding a cada lado), asi los `│` y los `┼`
/// caen en la misma columna en todas las filas.
fn render_table_line(info: &TableLine, theme: &Theme) -> Vec<Span<'static>> {
    let border = marker_style(theme);
    let ncols = info.widths.len();
    let mut spans: Vec<Span<'static>> = Vec::new();

    if info.kind == TableRowKind::Delimiter {
        spans.push(Span::styled("├".to_string(), border));
        for (i, w) in info.widths.iter().enumerate() {
            spans.push(Span::styled("─".repeat(w + 2), border));
            let junction = if i + 1 < ncols { "┼" } else { "┤" };
            spans.push(Span::styled(junction.to_string(), border));
        }
        return spans;
    }

    let cell_style = if info.kind == TableRowKind::Header {
        Style::default().add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    spans.push(Span::styled("│".to_string(), border));
    for (i, cell) in info.cells.iter().enumerate() {
        let pad = info.widths[i].saturating_sub(UnicodeWidthStr::width(cell.as_str()));
        let (lp, rp) = match info.aligns[i] {
            TableAlign::Center => (pad / 2, pad - pad / 2),
            TableAlign::Right => (pad, 0),
            TableAlign::Left => (0, pad),
        };
        // ` {leftpad}{contenido}{rightpad} ` + el `│` de cierre.
        spans.push(Span::styled(" ".repeat(1 + lp), Style::default()));
        spans.push(Span::styled(cell.clone(), cell_style));
        spans.push(Span::styled(" ".repeat(rp + 1), Style::default()));
        spans.push(Span::styled("│".to_string(), border));
    }
    spans
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
///
/// `code_box_width` es el ancho visual (en celdas) al que se extiende la "caja"
/// de los bloques de codigo: `main` le pasa el ancho del area de texto menos
/// `CODE_BOX_RIGHT_MARGIN`, asi la caja llega casi de lado a lado pero dentro de
/// un margen. `0` desactiva el ensanchado y la caja se ajusta al contenido (lo
/// usan los tests que no dependen del ancho del viewport). Si el contenido es mas
/// ancho que `code_box_width`, gana el contenido (no se recorta).
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
    code_box_width: usize,
) -> Vec<Line<'static>> {
    let (spans, hide_byte, replace_byte, code_pad) = collect_styles(source, theme);
    // Layout de grilla por linea para las tablas (None si la linea no es tabla).
    let table_lines = collect_tables(source);

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

        // Tablas: las filas INACTIVAS (sin cursor) se dibujan como grilla alineada
        // (box-drawing). La fila activa cae al render crudo de abajo, asi el cursor
        // mapea 1:1. Con `force_full` (Nivel 1, o seleccion/busqueda) la tabla va
        // cruda, para que el highlight quede sobre celdas reales.
        if !show_markers && let Some(info) = table_lines.get(line_idx).and_then(|o| o.as_ref()) {
            lines.push(Line::from(render_table_line(info, theme)));
            continue;
        }

        let mut line_spans: Vec<Span<'static>> = Vec::new();

        // Margen izquierdo de la caja de codigo: corre el contenido a la
        // derecha (el cursor lo compensa en `main`). Va antes del contenido.
        let in_code = code_pad.get(line_idx).copied().unwrap_or(0) > 0;
        if in_code && CODE_BOX_LEFT_PAD > 0 {
            line_spans.push(Span::styled(
                " ".repeat(CODE_BOX_LEFT_PAD),
                Style::default().bg(theme.code_bg),
            ));
        }

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

        // Relleno de "caja": si la linea pertenece a un bloque de codigo,
        // extender el fondo de codigo a la derecha hasta el ancho objetivo del
        // bloque. Asi todas las lineas del bloque (contenido y cercas) forman un
        // rectangulo uniforme. Las cercas, ocultas en lineas inactivas, quedan
        // como bandas vacias del mismo ancho.
        let pad_to = code_pad.get(line_idx).copied().unwrap_or(0);
        if pad_to > 0 {
            // Ancho objetivo de la caja: el que pide el caller (area de texto menos
            // el margen derecho), o el ancho del contenido si es mas ancho o si el
            // caller paso 0 (modo legacy / tests sin viewport).
            let target = code_box_width.max(pad_to);
            let cur: usize = line_spans
                .iter()
                .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
                .sum();
            if cur < target {
                line_spans.push(Span::styled(
                    " ".repeat(target - cur),
                    Style::default().bg(theme.code_bg),
                ));
            }
        }

        lines.push(Line::from(line_spans));
    }
    lines
}

/// Marca, por linea (0-based, indexando como `source.split('\n')`), si pertenece
/// a un bloque de codigo (fenced o indentado). `main` lo usa para correr el
/// cursor `CODE_BOX_LEFT_PAD` celdas cuando esta sobre una linea de codigo, ya
/// que el render aplica ese margen izquierdo a la caja.
pub fn code_line_flags(source: &str) -> Vec<bool> {
    let line_count = source.split('\n').count();
    let mut flags = vec![false; line_count];

    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return flags;
    };

    let mut line_starts: Vec<usize> = vec![0];
    for (i, b) in source.bytes().enumerate() {
        if b == b'\n' {
            line_starts.push(i + 1);
        }
    }
    let line_of = |byte: usize| match line_starts.binary_search(&byte) {
        Ok(i) => i,
        Err(i) => i - 1,
    };

    let mut cursor = tree.block_tree().walk();
    loop {
        let node = cursor.node();
        if matches!(node.kind(), "fenced_code_block" | "indented_code_block") {
            let r = node.byte_range();
            if r.start < r.end {
                let first = line_of(r.start);
                let last = line_of(r.end - 1).min(line_count - 1);
                for f in &mut flags[first..=last] {
                    *f = true;
                }
            }
        }
        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return flags;
            }
        }
    }
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

    /// Vuelca el documento con una linea de codigos de estilo debajo de cada
    /// linea de texto. Correr con: `cargo test -- --nocapture dump`
    #[test]
    fn dump() {
        let theme = test_theme();
        // Ruta anclada al crate (CARGO_MANIFEST_DIR), no al CWD: asi funciona
        // se corra `cargo test` desde la raiz del workspace o desde el crate.
        let fixture = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/sample.md");
        let source = std::fs::read_to_string(fixture).expect("falta examples/sample.md");
        // Nivel 1 (volcado simple, sin ocultar markers).
        let lines = render(&source, None, &[], None, &theme, None, 1, 0);
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
        let lines = render(source, None, &[], None, &theme, None, 1, 0);
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
        let lines = render(source, Some(0..4), &[], None, &theme, None, 1, 0);
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
        let lines = render(source, None, &matches, Some(1), &theme, None, 1, 0);
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

    // --- Bloques de codigo (fenced e indentado) ---------------------------

    /// Ancho visual total de una linea (suma de los anchos de sus spans).
    fn line_width(lines: &[Line<'static>], idx: usize) -> usize {
        lines[idx]
            .spans
            .iter()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
            .sum()
    }

    /// Ancho de caja para un bloque cuya linea mas ancha mide `content`.
    fn box_width(content: usize) -> usize {
        CODE_BOX_LEFT_PAD + content + CODE_BOX_PAD
    }

    #[test]
    fn fenced_code_block_pinta_el_cuerpo_como_caja() {
        // El contenido lee con fg/bg de codigo, con margen a izquierda y derecha
        // hasta el ancho de la caja. Cursor afuera (linea 0), Nivel 2.
        let theme = test_theme();
        let source = "antes\n```rust\nlet x = 1;\n```\ndespues\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        // El texto visible (sin contar los margenes) queda intacto.
        assert_eq!(line_text(&lines, 2).trim(), "let x = 1;");
        // Ancho de caja = margen_izq + max(7, 10, 3) + margen_der.
        let box_w = box_width("let x = 1;".len());
        assert_eq!(line_width(&lines, 2), box_w);
        // Todas las celdas (margenes + texto) llevan el fondo de codigo.
        for span in &lines[2].spans {
            for _ in span.content.chars() {
                assert_eq!(span.style.bg, Some(theme.code_bg));
            }
        }
    }

    #[test]
    fn code_box_width_extiende_la_caja_hasta_el_ancho_pedido() {
        // Con un `code_box_width` mayor que el contenido, todas las lineas del
        // bloque (cercas y cuerpo) se extienden a ese ancho, no al del contenido:
        // la caja llega "dentro de un margen" en vez de pegarse al texto.
        let theme = test_theme();
        let source = "antes\n```rust\nlet x = 1;\n```\ndespues\n";
        let wide = 60;
        let lines = render(source, None, &[], None, &theme, Some(0), 2, wide);
        for idx in [1, 2, 3] {
            assert_eq!(line_width(&lines, idx), wide);
        }
        // El texto sigue intacto; solo se agrego relleno de fondo a la derecha.
        assert_eq!(line_text(&lines, 2).trim(), "let x = 1;");
    }

    #[test]
    fn code_box_width_menor_que_el_contenido_no_recorta() {
        // Si el contenido es mas ancho que `code_box_width`, gana el contenido:
        // la caja nunca recorta el codigo.
        let theme = test_theme();
        let source = "antes\n```\nlet x = 1;\n```\ndespues\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 3);
        let box_w = box_width("let x = 1;".len());
        assert_eq!(line_width(&lines, 2), box_w);
    }

    #[test]
    fn fenced_code_block_oculta_las_cercas_como_banda() {
        // Las cercas (apertura con info string y cierre) se ocultan en lineas
        // inactivas: quedan como banda limpia (solo fondo de codigo, sin los
        // backticks) formando el borde de la caja. El cursor esta en la linea 0,
        // asi que las cercas (lineas 1 y 3) estan inactivas.
        let theme = test_theme();
        let source = "antes\n```rust\nlet x = 1;\n```\ndespues\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1).trim(), "");
        assert_eq!(line_text(&lines, 3).trim(), "");
        // Aun ocultas, las lineas de cerca conservan el ancho de caja y el fondo
        // de codigo (la banda se mantiene gracias al relleno).
        let box_w = box_width("let x = 1;".len());
        for idx in [1, 2, 3] {
            assert_eq!(line_width(&lines, idx), box_w);
            for span in &lines[idx].spans {
                assert_eq!(span.style.bg, Some(theme.code_bg));
            }
        }
    }

    #[test]
    fn fenced_code_block_muestra_la_cerca_en_la_linea_activa() {
        // En la linea activa las cercas reaparecen (con su info string) para
        // poder editarlas: el cursor en la linea 1 (apertura ```rust).
        let theme = test_theme();
        let source = "antes\n```rust\nlet x = 1;\n```\ndespues\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1).trim(), "```rust");
        // La cerca de cierre (linea 3) sigue inactiva => oculta.
        assert_eq!(line_text(&lines, 3).trim(), "");
    }

    #[test]
    fn indented_code_block_se_pinta_como_caja() {
        // Un bloque indentado (4 espacios) tambien recibe el fondo de codigo y
        // los margenes de caja.
        let theme = test_theme();
        let source = "antes\n\n    let y = 2;\n\ndespues\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 2).trim(), "let y = 2;");
        let box_w = box_width("    let y = 2;".len());
        assert_eq!(line_width(&lines, 2), box_w);
        for span in &lines[2].spans {
            for _ in span.content.chars() {
                assert_eq!(span.style.bg, Some(theme.code_bg));
            }
        }
    }

    #[test]
    fn code_line_flags_marca_solo_las_lineas_de_codigo() {
        // `code_line_flags` debe marcar las lineas del bloque (incluidas las
        // cercas) y nada mas; `main` lo usa para correr el cursor.
        let source = "antes\n```rust\nlet x = 1;\n```\ndespues\n";
        let flags = code_line_flags(source);
        assert!(!flags[0], "antes");
        assert!(flags[1], "```rust");
        assert!(flags[2], "let x = 1;");
        assert!(flags[3], "```");
        assert!(!flags[4], "despues");
    }

    // --- Tablas: grilla en filas inactivas, crudo en la activa -------------

    /// Primer span de la linea `idx` cuyo contenido es exactamente `content`.
    fn span_of<'a>(
        lines: &'a [Line<'static>],
        idx: usize,
        content: &str,
    ) -> Option<&'a Span<'static>> {
        lines[idx]
            .spans
            .iter()
            .find(|s| s.content.as_ref() == content)
    }

    #[test]
    fn tabla_fila_activa_se_renderiza_cruda_con_estilo() {
        // La fila con el cursor (activa) va CRUDA (1:1, sin grilla) con el realce
        // estilo-only: header en negrita y pipes atenuados. Asi el cursor mapea a
        // los bytes mientras editas esa fila.
        let theme = test_theme();
        let source = "| Name | Age |\n|------|-----|\n| Ada  | 36  |\n";
        // Cursor en el header (linea 0): esa fila va cruda.
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 0), "| Name | Age |");
        assert!(
            span_of(&lines, 0, "Name ")
                .unwrap()
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        // Pipes atenuados (color marker).
        for s in &lines[0].spans {
            if s.content.as_ref() == "|" {
                assert_eq!(s.style.fg, Some(theme.marker));
            }
        }
    }

    #[test]
    fn tabla_fila_delimitadora_activa_atenuada_y_colons_acentuados() {
        // En la fila delimitadora ACTIVA (cruda) se ve el realce estilo-only: los
        // guiones atenuados y los `:` de alignment acentuados.
        let theme = test_theme();
        let source = "| A | B |\n| :--- | ---: |\n| x | y |\n";
        // Cursor en la delimitadora (linea 1): va cruda.
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1), "| :--- | ---: |");
        assert_eq!(
            span_of(&lines, 1, ":").unwrap().style.fg,
            Some(theme.heading_1)
        );
        let dash = lines[1]
            .spans
            .iter()
            .find(|s| s.content.as_ref().contains('-'))
            .unwrap();
        assert_eq!(dash.style.fg, Some(theme.marker));
    }

    #[test]
    fn tabla_inactiva_se_alinea_en_grilla() {
        // Con el cursor FUERA de la tabla, las filas se dibujan como grilla alineada
        // (box-drawing), no como markdown crudo. Columnas de anchos distintos para
        // verificar que el padding las empareja.
        let theme = test_theme();
        let source = "| a | bbbb |\n| --- | --- |\n| cc | d |\n\nx\n";
        // Cursor en la linea 4 ('x'), fuera de la tabla.
        let lines = render(source, None, &[], None, &theme, Some(4), 2, 0);
        let header = line_text(&lines, 0);
        let body = line_text(&lines, 2);
        // Bordes box-drawing, sin pipes ASCII.
        assert!(header.contains('│') && body.contains('│'));
        assert!(!header.contains('|') && !body.contains('|'));
        // Header y cuerpo alinean al MISMO ancho (columnas en grilla).
        assert_eq!(line_width(&lines, 0), line_width(&lines, 2));
        // La delimitadora es el separador `├─┼─┤`.
        let delim = line_text(&lines, 1);
        assert!(delim.starts_with('├') && delim.contains('┼') && delim.ends_with('┤'));
    }

    #[test]
    fn tabla_grilla_respeta_alignment_a_la_derecha() {
        // Columna right-aligned (`--:`): el padding va a la IZQUIERDA del contenido.
        let theme = test_theme();
        let source = "| num |\n| --: |\n| 7 |\n\nx\n";
        // Cursor fuera de la tabla (linea 4).
        let lines = render(source, None, &[], None, &theme, Some(4), 2, 0);
        // Ancho de columna = max("num"=3, "7"=1) = 3. El "7" right-aligned queda
        // pegado a la derecha: tres espacios antes del 7 dentro de la celda.
        let body = line_text(&lines, 2);
        assert!(
            body.contains("   7"),
            "right-align: el 7 va a la derecha (padding a la izquierda): {body:?}"
        );
    }

    #[test]
    fn nivel2_oculta_asteriscos_en_linea_inactiva() {
        // Dos lineas: la activa (0) y otra con negrita (1). En Nivel 2, los
        // `**` de la linea 1 no se renderean.
        let theme = test_theme();
        let source = "linea activa\nplano **negrita** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 0), "linea activa");
        assert_eq!(line_text(&lines, 1), "plano negrita fin");
    }

    #[test]
    fn nivel2_preserva_markers_en_la_linea_activa() {
        // La linea con el cursor mantiene los `**`: el mapeo cursor->columna
        // no cambia respecto a Nivel 1.
        let theme = test_theme();
        let source = "linea activa\nplano **negrita** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1), "plano **negrita** fin");
    }

    #[test]
    fn nivel2_oculta_italica_y_code_inline() {
        // `*it*` y `` `cod` `` ambos ocultan sus delimitadores en lineas
        // inactivas.
        let theme = test_theme();
        let source = "cursor\nun *it* y `cod` aca\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "un it y cod aca");
    }

    #[test]
    fn nivel2_oculta_marker_de_heading_en_linea_inactiva() {
        // `# Titular` en una linea inactiva: el `#` + espacio se ocultan, queda
        // solo el texto del heading (que mantiene su estilo bold+color via el
        // `inline` hijo del `atx_heading`).
        let theme = test_theme();
        let source = "# Titular\n\notra linea\n";
        let lines = render(source, None, &[], None, &theme, Some(2), 2, 0);
        assert_eq!(line_text(&lines, 0), "Titular");
    }

    #[test]
    fn nivel2_oculta_h2_y_h3() {
        // Tambien funciona para `##` y `###` (extiende el hide al whitespace
        // posterior aunque tree-sitter no lo incluya en el rango del marker).
        let theme = test_theme();
        let source = "linea\n## H2\n### H3\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "H2");
        assert_eq!(line_text(&lines, 2), "H3");
    }

    #[test]
    fn nivel2_preserva_heading_en_linea_activa() {
        // En la linea activa el `#` se ve, como en Nivel 1.
        let theme = test_theme();
        let source = "antes\n# Activa\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1), "# Activa");
    }

    #[test]
    fn nivel2_reemplaza_bullet_de_lista_no_ordenada() {
        // `- item` se renderiza como `• item` en linea inactiva (el guion se
        // sustituye por bullet, el espacio se mantiene). Igual para `*` y `+`.
        let theme = test_theme();
        let source = "cursor\n- uno\n* dos\n+ tres\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "• uno");
        assert_eq!(line_text(&lines, 2), "• dos");
        assert_eq!(line_text(&lines, 3), "• tres");
    }

    #[test]
    fn nivel2_no_toca_listas_ordenadas() {
        // `1. ordn` queda igual: el numero es informativo, no se reemplaza.
        let theme = test_theme();
        let source = "cursor\n1. ordn\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "1. ordn");
    }

    #[test]
    fn nivel2_preserva_bullet_en_linea_activa() {
        // En la linea activa el bullet original (`-`) se mantiene para no
        // sorprender al usuario que esta editando.
        let theme = test_theme();
        let source = "antes\n- activo\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1), "- activo");
    }

    #[test]
    fn nivel2_reemplaza_blockquote_con_barra_vertical() {
        // `> cita` -> `│ cita` en linea inactiva. La continuacion en la linea
        // siguiente (`> linea 2`) tambien se sustituye.
        let theme = test_theme();
        let source = "antes\n> cita\n> linea 2\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "│ cita");
        assert_eq!(line_text(&lines, 2), "│ linea 2");
    }

    #[test]
    fn nivel2_oculta_marcadores_de_link() {
        // `[texto](url)` queda como `texto` en linea inactiva.
        let theme = test_theme();
        let source = "cursor\nun [hola](https://x.com) link\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "un hola link");
    }

    #[test]
    fn nivel2_oculta_url_y_parentesis_de_imagen_mostrando_alt() {
        // `![alt](pic.png)` queda como `alt` en linea inactiva.
        let theme = test_theme();
        let source = "cursor\nver ![logo](pic.png) abajo\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "ver logo abajo");
    }

    #[test]
    fn nivel2_preserva_link_completo_en_linea_activa() {
        // En la linea activa el link se ve crudo (la activa renderiza Nivel 1).
        let theme = test_theme();
        let source = "antes\n[ver](https://x.com)\n";
        let lines = render(source, None, &[], None, &theme, Some(1), 2, 0);
        assert_eq!(line_text(&lines, 1), "[ver](https://x.com)");
    }

    #[test]
    fn nivel1_explicito_preserva_comportamiento_historico() {
        // Con `level = 1`, ninguna linea oculta markers, exista o no
        // `active_line`.
        let theme = test_theme();
        let source = "cursor\nplano **neg** fin\n";
        let lines = render(source, None, &[], None, &theme, Some(0), 1, 0);
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
        let lines = render(source, Some(0..3), &[], None, &theme, Some(0), 2, 0);
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
        let lines = render(source, None, &matches, Some(0), &theme, Some(0), 2, 0);
        assert_eq!(line_text(&lines, 1), "plano **neg** fin");
    }

    #[test]
    fn nivel2_oculta_aunque_active_line_sea_none() {
        // `active_line = None` significa "no hay linea exenta": en Nivel 2
        // todas las lineas ocultan delimitadores. Es el modo usado por tests
        // o por un futuro modo preview.
        let theme = test_theme();
        let source = "uno **dos** tres\n";
        let lines = render(source, None, &[], None, &theme, None, 2, 0);
        assert_eq!(line_text(&lines, 0), "uno dos tres");
    }
}
