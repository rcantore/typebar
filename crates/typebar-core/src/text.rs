//! Primitivas de geometria de texto Unicode: segmentacion en grafemas y ancho
//! en celdas de terminal.
//!
//! El editor opera en tres unidades que NO coinciden: bytes (lo que guarda el
//! Rope), chars (code points) y columnas de pantalla (celdas). Ademas, lo que
//! el usuario percibe como "un caracter" es un *grafema* (cluster), que puede
//! abarcar varios chars (emoji con ZWJ, banderas, marcas combinantes).
//!
//! El cursor se mueve y borra por grafema (nunca cae adentro de un cluster) y
//! su columna de pantalla se calcula con `grapheme_width`, que replica EXACTO
//! la funcion `cell_width` de ratatui. Asi el cursor cae siempre sobre el mismo
//! glifo que dibujo el renderer: consistencia con el render por encima de
//! "correctitud teorica" (las terminales no se ponen de acuerdo en el ancho de
//! ciertos emoji, problema sin solucion portable).

use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Katakana de medio ancho: dakuten (ﾞ) y handakuten (ﾟ).
const HALFWIDTH_KATAKANA_VOICED: char = '\u{FF9E}';
const HALFWIDTH_KATAKANA_SEMI_VOICED: char = '\u{FF9F}';

/// Ancho en celdas de terminal de un cluster de grafemas.
///
/// Replica `CellWidth for str` de ratatui (`buffer/cell_width.rs`):
/// - ASCII de un byte -> 1 (los caracteres de control se filtran antes).
/// - resto -> `UnicodeWidthStr::width` + 1 por cada dakuten/handakuten de medio
///   ancho, que `unicode-width` reporta como 0 pero las terminales dibujan como
///   1 celda independiente.
pub fn grapheme_width(cluster: &str) -> usize {
    if cluster.len() == 1 {
        return 1;
    }
    let halfwidth_marks = cluster
        .chars()
        .filter(|c| {
            matches!(
                *c,
                HALFWIDTH_KATAKANA_VOICED | HALFWIDTH_KATAKANA_SEMI_VOICED
            )
        })
        .count();
    UnicodeWidthStr::width(cluster) + halfwidth_marks
}

/// Ancho en celdas de terminal de una cadena entera: suma el ancho de cada
/// grafema (con `grapheme_width`, que replica a ratatui). Lo que se necesita
/// para mapear texto a columnas visuales (ej hit-testing del mouse), donde
/// contar chars desalinea con CJK/emoji de doble ancho.
pub fn display_width(s: &str) -> usize {
    s.graphemes(true).map(grapheme_width).sum()
}

/// Itera los grafemas de `s` junto con su ancho en celdas (`grapheme_width`).
/// Evita que cada consumidor (ej el motor de soft wrap de la TUI) tenga que
/// agregar `unicode-segmentation` como dependencia propia solo para volver a
/// segmentar: el core ya expone la segmentacion + el ancho juntos.
pub fn graphemes_with_width(s: &str) -> impl Iterator<Item = (&str, usize)> {
    s.graphemes(true).map(|g| (g, grapheme_width(g)))
}

/// Cuenta palabras segun los limites de palabra de Unicode (UAX #29): cuenta
/// palabras reales, no espacios ni puntuacion, y anda en español, CJK, etc. sin
/// reglas ad-hoc. Apoyado en `unicode-segmentation`, que ya es dependencia.
pub fn count_words(s: &str) -> usize {
    s.unicode_words().count()
}

/// Analisis de una linea (sin el `\n` final): donde empieza cada grafema, en
/// indice de *char*, y el ancho en celdas de cada uno.
pub struct LineGraphemes {
    /// Indices de char de cada limite de grafema: `[0, .., len_chars]`.
    /// Longitud = cantidad de grafemas + 1.
    boundaries: Vec<usize>,
    /// Ancho en celdas del grafema `i` (entre `boundaries[i]` y `[i+1]`).
    widths: Vec<usize>,
}

impl LineGraphemes {
    /// Segmenta `line` en grafemas extendidos (igual que ratatui).
    pub fn analyze(line: &str) -> Self {
        let mut boundaries = vec![0];
        let mut widths = Vec::new();
        let mut chars = 0;
        for g in line.graphemes(true) {
            chars += g.chars().count();
            boundaries.push(chars);
            widths.push(grapheme_width(g));
        }
        Self { boundaries, widths }
    }

    /// Cantidad de chars de la linea (= indice de char del fin de linea).
    pub fn len_chars(&self) -> usize {
        *self
            .boundaries
            .last()
            .expect("boundaries siempre tiene el 0")
    }

    /// Indice del limite `col` dentro de `boundaries`, si `col` es un limite.
    fn boundary_index(&self, col: usize) -> Option<usize> {
        self.boundaries.iter().position(|&b| b == col)
    }

    /// Limite de grafema *siguiente* a `col` (o `col` mismo si ya esta al fin).
    pub fn next_boundary(&self, col: usize) -> usize {
        match self.boundary_index(col) {
            Some(i) if i + 1 < self.boundaries.len() => self.boundaries[i + 1],
            _ => col,
        }
    }

    /// Limite de grafema *anterior* a `col` (o `col` mismo si ya esta al inicio).
    pub fn prev_boundary(&self, col: usize) -> usize {
        match self.boundary_index(col) {
            Some(i) if i > 0 => self.boundaries[i - 1],
            _ => col,
        }
    }

