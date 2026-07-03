//! Export a HTML standalone desde Markdown.
//!
//! Convierte el contenido del documento a un HTML completo y autocontenido
//! (con estilos embebidos), pensado para abrir directo en un navegador o
//! compartir como archivo unico. Usa `pulldown-cmark` (parser CommonMark puro
//! Rust) con las extensiones utiles activadas: tablas, footnotes, tachado y
//! task lists.

use pulldown_cmark::{Options, Parser, html};

/// Hoja de estilos minimalista embebida en el HTML exportado: ancho centrado,
/// tipografia del sistema, y estilos legibles para code, pre y blockquote.
const STYLE: &str = "\
    :root { color-scheme: light dark; }\n\
    body {\n\
      max-width: 42rem;\n\
      margin: 2rem auto;\n\
      padding: 0 1.25rem;\n\
      font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif;\n\
      line-height: 1.6;\n\
      color: #1a1a1a;\n\
      background: #fdfdfd;\n\
    }\n\
    @media (prefers-color-scheme: dark) {\n\
      body { color: #e4e4e4; background: #1a1a1a; }\n\
      code, pre { background: #2a2a2a; }\n\
      blockquote { color: #aaa; border-left-color: #444; }\n\
    }\n\
    h1, h2, h3, h4, h5, h6 { line-height: 1.25; margin-top: 2rem; }\n\
    code {\n\
      font-family: ui-monospace, SFMono-Regular, \"SF Mono\", Menlo, Consolas, monospace;\n\
      font-size: 0.9em;\n\
      background: #f0f0f0;\n\
      padding: 0.15em 0.35em;\n\
      border-radius: 4px;\n\
    }\n\
    pre {\n\
      background: #f0f0f0;\n\
      padding: 1rem;\n\
      border-radius: 6px;\n\
      overflow-x: auto;\n\
    }\n\
    pre code { background: none; padding: 0; }\n\
    blockquote {\n\
      margin: 1rem 0;\n\
      padding-left: 1rem;\n\
      border-left: 4px solid #ddd;\n\
      color: #555;\n\
    }\n\
    table { border-collapse: collapse; }\n\
    th, td { border: 1px solid #ccc; padding: 0.4rem 0.7rem; }\n\
    img { max-width: 100%; }\n\
    a { color: #2563eb; }";

/// Convierte `markdown` a un documento HTML completo y standalone. `title` es
/// el titulo de la pagina (va en `<title>`, escapado). El body lleva el HTML
/// renderizado por pulldown-cmark con las extensiones utiles activadas.
pub fn to_html(markdown: &str, title: &str) -> String {
    // Activar las extensiones utiles mas alla de CommonMark base.
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let parser = Parser::new_ext(markdown, options);
    let mut body = String::new();
    html::push_html(&mut body, parser);

    let escaped_title = escape_html(title);

    format!(
        "<!DOCTYPE html>\n\
<html lang=\"en\">\n\
<head>\n\
  <meta charset=\"utf-8\">\n\
  <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n\
  <title>{escaped_title}</title>\n\
  <style>\n{STYLE}\n  </style>\n\
</head>\n\
<body>\n{body}</body>\n\
</html>\n"
    )
}

/// Escapa los caracteres especiales de HTML para insertar texto plano (como el
/// title) sin riesgo de romper el markup. Cubre los cinco caracteres clasicos.
fn escape_html(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_se_convierte_en_h1() {
        let html = to_html("# Heading", "t");
        assert!(html.contains("<h1>Heading</h1>"), "html: {html}");
    }

    #[test]
    fn bold_se_convierte_en_strong() {
        let html = to_html("**bold**", "t");
        assert!(html.contains("<strong>bold</strong>"), "html: {html}");
    }

    #[test]
    fn lista_se_convierte_en_li() {
        let html = to_html("- a", "t");
        assert!(html.contains("<li>a</li>"), "html: {html}");
    }

    #[test]
    fn fence_de_codigo_genera_pre_code() {
        let html = to_html("```\ncode\n```", "t");
        assert!(html.contains("<pre><code"), "html: {html}");
    }

    #[test]
    fn link_se_convierte_en_anchor() {
        let html = to_html("[t](u)", "t");
        assert!(html.contains("<a href=\"u\">t</a>"), "html: {html}");
    }

    #[test]
    fn doc_es_standalone_con_doctype_y_title() {
        let html = to_html("hola", "Mi Titulo");
        assert!(html.contains("<!DOCTYPE html>"), "html: {html}");
        assert!(html.contains("<title>Mi Titulo</title>"), "html: {html}");
        assert!(html.contains("<meta charset=\"utf-8\">"), "html: {html}");
    }

    #[test]
    fn title_se_escapa() {
        // Un title con caracteres especiales no debe romper el markup.
        let html = to_html("x", "a & b <c>");
        assert!(
            html.contains("<title>a &amp; b &lt;c&gt;</title>"),
            "html: {html}"
        );
    }

    #[test]
    fn tablas_habilitadas() {
        // La extension de tablas (GFM) debe estar activa.
        let md = "| a | b |\n|---|---|\n| 1 | 2 |";
        let html = to_html(md, "t");
        assert!(html.contains("<table>"), "html: {html}");
    }
}
