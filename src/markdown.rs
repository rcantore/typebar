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
}