    /// Columna de pantalla (celdas) del limite en el char-index `col`.
    pub fn display_col(&self, col: usize) -> usize {
        // `col` deberia ser siempre un limite; si no, caemos al limite <= col.
        let i = self
            .boundary_index(col)
            .unwrap_or_else(|| self.boundaries.iter().rposition(|&b| b <= col).unwrap_or(0));
        self.widths[..i].iter().sum()
    }

    /// Char-index del limite de grafema cuya columna de pantalla sea la mayor
    /// `<= target`. Sirve para el movimiento vertical (preferred column).
    pub fn col_for_display(&self, target: usize) -> usize {
        let mut acc = 0;
        let mut best = 0;
        for (i, &w) in self.widths.iter().enumerate() {
            if acc <= target {
                best = self.boundaries[i];
            } else {
                return best;
            }
            acc += w;
        }
        // El fin de linea tambien es un destino valido si target cae mas alla.
        if acc <= target {
            best = self.len_chars();
        }
        best
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ancho_ascii_y_acentos() {
        assert_eq!(grapheme_width("a"), 1);
        assert_eq!(grapheme_width("é"), 1); // NFC: 1 char, 1 celda
        assert_eq!(grapheme_width("e\u{301}"), 1); // NFD: base + combinante, 1 celda
    }

    #[test]
    fn ancho_cjk_y_emoji() {
        assert_eq!(grapheme_width("中"), 2);
        assert_eq!(grapheme_width("あ"), 2);
        assert_eq!(grapheme_width("😀"), 2);
    }

    #[test]
    fn ancho_katakana_halfwidth_dakuten() {
        // ﾞ sola: unicode-width la da 0, pero ocupa 1 celda en terminal.
        assert_eq!(grapheme_width("\u{FF9E}"), 1);
    }

    #[test]
    fn limites_saltan_grafema_completo() {
        // "a😀b": el emoji es 1 grafema de 1 char (scalar unico) -> limites 0,1,2,3
        let g = LineGraphemes::analyze("a😀b");
        assert_eq!(g.len_chars(), 3);
        assert_eq!(g.next_boundary(0), 1); // a -> emoji
        assert_eq!(g.next_boundary(1), 2); // emoji -> b
        assert_eq!(g.prev_boundary(2), 1);
    }

    #[test]
    fn limites_cluster_zwj_multichar() {
        // Familia: varios chars unidos por ZWJ, un solo grafema.
        let familia = "👨\u{200D}👩\u{200D}👧";
        let line = format!("x{familia}y");
        let g = LineGraphemes::analyze(&line);
        let fam_chars = familia.chars().count(); // 5
        // Desde el 'x' (col 1) el siguiente limite salta TODO el cluster.
        assert_eq!(g.next_boundary(1), 1 + fam_chars);
        // Y volver salta el cluster entero, no un char ZWJ suelto.
        assert_eq!(g.prev_boundary(1 + fam_chars), 1);
    }

    #[test]
    fn display_col_suma_anchos() {
        // "a中b": a(1) 中(2) b(1) -> columnas 0,1,3,4
        let g = LineGraphemes::analyze("a中b");
        assert_eq!(g.display_col(0), 0);
        assert_eq!(g.display_col(1), 1); // despues de 'a'
        assert_eq!(g.display_col(2), 3); // despues de '中' (ocupa 2)
        assert_eq!(g.display_col(3), 4);
    }

    #[test]
    fn col_for_display_respeta_anchos() {
        // "中中": cada uno 2 celdas. Limites en chars: 0,1,2. Columnas: 0,2,4.
        let g = LineGraphemes::analyze("中中");
        assert_eq!(g.col_for_display(0), 0);
        assert_eq!(g.col_for_display(1), 0); // col 1 cae en medio del 1er glifo -> snap al 0
        assert_eq!(g.col_for_display(2), 1); // inicio del 2do glifo
        assert_eq!(g.col_for_display(3), 1);
        assert_eq!(g.col_for_display(99), 2); // mas alla -> fin de linea
    }

    #[test]
    fn graphemes_with_width_empareja_cluster_y_ancho() {
        // "a中b": grafemas y anchos emparejados en orden, sin partir el CJK.
        let pairs: Vec<(&str, usize)> = graphemes_with_width("a中b").collect();
        assert_eq!(pairs, vec![("a", 1), ("中", 2), ("b", 1)]);
    }

    #[test]
    fn graphemes_with_width_no_parte_clusters_extendidos() {
        // Familia con ZWJ: un solo item, no uno por char interno.
        let familia = "👨\u{200D}👩\u{200D}👧";
        let pairs: Vec<(&str, usize)> = graphemes_with_width(familia).collect();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].0, familia);
    }

    #[test]
    fn cuenta_palabras_unicode() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \n\t  "), 0);
        assert_eq!(count_words("hola"), 1);
        assert_eq!(count_words("hola mundo"), 2);
        // Puntuacion y markdown no cuentan como palabras sueltas.
        assert_eq!(count_words("**hola**, _mundo_!"), 2);
        // Multiples espacios / saltos de linea colapsan.
        assert_eq!(count_words("uno  dos\ntres"), 3);
        // Acentos y ñ: una palabra cada uno.
        assert_eq!(count_words("año señor"), 2);
        // Contracciones cuentan como una palabra (UAX #29).
        assert_eq!(count_words("don't"), 1);
    }
}
