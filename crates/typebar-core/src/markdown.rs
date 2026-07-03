//! Consultas semanticas al AST de tree-sitter-md, aisladas del modelo de
//! documento. La idea es que `document.rs` no toque tree-sitter directo: pide
//! aca lo que necesita (ej "el cursor esta dentro de una negrita?") y recibe
//! rangos en BYTES listos para editar el buffer.
//!
//! Estructura de nodos inline relevante (gramatica inline de tree-sitter-md),
//! verificada empiricamente:
//! - `strong_emphasis` (negrita `**`): los `**` NO son un solo nodo, sino DOS
//!   `emphasis_delimiter` de 1 byte cada uno. O sea la apertura `**` son los dos
//!   primeros hijos delimitadores contiguos y el cierre `**` los dos ultimos.
//! - `emphasis` (italica `*`): un `emphasis_delimiter` de apertura y otro de
//!   cierre.
//! - `code_span` (codigo `` ` ``): un `code_span_delimiter` de apertura y otro
//!   de cierre.
//!
//! Por eso `delimiters` agrupa la corrida CONTIGUA de delimitadores del inicio
//! como marcador de apertura y la corrida contigua del final como cierre, en
//! vez de quedarse con el primer/ultimo hijo suelto.

use std::ops::Range;

use tree_sitter_md::MarkdownParser;

/// Tipo de enfasis inline que se puede togglear.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineKind {
    Bold,
    Italic,
    Code,
}

impl InlineKind {
    /// `kind()` del nodo de tree-sitter que representa este enfasis.
    fn node_kind(self) -> &'static str {
        match self {
            InlineKind::Bold => "strong_emphasis",
            InlineKind::Italic => "emphasis",
            InlineKind::Code => "code_span",
        }
    }

    /// Marcador textual (lo que se inserta al togglear).
    pub fn marker(self) -> &'static str {
        match self {
            InlineKind::Bold => "**",
            InlineKind::Italic => "*",
            InlineKind::Code => "`",
        }
    }

    /// Largo del marcador en chars (`**` = 2, `*`/`` ` `` = 1).
    pub fn marker_len(self) -> usize {
        self.marker().chars().count()
    }
}

/// Rangos en BYTES de los marcadores de apertura y cierre de un nodo inline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Markers {
    pub open: Range<usize>,
    pub close: Range<usize>,
}

/// Marcador de un item de lista: vineta (`-`/`*`/`+`) u ordenado (`1.`/`2)`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListMarker {
    /// Vineta con el char usado (`-`, `*` o `+`).
    Bullet(char),
    /// Ordenado: numero actual y delimitador (`.` o `)`).
    Ordered(u64, char),
}

/// Prefijo de un item de lista Markdown al inicio de una linea: la sangria, el
/// marcador y la columna (en chars) donde arranca el contenido. Sirve para que
/// el editor continue la lista al apretar Enter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPrefix {
    /// Espacios/tabs iniciales, reproducidos tal cual en la continuacion.
    pub indent: String,
    /// El marcador del item.
    pub marker: ListMarker,
    /// Indice de char donde empieza el contenido (despues del marcador y su
    /// espacio). Si la linea no tiene mas que el marcador, el item esta vacio.
    pub content_col: usize,
}

impl ListPrefix {
    /// String del prefijo para el SIGUIENTE item: misma sangria y vineta, o el
    /// numero incrementado en listas ordenadas. Incluye el espacio final.
    pub fn continuation(&self) -> String {
        match self.marker {
            ListMarker::Bullet(c) => format!("{}{} ", self.indent, c),
            // `saturating_add` blinda el incremento: con `n == u64::MAX` no hay
            // overflow (paniquearia en debug); satura y la continuacion reusa el
            // mismo numero en vez de romper el editor.
            ListMarker::Ordered(n, delim) => {
                format!("{}{}{} ", self.indent, n.saturating_add(1), delim)
            }
        }
    }
}

