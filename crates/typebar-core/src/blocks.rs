//! Segmentacion del documento en bloques de nivel superior para el modo
//! WYSIWYG por bloques de la GUI (y, a futuro, quiza la TUI).
//!
//! Es el mismo concepto del "Nivel 2" de la TUI (la linea activa se ve cruda y
//! el resto contraido/renderizado), subido de linea a bloque: cada bloque de
//! nivel superior se puede mostrar renderizado (`html`) cuando no tiene el foco
//! o crudo (`source`) cuando se edita. El markdown sigue siendo la unica fuente
//! de verdad: nunca convertimos HTML de vuelta a markdown.
//!
//! Esta API la consume hoy `typebar-gui` (columna de bloques estilo Typora) y
//! podria consumirla manana la TUI. El markdown se parsea con `pulldown-cmark`,
//! con las mismas extensiones que activa `export::to_html`.

use pulldown_cmark::{Event, Options, Parser, html};

/// Un bloque de nivel superior del documento.
///
/// - `source` es el slice EXACTO del markdown original correspondiente al
///   bloque, incluyendo el espaciado y las lineas en blanco que lo siguen hasta
///   el proximo bloque. El primer bloque absorbe ademas cualquier espacio
///   inicial del documento y el ultimo el espacio final, de modo que la
///   concatenacion de los `source` de todos los bloques reconstruye el
///   documento original byte a byte (round-trip sin perdidas).
/// - `html` es el fragmento HTML de ese mismo slice, generado con `push_html`,
///   SIN el envoltorio standalone (`<!DOCTYPE>`, `<head>`, estilos) que arma
///   `export::to_html`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    /// Markdown crudo del bloque (slice exacto del documento original).
    pub source: String,
    /// Fragmento HTML renderizado de `source`.
    pub html: String,
}

/// Extensiones habilitadas, iguales a las de `export::to_html`, para que el
/// render por bloques coincida con el export standalone: tablas, footnotes,
/// tachado y task lists.
fn block_options() -> Options {
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options
}

/// Renderiza un fragmento de markdown a HTML (sin envoltorio standalone).
fn render_fragment(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, block_options());
    let mut out = String::new();
    html::push_html(&mut out, parser);
    out
}

/// Parte `markdown` en la columna de bloques de nivel superior que muestra la
/// GUI. Devuelve un `Vec<Block>` en orden de documento; la concatenacion de los
/// `source` reconstruye el documento entrada byte a byte.
///
/// Un documento vacio devuelve un vector vacio (la GUI se encarga de mostrar un
/// bloque editable en blanco para poder empezar a escribir).
///
/// La segmentacion usa `into_offset_iter()`: cada evento del parser trae su
/// rango en bytes. Llevando la cuenta de la profundidad de anidamiento nos
/// quedamos solo con los bloques de profundidad 0; asi una lista con sublistas,
/// un blockquote multilinea o un code fence con lineas en blanco adentro quedan
/// como un unico bloque de nivel superior y no se parten.
pub fn html_blocks(markdown: &str) -> Vec<Block> {
    if markdown.is_empty() {
        return Vec::new();
    }

    // Puntos de corte: el offset (en bytes) donde arranca cada bloque de nivel
    // superior. Los eventos Start incrementan la profundidad y los End la
    // decrementan; solo registramos los que ocurren en profundidad 0. La regla
    // horizontal y los bloques HTML no vienen como par Start/End sino como
    // eventos atomicos, asi que tambien los contemplamos en profundidad 0.
    let parser = Parser::new_ext(markdown, block_options());
    let mut starts: Vec<usize> = Vec::new();
    let mut depth: i32 = 0;
    for (event, range) in parser.into_offset_iter() {
        match event {
            Event::Start(_) => {
                if depth == 0 {
                    push_start(&mut starts, range.start);
                }
                depth += 1;
            }
            Event::End(_) => {
                depth -= 1;
            }
            Event::Rule | Event::Html(_) => {
                if depth == 0 {
                    push_start(&mut starts, range.start);
                }
            }
            _ => {}
        }
    }

    // Sin cortes (p.ej. un documento de solo espacios en blanco, o solo una
    // definicion de referencia de enlace que no emite eventos): devolvemos un
    // unico bloque con todo el contenido para no perder esos bytes.
    if starts.is_empty() {
        return vec![Block {
            source: markdown.to_string(),
            html: render_fragment(markdown),
        }];
    }

    // Particionamos el documento por los puntos de corte. El primer bloque
    // arranca en 0 (absorbe el espacio inicial) y el ultimo llega hasta el final
    // (absorbe el espacio final). Cada bloque va desde su inicio hasta el inicio
    // del siguiente, sin huecos ni solapes: la union de los slices es el
    // documento entero.
    let mut blocks = Vec::with_capacity(starts.len());
    for i in 0..starts.len() {
        let from = if i == 0 { 0 } else { starts[i] };
        let to = starts.get(i + 1).copied().unwrap_or(markdown.len());
        let source = &markdown[from..to];
        blocks.push(Block {
            source: source.to_string(),
            html: render_fragment(source),
        });
    }
    blocks
}

