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

// --- Rangos de elementos (Nivel 2 del bloque activo) -----------------------
//
// El "Nivel 2" de la GUI oculta los marcadores del bloque en edicion, salvo los
// del elemento donde esta el caret, que se revelan para editarlos (la sensacion
// Typora). Es el mismo salto que el "Nivel 2" de la TUI, pero en vez de linea
// activa cruda es el ELEMENTO activo el que muestra su sintaxis.
//
// Para decidir "que marcador pertenece a que elemento" el JS necesita, ademas de
// los spans planos, el rango COMPLETO de cada elemento (con sus marcadores
// adentro). tree-sitter ya da esos nodos: el rango del nodo `strong_emphasis`
// entero es exactamente una negrita con sus `**`, el de `atx_heading` la linea
// del heading con su `#`, etc. `style_elements` los expone en offsets UTF-16;
// el JS calcula por interseccion de rangos que revelar, sin logica de markdown.

/// Tipo de elemento revelable. Los inline (`Bold`/`Italic`/`Code`/`Link`) tienen
/// como rango el nodo entero (marcadores incluidos); los de linea/bloque
/// (`Heading`/`ListItem`/`Blockquote`) el rango del nodo de tree-sitter, que para
/// un heading atx es su linea y para un item de lista puede abarcar varias.
/// La `Blockquote` es la excepcion: como repite su `>` en cada linea, se emite
/// un elemento POR LINEA (ver `style_elements`) y nunca abarca mas de una.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
    Bold,
    Italic,
    Code,
    Link,
    Heading,
    ListItem,
    Blockquote,
}

impl ElementKind {
    /// Nombre estable de la categoria (el frontend lo recibe como metadato).
    pub fn as_str(self) -> &'static str {
        match self {
            ElementKind::Bold => "bold",
            ElementKind::Italic => "italic",
            ElementKind::Code => "code",
            ElementKind::Link => "link",
            ElementKind::Heading => "heading",
            ElementKind::ListItem => "list_item",
            ElementKind::Blockquote => "blockquote",
        }
    }
}

/// Rango COMPLETO de un elemento revelable, en offsets UTF-16, incluyendo sus
/// marcadores. Los rangos PUEDEN anidarse (una negrita dentro de un heading da
/// dos elementos que se contienen); el consumidor resuelve la profundidad.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleElement {
    /// Inicio del elemento, en unidades UTF-16.
    pub start: usize,
    /// Fin (inclusive en la logica del consumidor) del elemento, en UTF-16.
    pub end: usize,
    /// Tipo de elemento.
    pub kind: ElementKind,
}