/// Detecta si `line` (sin el `\n`) arranca con un item de lista Markdown y
/// devuelve su prefijo. Acepta vinetas `-`/`*`/`+` y ordenados `N.`/`N)`, en
/// ambos casos seguidos de al menos un espacio. `None` si no es un item.
pub fn list_prefix(line: &str) -> Option<ListPrefix> {
    let chars: Vec<char> = line.chars().collect();
    let mut i = 0;

    // Sangria inicial (espacios o tabs).
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }
    let indent: String = chars[..i].iter().collect();

    let marker = if matches!(chars.get(i), Some('-' | '*' | '+')) {
        let c = chars[i];
        i += 1;
        ListMarker::Bullet(c)
    } else {
        // Ordenado: una corrida de digitos seguida de '.' o ')'.
        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        if i == start || !matches!(chars.get(i), Some('.' | ')')) {
            return None;
        }
        let n: u64 = chars[start..i].iter().collect::<String>().parse().ok()?;
        let delim = chars[i];
        i += 1;
        ListMarker::Ordered(n, delim)
    };

    // Tiene que haber al menos un espacio tras el marcador para ser un item.
    if !matches!(chars.get(i), Some(' ' | '\t')) {
        return None;
    }
    while i < chars.len() && (chars[i] == ' ' || chars[i] == '\t') {
        i += 1;
    }

    Some(ListPrefix {
        indent,
        marker,
        content_col: i,
    })
}

/// Si `byte_offset` cae dentro de un nodo del tipo `kind`, devuelve los rangos
/// (en bytes) de sus marcadores de apertura y cierre. Si no, `None`.
///
/// Se elige el nodo MAS INTERNO que matchea: recorremos el arbol de corrido
/// (block + inline) y nos quedamos con el candidato de mayor `start` (el mas
/// profundo de los que contienen el offset). El criterio de contencion es
/// `start <= offset <= end` (semiabierto extendido al borde interior, para que
/// el cursor parado justo sobre el contenido tras el marcador de apertura
/// cuente como "adentro").
pub fn enclosing(text: &str, byte_offset: usize, kind: InlineKind) -> Option<Markers> {
    let mut parser = MarkdownParser::default();
    let tree = parser.parse(text.as_bytes(), None)?;
    let target = kind.node_kind();

    let mut cursor = tree.walk();
    let mut best: Option<Markers> = None;
    let mut best_start: usize = 0;

    // DFS iterativo de corrido (block + inline). En cada nodo que matchea el
    // kind y contiene el offset, extraemos los delimitadores y nos quedamos con
    // el de mayor start (mas interno).
    loop {
        let node = cursor.node();
        let range = node.byte_range();
        if node.kind() == target
            && range.start <= byte_offset
            && byte_offset <= range.end
            && let Some(markers) = delimiters(&node)
            && (best.is_none() || range.start >= best_start)
        {
            best_start = range.start;
            best = Some(markers);
        }

        if cursor.goto_first_child() {
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if !cursor.goto_parent() {
                return best;
            }
        }
    }
}

/// Rangos en bytes de los marcadores de apertura y cierre de un nodo de
/// enfasis. La apertura es la corrida CONTIGUA de delimitadores del inicio (uno
/// para `*`/`` ` ``, dos para `**`) y el cierre la corrida contigua del final.
/// Devuelve `None` si no hay al menos dos delimitadores (ej un enfasis sin
/// cerrar, que tree-sitter puede dejar incompleto).
fn delimiters(node: &tree_sitter::Node) -> Option<Markers> {
    let mut delims: Vec<Range<usize>> = Vec::new();
    let mut walk = node.walk();
    for child in node.children(&mut walk) {
        if child.kind().ends_with("_delimiter") {
            delims.push(child.byte_range());
        }
    }
    if delims.len() < 2 {
        return None;
    }

    // Apertura: desde el primer delimitador, extender mientras sean contiguos
    // (el `end` de uno es el `start` del siguiente).
    let mut open = delims[0].clone();
    let mut i = 1;
    while i < delims.len() && delims[i].start == open.end {
        open.end = delims[i].end;
        i += 1;
    }

    // Cierre: desde el ultimo delimitador, extender hacia atras mientras sean
    // contiguos.
    let last = delims.len() - 1;
    let mut close = delims[last].clone();
    let mut j = last;
    while j > 0 && delims[j - 1].end == close.start {
        close.start = delims[j - 1].start;
        j -= 1;
    }

    // Apertura y cierre no deben solaparse (caso degenerado de un solo par).
    if open.end > close.start {
        return None;
    }
    Some(Markers { open, close })
}