/// Registra un punto de corte, evitando duplicar el mismo offset (un bloque HTML
/// multilinea puede emitir varios eventos `Html` contiguos con el mismo inicio
/// de bloque; nos quedamos con el primero para no partirlo).
fn push_start(starts: &mut Vec<usize>, offset: usize) {
    if starts.last() != Some(&offset) {
        starts.push(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: concatena los `source` de los bloques (debe reconstruir la
    /// entrada byte a byte).
    fn rejoin(blocks: &[Block]) -> String {
        blocks.iter().map(|b| b.source.as_str()).collect()
    }

    #[test]
    fn doc_vacio_no_da_bloques() {
        let blocks = html_blocks("");
        assert!(blocks.is_empty(), "blocks: {blocks:?}");
        assert_eq!(rejoin(&blocks), "");
    }

    #[test]
    fn un_solo_parrafo_es_un_bloque() {
        let doc = "Un parrafo simple.";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].html.contains("<p>Un parrafo simple.</p>"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn varios_headings_y_parrafos_se_separan() {
        let doc = "# Titulo\n\nParrafo uno.\n\n## Subtitulo\n\nParrafo dos.\n";
        let blocks = html_blocks(doc);
        // Cuatro bloques: h1, parrafo, h2, parrafo.
        assert_eq!(blocks.len(), 4, "blocks: {blocks:?}");
        assert!(blocks[0].html.contains("<h1>Titulo</h1>"));
        assert!(blocks[1].html.contains("<p>Parrafo uno.</p>"));
        assert!(blocks[2].html.contains("<h2>Subtitulo</h2>"));
        assert!(blocks[3].html.contains("<p>Parrafo dos.</p>"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn lista_anidada_es_un_solo_bloque() {
        let doc = "- uno\n  - anidado a\n  - anidado b\n- dos\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 1, "una lista de nivel superior = un bloque");
        assert!(blocks[0].html.contains("<ul>"));
        assert!(blocks[0].html.contains("anidado a"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn code_fence_con_lineas_en_blanco_no_se_parte() {
        // Las lineas en blanco DENTRO del fence no deben partir el bloque.
        let doc = "```rust\nlet a = 1;\n\nlet b = 2;\n```\n\nDespues.\n";
        let blocks = html_blocks(doc);
        assert_eq!(
            blocks.len(),
            2,
            "fence + parrafo = 2 bloques; blocks: {blocks:?}"
        );
        assert!(blocks[0].source.contains("let a = 1;"));
        assert!(blocks[0].source.contains("let b = 2;"));
        assert!(blocks[0].html.contains("<pre><code"));
        assert!(blocks[1].html.contains("<p>Despues.</p>"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn blockquote_multilinea_es_un_bloque() {
        let doc = "> primera linea\n> segunda linea\n> tercera linea\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].html.contains("<blockquote>"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn tabla_es_un_bloque() {
        let doc = "| a | b |\n|---|---|\n| 1 | 2 |\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert!(blocks[0].html.contains("<table>"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn espaciado_raro_entre_bloques_se_conserva() {
        // Tres o mas newlines entre bloques: el round-trip los preserva pegados
        // al bloque que los precede.
        let doc = "Primero.\n\n\n\nSegundo.\n\n\nTercero.\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 3, "blocks: {blocks:?}");
        assert_eq!(rejoin(&blocks), doc, "el espaciado raro debe reconstruirse");
    }

    #[test]
    fn espacio_inicial_y_final_se_absorbe() {
        // Lineas en blanco al principio y al final del documento tambien deben
        // sobrevivir al round-trip (las absorben el primer y el ultimo bloque).
        let doc = "\n\n# Titulo\n\nCuerpo.\n\n\n";
        let blocks = html_blocks(doc);
        assert_eq!(rejoin(&blocks), doc);
        assert!(!blocks.is_empty());
    }

    #[test]
    fn documento_mixto_hace_round_trip_byte_a_byte() {
        // Un documento con casi todas las construcciones juntas: la union de los
        // sources debe ser identica a la entrada, byte por byte.
        let doc = "\
# Encabezado

Un parrafo con **negrita** y `codigo`.

- item uno
  - sub item
- item dos

```python
def f():

    return 1
```

> una cita
> en dos lineas

| col a | col b |
|-------|-------|
| 1     | 2     |

---

Parrafo final.
";
        let blocks = html_blocks(doc);
        assert_eq!(rejoin(&blocks), doc, "round-trip debe ser exacto");
        // Chequeo de cordura: se segmento en varios bloques, no en uno solo.
        assert!(
            blocks.len() >= 6,
            "esperaba varios bloques; blocks: {blocks:?}"
        );
    }

    #[test]
    fn regla_horizontal_es_su_propio_bloque() {
        let doc = "Antes.\n\n---\n\nDespues.\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 3, "blocks: {blocks:?}");
        assert!(blocks[1].html.contains("<hr"));
        assert_eq!(rejoin(&blocks), doc);
    }

    #[test]
    fn solo_espacios_en_blanco_es_un_bloque_sin_perder_bytes() {
        let doc = "\n\n\n";
        let blocks = html_blocks(doc);
        assert_eq!(blocks.len(), 1);
        assert_eq!(rejoin(&blocks), doc);
    }
}