/// Respuesta unificada para el frontend: los tramos de estilo (Nivel 1) y los
/// rangos de elementos (Nivel 2), en UNA sola pasada para el consumidor. El
/// comando IPC devuelve ambas listas juntas para evitar dos round-trips.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StyleInfo {
    pub spans: Vec<StyleSpan>,
    pub elements: Vec<StyleElement>,
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
fn classify(kind: &str, parent: Option<&str>, grandparent: Option<&str>) -> Option<SpanKind> {
    match kind {
        // El texto de un heading es el nodo `inline` hijo del heading. En un atx
        // cuelga directo del `atx_heading`; en un setext, tree-sitter interpone
        // un `paragraph` (setext_heading > paragraph > inline), asi que ese caso
        // se reconoce por el ABUELO. Sin esta segunda rama el titulo de un setext
        // no recibia ningun tramo y quedaba como texto plano.
        "inline" if parent == Some("atx_heading") => Some(SpanKind::Heading),
        "inline" if parent == Some("paragraph") && grandparent == Some("setext_heading") => {
            Some(SpanKind::Heading)
        }
        // El subrayado de un setext (`===` / `---`) es SU marcador. Va explicito
        // porque el nodo no termina en `_marker` ni `_delimiter` y por lo tanto
        // no lo agarra la rama generica de abajo.
        "setext_h1_underline" | "setext_h2_underline" => Some(SpanKind::Marker),
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

/// Rangos en BYTES de los marcadores `>` de las lineas de CONTINUACION de la
/// cita que ocupa `[start, end)`. Incluye el espacio que sigue al `>`, igual que
/// el `block_quote_marker` que emite tree-sitter para la primera linea ("> ").
///
/// Hace falta porque tree-sitter-md solo marca el `>` de la PRIMERA linea: las
/// siguientes continuan el mismo parrafo y sus `>` quedan adentro del nodo
/// `inline`, sin nodo propio. Sin esto no serian marcadores, o sea que no se
/// atenuarian en el Nivel 1 ni se podrian ocultar en el Nivel 2: la cita
/// multilinea mostraba el primer `>` tenue y los demas como texto comun.
fn continuation_quote_markers(source: &str, start: usize, end: usize) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    for (i, b) in bytes.iter().enumerate().take(end).skip(start) {
        if *b != b'\n' {
            continue;
        }
        // Arranque de la linea siguiente, con hasta 3 espacios de sangria (lo
        // que CommonMark tolera antes de un marcador de bloque).
        let mut j = i + 1;
        let limite = (j + 3).min(end);
        while j < limite && bytes[j] == b' ' {
            j += 1;
        }
        if j < end && bytes[j] == b'>' {
            let mut fin = j + 1;
            if fin < end && bytes[fin] == b' ' {
                fin += 1;
            }
            out.push((j, fin));
        }
    }
    out
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

        let grandparent = stack.iter().rev().nth(1).copied();
        if let Some(k) = classify(kind, parent, grandparent) {
            // El subrayado de un setext se lleva puesto el `\n` que lo separa del
            // titulo: es parte del marcador (sin el salto de linea no hay
            // subrayado) y asi, cuando el Nivel 2 lo oculta, no queda una linea
            // vacia colgando bajo el titulo (el texto no salta verticalmente al
            // entrar y salir del bloque).
            let mut start = range.start;
            if matches!(kind, "setext_h1_underline" | "setext_h2_underline")
                && start > 0
                && source.as_bytes()[start - 1] == b'\n'
            {
                start -= 1;
            }
            ranges.push(StyleRange {
                start,
                end: range.end,
                kind: k,
                depth,
            });
        }

        // Los `>` de las lineas de continuacion de una cita no tienen nodo
        // propio; los agregamos nosotros (ver `continuation_quote_markers`).
        // Pisar un rango ya pintado no molesta: el aplanado de `style_spans` es
        // por byte, asi que repetir el mismo kind sobre el mismo byte es inocuo.
        if kind == "block_quote" {
            for (ini, fin) in continuation_quote_markers(source, range.start, range.end) {
                ranges.push(StyleRange {
                    start: ini,
                    end: fin,
                    kind: SpanKind::Blockquote,
                    depth: depth + 1,
                });
            }
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

/// Mapea el `kind()` de un nodo de tree-sitter-md a un `ElementKind` revelable, o
/// `None` si el nodo no es un elemento que oculte/revele marcadores.
fn element_kind(kind: &str) -> Option<ElementKind> {
    match kind {
        "strong_emphasis" => Some(ElementKind::Bold),
        "emphasis" => Some(ElementKind::Italic),
        "code_span" => Some(ElementKind::Code),
        "inline_link" | "image" => Some(ElementKind::Link),
        "atx_heading" | "setext_heading" => Some(ElementKind::Heading),
        "list_item" => Some(ElementKind::ListItem),
        "block_quote" => Some(ElementKind::Blockquote),
        _ => None,
    }
}

/// Parte el rango de bytes `[start, end)` en un sub-rango POR LINEA y los apila
/// en `out` con el mismo `kind`. Los `\n` separadores quedan afuera y las lineas
/// vacias se descartan (un rango vacio no es un elemento).
///
/// Lo usa `style_elements` con las citas: a diferencia del resto de los nodos de
/// bloque, una cita repite su marcador en cada linea, y el consumidor necesita
/// un elemento por linea para revelar solo el marcador de la linea del caret.
fn push_line_elements(
    out: &mut Vec<(usize, usize, ElementKind)>,
    bytes: &[u8],
    start: usize,
    end: usize,
    kind: ElementKind,
) {
    let mut line_start = start;
    for (i, b) in bytes.iter().enumerate().take(end).skip(start) {
        if *b == b'\n' {
            if line_start < i {
                out.push((line_start, i, kind));
            }
            line_start = i + 1;
        }
    }
    if line_start < end {
        out.push((line_start, end, kind));
    }
}

/// Devuelve los rangos COMPLETOS de los elementos revelables del `source`, en
/// offsets UTF-16, para el "Nivel 2" del bloque en edicion. Cada rango abarca el
/// nodo entero (marcadores incluidos): una negrita con sus `**`, un heading con
/// su `#`, un item de lista con su vineta, etc. Los rangos pueden anidarse; el
/// consumidor resuelve la profundidad (el elemento mas interno es el "dueno" de
/// un marcador).
///
/// A los nodos de linea/bloque (heading, item, cita) se les recorta el `\n` final
/// para que el rango NO invada el arranque del bloque siguiente.
///
/// Las citas multilinea se parten en un elemento POR LINEA: su marcador `>` se
/// repite en cada linea, asi que cada linea es su propia unidad de revelado (con
/// el nodo entero, el caret en una linea revelaba los `>` de TODA la cita).
pub fn style_elements(source: &str) -> Vec<StyleElement> {
    // Mismo `\n` sintetico que `style_spans`: sin el, un bloque a medio tipear
    // ("# T", "> cita") parsea como ERROR y no daria elemento. Recortamos al
    // largo original despues (los offsets internos no se corren, solo se apendea).
    let len = source.len();
    let owned;
    let parse_src: &str = if source.is_empty() || source.ends_with('\n') {
        source
    } else {
        owned = format!("{source}\n");
        &owned
    };

    let mut parser = MarkdownParser::default();
    let Some(tree) = parser.parse(parse_src.as_bytes(), None) else {
        return Vec::new();
    };
    let bytes = parse_src.as_bytes();

    // DFS iterativo de corrido (block + inline), como `enclosing`. Junto los
    // rangos en BYTES y los convierto a UTF-16 al final.
    let mut cursor = tree.walk();
    let mut byte_elems: Vec<(usize, usize, ElementKind)> = Vec::new();
    loop {
        let node = cursor.node();
        if let Some(kind) = element_kind(node.kind()) {
            let range = node.byte_range();
            let start = range.start.min(len);
            let mut end = range.end.min(len);
            // Recortar el/los newline(s) finales del nodo (nodos de bloque los
            // incluyen) para no pisar el arranque del bloque siguiente.
            while end > start && bytes[end - 1] == b'\n' {
                end -= 1;
            }
            if start < end {
                // Una cita multilinea repite su marcador `>` en CADA linea, asi
                // que cada linea se edita por su cuenta: la partimos en un
                // elemento POR LINEA para que el caret revele solo el `>` de su
                // linea y no los de la cita entera.
                //
                // El item de lista NO se parte: tiene un unico marcador al
                // principio y sus lineas de continuacion pertenecen al mismo
                // elemento, asi que el caret en la continuacion sigue "dentro"
                // del item y su vineta se revela, que es lo que se espera. Una
                // lista de varios items ya da un elemento por item.
                if kind == ElementKind::Blockquote {
                    push_line_elements(&mut byte_elems, bytes, start, end, kind);
                } else {
                    byte_elems.push((start, end, kind));
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
                // Volvimos a la raiz: convierto a UTF-16 y devuelvo.
                let map = byte_to_utf16_map(source);
                return byte_elems
                    .into_iter()
                    .map(|(start, end, kind)| StyleElement {
                        start: map[start],
                        end: map[end],
                        kind,
                    })
                    .collect();
            }
        }
    }
}

/// Devuelve en una sola pasada los tramos de estilo (Nivel 1) y los rangos de
/// elementos (Nivel 2) del `source`. Es lo que consume el comando IPC de la GUI:
/// una llamada, dos listas, sin dos round-trips.
pub fn style_info(source: &str) -> StyleInfo {
    StyleInfo {
        spans: style_spans(source),
        elements: style_elements(source),
    }
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

    // --- Rangos de elementos (Nivel 2 de la GUI) ---------------------------

    /// Helper: primer elemento del kind dado.
    fn first_el(els: &[StyleElement], kind: ElementKind) -> Option<&StyleElement> {
        els.iter().find(|e| e.kind == kind)
    }

    #[test]
    fn elements_negrita_cubre_los_marcadores() {
        // "**hola**": el elemento Bold abarca los `**` y el contenido: 0..8.
        let els = style_elements("**hola**");
        let bold = first_el(&els, ElementKind::Bold).expect("bold");
        assert_eq!((bold.start, bold.end), (0, 8));
    }

    #[test]
    fn elements_bold_anidado_en_heading() {
        // "## a **b** c": el heading cubre toda la linea (con su `##`) y la
        // negrita, anidada, cubre solo `**b**`. Se contienen.
        let src = "## a **b** c";
        let els = style_elements(src);
        let heading = first_el(&els, ElementKind::Heading).expect("heading");
        assert_eq!((heading.start, heading.end), (0, src.chars().count()));
        let bold = first_el(&els, ElementKind::Bold).expect("bold");
        assert_eq!((bold.start, bold.end), (5, 10));
        // Anidamiento: el heading contiene a la negrita.
        assert!(heading.start <= bold.start && bold.end <= heading.end);
    }

    #[test]
    fn elements_link_cubre_corchetes_y_destino() {
        // "[txt](url)": el elemento Link abarca todo, corchetes y parentesis: 0..10.
        let els = style_elements("[txt](url)");
        let link = first_el(&els, ElementKind::Link).expect("link");
        assert_eq!((link.start, link.end), (0, 10));
    }

    #[test]
    fn elements_item_de_lista_multilinea() {
        // "- one\n  two\n": un item con una continuacion indentada. El elemento
        // ListItem abarca AMBAS lineas (sin el `\n` final): arranca en 0 y llega
        // mas alla del primer salto de linea (offset 5).
        let src = "- one\n  two\n";
        let els = style_elements(src);
        let item = first_el(&els, ElementKind::ListItem).expect("list item");
        assert_eq!(item.start, 0);
        assert!(
            item.end > 6,
            "el item deberia abarcar la segunda linea; els: {els:?}"
        );
        // No incluye el `\n` final.
        assert!(item.end < src.chars().count());
    }

    #[test]
    fn elements_blockquote_y_code_span() {
        let cita = style_elements("> hola");
        assert!(
            first_el(&cita, ElementKind::Blockquote).is_some(),
            "els: {cita:?}"
        );
        let code = style_elements("`x`");
        let c = first_el(&code, ElementKind::Code).expect("code");
        assert_eq!((c.start, c.end), (0, 3));
    }

    #[test]
    fn elements_cita_multilinea_da_un_elemento_por_linea() {
        // "> a\n> b\n> c": la cita se parte en UNA unidad de revelado por linea,
        // para que el caret en una linea no revele los `>` de las otras.
        let src = "> a\n> b\n> c";
        let els = style_elements(src);
        let citas: Vec<_> = els
            .iter()
            .filter(|e| e.kind == ElementKind::Blockquote)
            .map(|e| (e.start, e.end))
            .collect();
        assert_eq!(citas, vec![(0, 3), (4, 7), (8, 11)], "els: {els:?}");
    }

    #[test]
    fn elements_cita_de_una_linea_no_se_parte() {
        // Sin `\n` adentro no hay nada que partir: sigue siendo un solo elemento
        // que cubre la linea entera, marcador incluido.
        let els = style_elements("> hola");
        let citas: Vec<_> = els
            .iter()
            .filter(|e| e.kind == ElementKind::Blockquote)
            .map(|e| (e.start, e.end))
            .collect();
        assert_eq!(citas, vec![(0, 6)], "els: {els:?}");
    }

    #[test]
    fn elements_cita_multilinea_no_cubre_los_marcadores_de_otras_lineas() {
        // La propiedad que le importa al consumidor: el marcador `>` de la
        // segunda linea NO cae dentro del elemento de la primera, asi que el
        // caret en la primera ya no lo revela.
        let src = "> uno\n> dos";
        let els = style_elements(src);
        let primera = els
            .iter()
            .find(|e| e.kind == ElementKind::Blockquote)
            .expect("cita");
        let marcador_segunda_linea = 6; // el `>` de "> dos"
        assert!(
            marcador_segunda_linea >= primera.end,
            "el elemento de la primera linea no deberia llegar al `>` de la \
             segunda; primera: {primera:?}"
        );
    }

    #[test]
    fn elements_cita_con_lazy_continuation() {
        // "> uno\ndos": la segunda linea pertenece a la cita pero no tiene `>`.
        // Igual da su propio elemento (no hay marcador que revelar, pero el
        // rango por linea se mantiene consistente).
        let src = "> uno\ndos";
        let els = style_elements(src);
        let citas: Vec<_> = els
            .iter()
            .filter(|e| e.kind == ElementKind::Blockquote)
            .map(|e| (e.start, e.end))
            .collect();
        assert_eq!(citas, vec![(0, 5), (6, 9)], "els: {els:?}");
    }

    #[test]
    fn style_spans_cita_multilinea_marca_todos_los_mayor_que() {
        // tree-sitter solo da nodo al `>` de la primera linea; los de las lineas
        // de continuacion los agregamos nosotros. Sin esto, el segundo y el
        // tercer `>` quedaban como texto comun (ni tenues ni ocultables).
        let src = "> uno\n> dos\n> tres\n";
        let spans = style_spans(src);
        let citas: Vec<_> = spans
            .iter()
            .filter(|s| s.kind == SpanKind::Blockquote)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(citas, vec![(0, 2), (6, 8), (12, 14)], "spans: {spans:?}");
        for (ini, fin) in citas {
            assert_eq!(&src[ini..fin], "> ");
        }
    }

    #[test]
    fn style_spans_cita_con_sangria_y_sin_espacio() {
        // `>` pegado al texto (sin espacio) y con sangria de hasta 3 espacios.
        let src = "> uno\n  >dos\n";
        let spans = style_spans(src);
        let citas: Vec<_> = spans
            .iter()
            .filter(|s| s.kind == SpanKind::Blockquote)
            .map(|s| (s.start, s.end))
            .collect();
        assert_eq!(citas, vec![(0, 2), (8, 9)], "spans: {spans:?}");
    }

    #[test]
    fn style_spans_cita_con_lazy_continuation_no_inventa_marcador() {
        // Segunda linea sin `>`: no hay marcador que marcar.
        let spans = style_spans("> uno\ndos\n");
        let citas = spans
            .iter()
            .filter(|s| s.kind == SpanKind::Blockquote)
            .count();
        assert_eq!(citas, 1, "spans: {spans:?}");
    }

    #[test]
    fn style_spans_mayor_que_fuera_de_cita_no_es_marcador() {
        // Un `>` en medio de un parrafo comun no se toca.
        let spans = style_spans("a > b\n");
        assert!(
            !spans.iter().any(|s| s.kind == SpanKind::Blockquote),
            "spans: {spans:?}"
        );
    }

    // --- Headings setext (`Titulo\n======`) -------------------------------

    #[test]
    fn style_spans_setext_marca_titulo_y_subrayado() {
        // El titulo de un setext es un tramo heading (antes quedaba sin tramo:
        // su `inline` cuelga de un `paragraph`, no del `setext_heading`), y el
        // subrayado es su marcador.
        let src = "Titulo\n======\n";
        let spans = style_spans(src);
        let heading = first(&spans, SpanKind::Heading).expect("tramo heading");
        assert_eq!((heading.start, heading.end), (0, 6), "spans: {spans:?}");
        let marker = first(&spans, SpanKind::Marker).expect("tramo marcador");
        // El marcador arranca en el `\n` (6), no en el primer `=` (7).
        assert_eq!((marker.start, marker.end), (6, 13), "spans: {spans:?}");
    }

    #[test]
    fn style_spans_setext_h2_tambien() {
        let spans = style_spans("Sub\n---\n");
        assert!(
            first(&spans, SpanKind::Heading).is_some(),
            "spans: {spans:?}"
        );
        assert!(
            first(&spans, SpanKind::Marker).is_some(),
            "spans: {spans:?}"
        );
    }

    #[test]
    fn style_spans_parrafo_normal_no_es_heading() {
        // Guarda de la rama nueva: un `inline` dentro de un `paragraph` que NO
        // cuelga de un setext_heading sigue siendo texto plano.
        let spans = style_spans("un parrafo comun\n");
        assert!(
            first(&spans, SpanKind::Heading).is_none(),
            "spans: {spans:?}"
        );
    }

    #[test]
    fn elements_setext_cubre_titulo_y_subrayado() {
        // El elemento heading abarca las dos lineas, asi el caret en el titulo
        // revela el subrayado (y viceversa).
        let els = style_elements("Titulo\n======\n");
        let h = first_el(&els, ElementKind::Heading).expect("heading");
        assert_eq!((h.start, h.end), (0, 13), "els: {els:?}");
    }

    #[test]
    fn elements_texto_plano_no_tiene_elementos() {
        assert!(style_elements("hola mundo sin formato").is_empty());
        assert!(style_elements("").is_empty());
    }

    #[test]
    fn elements_offsets_utf16_con_emoji() {
        // "**😀**": el emoji son 2 unidades UTF-16; el elemento Bold va 0..6.
        let els = style_elements("**😀**");
        let bold = first_el(&els, ElementKind::Bold).expect("bold");
        assert_eq!((bold.start, bold.end), (0, 6));
    }

    #[test]
    fn style_info_devuelve_spans_y_elementos_juntos() {
        // Una sola llamada trae ambas listas coherentes.
        let info = style_info("**hola**");
        assert!(info.spans.iter().any(|s| s.kind == SpanKind::Bold));
        assert!(info.elements.iter().any(|e| e.kind == ElementKind::Bold));
    }
}