// --- Mapa de estilos por rango (Nivel 1) para la GUI -----------------------
//
// La TUI ya mapea el documento a estilos por byte en su renderer (ver
// `typebar-tui/src/render.rs::collect_styles`): recorre el arbol de
// tree-sitter-md y le da a cada tramo un estilo, resolviendo el solapamiento por
// PROFUNDIDAD (el nodo mas interno gana, p.ej. los `**` de una negrita pintan
// "marcador" por encima del "negrita" que los contiene). Ese es el "Nivel 1" de
// la TUI: los marcadores nunca se ocultan, solo se atenuan.
//
// `style_spans` expone esa misma logica como una API estable y agnostica de
// terminal para que la GUI pinte el bloque en edicion con el markdown CRUDO
// visible pero estilizado. No inventa categorias nuevas: mapea las que el motor
// ya distingue a un enum acotado, y aplana el resultado a tramos que NO se
// solapan (los huecos son texto plano), que es lo que el JS necesita para armar
// la secuencia de `<span>`s.

/// Categoria de estilo de un tramo del source markdown. Espeja las distinciones
/// del renderer "Nivel 1" de la TUI, expuestas para la GUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpanKind {
    /// Texto de un heading (`# ...`). El nivel lo infiere el consumidor por la
    /// cantidad de `#` del marcador (que cae en `Marker`).
    Heading,
    /// Contenido de una negrita (`**...**`); los `**` caen en `Marker`.
    Bold,
    /// Contenido de una italica (`*...*` / `_..._`); los delimitadores en `Marker`.
    Italic,
    /// Contenido de codigo inline (`` `...` ``); los backticks en `Marker`.
    Code,
    /// Bloque de codigo (fenced o indentado): todo el bloque en mono.
    CodeBlock,
    /// Marcador de sintaxis atenuado: `#`, `**`, `*`, `` ` ``, cercas ```` ``` ````,
    /// pipes de tabla, corchetes/parentesis/`!` de links e imagenes, etc.
    Marker,
    /// Marcador de item de lista (`-`, `*`, `+`, `1.`, `1)`).
    ListMarker,
    /// Marcador `>` de blockquote.
    Blockquote,
    /// Texto visible de un link o alt de una imagen (`[esto](...)`).
    LinkText,
    /// Destino o titulo de un link (`[...](esto)`).
    LinkUrl,
}

impl SpanKind {
    /// Nombre estable de la categoria; la GUI lo usa como clase CSS `md-<kind>`.
    /// Estable: el frontend depende de estos strings.
    pub fn as_str(self) -> &'static str {
        match self {
            SpanKind::Heading => "heading",
            SpanKind::Bold => "bold",
            SpanKind::Italic => "italic",
            SpanKind::Code => "code",
            SpanKind::CodeBlock => "code_block",
            SpanKind::Marker => "marker",
            SpanKind::ListMarker => "list_marker",
            SpanKind::Blockquote => "blockquote",
            SpanKind::LinkText => "link_text",
            SpanKind::LinkUrl => "link_url",
        }
    }
}

/// Un tramo estilizado del source, en offsets UTF-16 (las unidades del `String`
/// de JS), listo para el consumidor JS de la GUI. Los tramos NO se solapan y van
/// ordenados por `start`; los huecos entre tramos son texto plano.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleSpan {
    /// Inicio del tramo, en unidades UTF-16.
    pub start: usize,
    /// Fin exclusivo del tramo, en unidades UTF-16.
    pub end: usize,
    /// Categoria de estilo.
    pub kind: SpanKind,
}

/// Tramo estilizado en BYTES con la profundidad del nodo, antes de resolver el
/// solapamiento. Interno: `style_spans` lo aplana.
struct StyleRange {
    start: usize,
    end: usize,
    kind: SpanKind,
    depth: usize,
}

/// Mapea un `kind`/`parent` de nodo de tree-sitter-md a nuestra categoria de
/// estilo. Es la traduccion de las mismas reglas del renderer de la TUI (ver
/// `collect_styles`) a nuestro enum acotado. Las ramas van de la mas especifica
/// a la mas generica: los `_marker`/`_delimiter` genericos quedan al final.
fn classify(kind: &str, parent: Option<&str>) -> Option<SpanKind> {
    match kind {
        // El texto de un heading es el nodo `inline` hijo del heading (atx o
        // setext). El marcador `#`/`##` se atenua aparte (rama generica).
        "inline" if matches!(parent, Some("atx_heading" | "setext_heading")) => {
            Some(SpanKind::Heading)
        }
        "strong_emphasis" => Some(SpanKind::Bold),
        "emphasis" => Some(SpanKind::Italic),
        "code_span" => Some(SpanKind::Code),
        "fenced_code_block" | "indented_code_block" => Some(SpanKind::CodeBlock),
        // La cerca ``` de apertura/cierre es un marcador (dentro de la caja mono).
        "fenced_code_block_delimiter" => Some(SpanKind::Marker),
        "block_quote_marker" => Some(SpanKind::Blockquote),
        "link_text" | "image_description" => Some(SpanKind::LinkText),
        "link_destination" | "link_title" => Some(SpanKind::LinkUrl),
        // Tablas: la fila de cabecera en negrita, la fila delimitadora y los
        // pipes `|` como estructura atenuada (mismas distinciones que la TUI).
        "pipe_table_cell" if parent == Some("pipe_table_header") => Some(SpanKind::Bold),
        "pipe_table_delimiter_row" => Some(SpanKind::Marker),
        "|" if matches!(
            parent,
            Some("pipe_table_header" | "pipe_table_row" | "pipe_table_delimiter_row")
        ) =>
        {
            Some(SpanKind::Marker)
        }
        // Marcadores de item de lista (bullet u ordenado).
        k if k.starts_with("list_marker") => Some(SpanKind::ListMarker),
        // Todo lo que rodea al texto visible de un link o imagen (`[`, `]`, `(`,
        // `)`, `!`) es marcador; el texto/destino ya se resolvio arriba.
        _ if matches!(parent, Some("inline_link" | "image")) => Some(SpanKind::Marker),
        // Cualquier otro marcador o delimitador: atenuado.
        k if k.ends_with("_marker") || k.ends_with("_delimiter") => Some(SpanKind::Marker),
        _ => None,
    }
}

/// DFS iterativo del arbol de tree-sitter-md que junta los tramos estilizados en
/// BYTES con su profundidad. Mismo recorrido block+inline que usa la TUI.
fn collect_style_ranges(source: &str) -> Vec<StyleRange> {
    let mut ranges: Vec<StyleRange> = Vec::new();
    if source.is_empty() {
        return ranges;
    }
    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(source.as_bytes(), None) else {
        return ranges;
    };
    let mut cursor = tree.walk();
    // `stack` guarda los kinds de los ancestros; su largo es la profundidad.
    let mut stack: Vec<&str> = Vec::new();

    'dfs: loop {
        let node = cursor.node();
        let kind = node.kind();
        let range = node.byte_range();
        let depth = stack.len();
        let parent = stack.last().copied();

        if let Some(k) = classify(kind, parent) {
            ranges.push(StyleRange {
                start: range.start,
                end: range.end,
                kind: k,
                depth,
            });
        }

        if cursor.goto_first_child() {
            stack.push(kind);
            continue;
        }
        loop {
            if cursor.goto_next_sibling() {
                break;
            }
            if stack.pop().is_none() {
                break 'dfs; // volvimos a la raiz: fin del DFS.
            }
            cursor.goto_parent();
        }
    }
    ranges
}

/// Mapa byte -> offset UTF-16. Cada byte apunta al offset UTF-16 del char al que
/// pertenece (su inicio); el ultimo slot (len) es el total. Como los limites de
/// los tramos de tree-sitter caen siempre en frontera de char, la conversion es
/// exacta. tree-sitter trabaja en BYTES de Rust y el JS en unidades UTF-16, asi
/// que sin esta conversion los offsets se corren con acentos, emoji o CJK.
fn byte_to_utf16_map(source: &str) -> Vec<usize> {
    let mut map = vec![0usize; source.len() + 1];
    let mut u16_off = 0usize;
    for (b, ch) in source.char_indices() {
        for slot in &mut map[b..b + ch.len_utf8()] {
            *slot = u16_off;
        }
        u16_off += ch.len_utf16();
    }
    map[source.len()] = u16_off;
    map
}

/// Devuelve los tramos de estilo del `source` markdown, en offsets UTF-16, para
/// que la GUI pinte el "Nivel 1" del bloque en edicion: el markdown crudo
/// VISIBLE pero estilizado. Los tramos no se solapan y van ordenados por `start`;
/// los huecos son texto plano.
///
/// Resuelve el solapamiento por profundidad (el nodo mas interno gana, igual que
/// el pintado por `depth` de la TUI) pintando byte a byte, y despues coalesce las
/// corridas contiguas del mismo kind en un solo tramo.
pub fn style_spans(source: &str) -> Vec<StyleSpan> {
    // tree-sitter-md necesita el newline final para reconocer un bloque: sin el,
    // "# T" o "> cita" a medio tipear parsean como ERROR y no se estilarian. Le
    // agregamos un '\n' sintetico para parsear y despues recortamos los tramos al
    // largo original (los offsets internos no cambian porque solo se apendea).
    let len = source.len();
    let owned;
    let parse_src: &str = if source.is_empty() || source.ends_with('\n') {
        source
    } else {
        owned = format!("{source}\n");
        &owned
    };

    // 1. Junto los tramos y los ordeno por profundidad ascendente, para pintar el
    //    mas profundo (mas especifico) al final y que gane.
    let mut ranges = collect_style_ranges(parse_src);
    ranges.sort_by_key(|r| r.depth);

    let mut byte_kind: Vec<Option<SpanKind>> = vec![None; parse_src.len()];
    for r in &ranges {
        for slot in &mut byte_kind[r.start..r.end] {
            *slot = Some(r.kind);
        }
    }

    // 2. Coalesce corridas contiguas del mismo kind en tramos (todavia en bytes),
    //    recortando al largo original (descarta lo que caiga en el '\n' sintetico).
    let mut byte_spans: Vec<(usize, usize, SpanKind)> = Vec::new();
    let mut i = 0;
    while i < byte_kind.len() {
        if let Some(kind) = byte_kind[i] {
            let start = i;
            i += 1;
            while i < byte_kind.len() && byte_kind[i] == Some(kind) {
                i += 1;
            }
            if start < len {
                byte_spans.push((start, i.min(len), kind));
            }
        } else {
            i += 1;
        }
    }

    // 3. Convierto los offsets de bytes a UTF-16 (lo que consume el JS).
    let map = byte_to_utf16_map(source);
    byte_spans
        .into_iter()
        .map(|(start, end, kind)| StyleSpan {
            start: map[start],
            end: map[end],
            kind,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detecta_negrita() {
        // "**negro**": '*' en 0,1; "negro" en 2..7; '*' en 7,8.
        let t = "**negro**";
        let m = enclosing(t, 4, InlineKind::Bold).expect("deberia matchear negrita");
        assert_eq!(m.open, 0..2);
        assert_eq!(m.close, 7..9);
    }

    #[test]
    fn detecta_italica() {
        let t = "*x*";
        let m = enclosing(t, 1, InlineKind::Italic).expect("deberia matchear italica");
        assert_eq!(m.open, 0..1);
        assert_eq!(m.close, 2..3);
    }

    #[test]
    fn detecta_codigo() {
        let t = "`cod`";
        let m = enclosing(t, 2, InlineKind::Code).expect("deberia matchear codigo");
        assert_eq!(m.open, 0..1);
        assert_eq!(m.close, 4..5);
    }

    #[test]
    fn fuera_de_rango_es_none() {
        // "ab **x** cd": el offset 0 (sobre 'a') no esta en ninguna negrita.
        let t = "ab **x** cd";
        assert_eq!(enclosing(t, 0, InlineKind::Bold), None);
    }

    #[test]
    fn bold_no_matchea_italica() {
        // "*x*" es `emphasis`, no `strong_emphasis`: Bold no debe matchear.
        let t = "*x*";
        assert_eq!(enclosing(t, 1, InlineKind::Bold), None);
    }

    #[test]
    fn italica_no_matchea_negrita() {
        // "**x**" es `strong_emphasis`: Italic no debe matchear.
        let t = "**x**";
        assert_eq!(enclosing(t, 2, InlineKind::Italic), None);
    }

    #[test]
    fn offset_absoluto_en_linea_tardia() {
        // Los offsets inline son absolutos al documento: una negrita en una
        // linea posterior se detecta con offsets globales.
        let t = "# T\n\nplano **negro** fin\n";
        let pos = t.find("negro").unwrap() + 1;
        let m = enclosing(t, pos, InlineKind::Bold).expect("deberia matchear");
        let open_start = t.find("**").unwrap();
        assert_eq!(m.open, open_start..open_start + 2);
    }

    // --- Prefijos de lista -------------------------------------------------

    #[test]
    fn list_prefix_vineta() {
        let p = list_prefix("- hola").expect("deberia ser item");
        assert_eq!(p.indent, "");
        assert_eq!(p.marker, ListMarker::Bullet('-'));
        assert_eq!(p.content_col, 2);
        assert_eq!(p.continuation(), "- ");
    }

    #[test]
    fn list_prefix_vineta_con_sangria_y_asterisco() {
        let p = list_prefix("    * item").expect("deberia ser item");
        assert_eq!(p.indent, "    ");
        assert_eq!(p.marker, ListMarker::Bullet('*'));
        assert_eq!(p.continuation(), "    * ");
    }

    #[test]
    fn list_prefix_ordenado_incrementa() {
        let p = list_prefix("3. tercero").expect("deberia ser item");
        assert_eq!(p.marker, ListMarker::Ordered(3, '.'));
        assert_eq!(p.content_col, 3);
        assert_eq!(p.continuation(), "4. ");
    }

    #[test]
    fn list_prefix_ordenado_con_paren() {
        let p = list_prefix("10) diez").expect("deberia ser item");
        assert_eq!(p.marker, ListMarker::Ordered(10, ')'));
        assert_eq!(p.continuation(), "11) ");
    }

    #[test]
    fn continuation_ordenada_en_u64_max_no_paniquea() {
        // Con el numero maximo, `saturating_add` evita el overflow (que en debug
        // paniquearia): satura en u64::MAX y devuelve algo razonable.
        let p = ListPrefix {
            indent: String::new(),
            marker: ListMarker::Ordered(u64::MAX, '.'),
            content_col: 0,
        };
        assert_eq!(p.continuation(), format!("{}. ", u64::MAX));
    }

    #[test]
    fn list_prefix_item_vacio() {
        // Solo el marcador y un espacio: content_col cae al final (item vacio).
        let p = list_prefix("- ").expect("deberia ser item");
        assert_eq!(p.content_col, 2);
        assert_eq!("- ".chars().count(), p.content_col);
    }

    #[test]
    fn list_prefix_rechaza_no_items() {
        // Sin espacio tras el marcador no es item.
        assert_eq!(list_prefix("-sin-espacio"), None);
        assert_eq!(list_prefix("1.sin"), None);
        // Texto plano.
        assert_eq!(list_prefix("hola mundo"), None);
        // Solo el guion sin espacio (posible thematic break, no item).
        assert_eq!(list_prefix("-"), None);
    }

    // --- Spans de estilo (Nivel 1 de la GUI) -------------------------------

    /// Helper: primer tramo del kind dado.
    fn first(spans: &[StyleSpan], kind: SpanKind) -> Option<&StyleSpan> {
        spans.iter().find(|s| s.kind == kind)
    }

    #[test]
    fn style_spans_heading_marca_marcador_y_texto() {
        // "# Hola": '#' es marcador (0..1) y "Hola" es heading (2..6).
        let spans = style_spans("# Hola");
        let marker = first(&spans, SpanKind::Marker).expect("marcador '#'");
        assert_eq!((marker.start, marker.end), (0, 1));
        let h = first(&spans, SpanKind::Heading).expect("texto heading");
        assert_eq!((h.start, h.end), (2, 6));
    }

    #[test]
    fn style_spans_negrita_separa_marcadores_del_contenido() {
        // "**hola**": '**' marcador en 0..2 y 6..8, "hola" negrita en 2..6.
        let spans = style_spans("**hola**");
        let bold = first(&spans, SpanKind::Bold).expect("negrita");
        assert_eq!((bold.start, bold.end), (2, 6));
        let markers: Vec<_> = spans
            .iter()
            .filter(|s| s.kind == SpanKind::Marker)
            .collect();
        assert_eq!(markers.len(), 2, "spans: {spans:?}");
        assert_eq!((markers[0].start, markers[0].end), (0, 2));
        assert_eq!((markers[1].start, markers[1].end), (6, 8));
    }

    #[test]
    fn style_spans_codigo_inline_separa_backticks() {
        // "`x`": backticks marcador, 'x' codigo en 1..2.
        let spans = style_spans("`x`");
        let code = first(&spans, SpanKind::Code).expect("codigo");
        assert_eq!((code.start, code.end), (1, 2));
        // Los dos backticks quedan como marcador.
        assert_eq!(
            spans.iter().filter(|s| s.kind == SpanKind::Marker).count(),
            2
        );
    }

    #[test]
    fn style_spans_lista_y_blockquote() {
        let lista = style_spans("- item");
        assert!(
            first(&lista, SpanKind::ListMarker).is_some(),
            "spans: {lista:?}"
        );
        let cita = style_spans("> cita");
        assert!(
            first(&cita, SpanKind::Blockquote).is_some(),
            "spans: {cita:?}"
        );
    }

    #[test]
    fn style_spans_texto_plano_no_tiene_tramos() {
        assert!(style_spans("hola mundo sin formato").is_empty());
        assert!(style_spans("").is_empty());
    }

    #[test]
    fn style_spans_tramos_no_se_solapan_y_van_ordenados() {
        // Invariante para el consumidor JS: tramos ordenados y disjuntos.
        let spans = style_spans("# T con **negro** y `cod`\n");
        let mut prev_end = 0;
        for s in &spans {
            assert!(s.start >= prev_end, "solapan/desordenados: {spans:?}");
            assert!(s.end > s.start);
            prev_end = s.end;
        }
    }

    // --- Conversion de offsets a UTF-16 con multibyte -----------------------

    #[test]
    fn style_spans_offsets_utf16_con_acento() {
        // 'é' ocupa 2 bytes UTF-8 pero 1 unidad UTF-16: el contenido cae en 2..3.
        let spans = style_spans("**é**");
        let bold = first(&spans, SpanKind::Bold).expect("negrita");
        assert_eq!((bold.start, bold.end), (2, 3));
    }

    #[test]
    fn style_spans_offsets_utf16_con_emoji() {
        // '😀' es 4 bytes UTF-8 y 2 unidades UTF-16 (par surrogate): negrita en 2..4
        // y el marcador de cierre arranca despues del par, en 4..6.
        let spans = style_spans("**😀**");
        let bold = first(&spans, SpanKind::Bold).expect("negrita");
        assert_eq!((bold.start, bold.end), (2, 4));
        let close = spans
            .iter()
            .rfind(|s| s.kind == SpanKind::Marker)
            .expect("marcador de cierre");
        assert_eq!((close.start, close.end), (4, 6));
    }

    #[test]
    fn style_spans_offsets_utf16_con_cjk() {
        // Cada CJK ocupa 3 bytes UTF-8 y 1 unidad UTF-16: "中文" cae en 2..4.
        let spans = style_spans("# 中文");
        let h = first(&spans, SpanKind::Heading).expect("heading");
        assert_eq!((h.start, h.end), (2, 4));
    }
}
