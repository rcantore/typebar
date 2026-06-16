//! Busqueda literal de subcadenas sobre el texto del documento.
//!
//! Es busqueda *literal* (no regex) y sensible a mayusculas, que es lo que
//! espera la mayoria para un find rapido. Devuelve rangos en BYTES (no chars):
//! el render trabaja en bytes para resaltar y el buffer convierte byte->char
//! cuando hace falta mover el cursor. Las coincidencias son NO solapadas: tras
//! cada match la busqueda continua desde su final.

use std::ops::Range;

/// Todas las coincidencias (no solapadas) de `needle` en `haystack`, como rangos
/// en bytes. Un `needle` vacio no matchea nada (devuelve vacio): evita un bucle
/// infinito y es el comportamiento util mientras el usuario todavia no tipeo.
pub fn find_all(haystack: &str, needle: &str) -> Vec<Range<usize>> {
    if needle.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut start = 0;
    while let Some(pos) = haystack[start..].find(needle) {
        let s = start + pos;
        let e = s + needle.len();
        out.push(s..e);
        start = e; // no solapado: seguir despues del match
    }
    out
}

/// Indice de la primera coincidencia que arranca en/despues de `from` (en
/// bytes), o la primera de todas si ninguna llega tan lejos (wrap-around). Sirve
/// para saltar al match "siguiente" desde la posicion del cursor. `None` si no
/// hay coincidencias.
pub fn next_match_from(matches: &[Range<usize>], from: usize) -> Option<usize> {
    if matches.is_empty() {
        return None;
    }
    let idx = matches.iter().position(|m| m.start >= from).unwrap_or(0);
    Some(idx)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encuentra_varias_no_solapadas() {
        assert_eq!(find_all("abcabc", "abc"), vec![0..3, 3..6]);
    }

    #[test]
    fn no_solapa_coincidencias() {
        // "aa" en "aaaa": posiciones 0 y 2, no 0,1,2 (no solapado).
        assert_eq!(find_all("aaaa", "aa"), vec![0..2, 2..4]);
    }

    #[test]
    fn needle_vacio_no_matchea() {
        assert!(find_all("hola", "").is_empty());
    }

    #[test]
    fn sensible_a_mayusculas() {
        assert_eq!(find_all("Hola hola", "hola"), vec![5..9]);
    }

    #[test]
    fn rangos_en_bytes_con_multibyte() {
        // "ñ" ocupa 2 bytes: el match de "o" tras "ñ" cae en el byte correcto.
        let t = "ñox";
        assert_eq!(find_all(t, "o"), vec![2..3]);
    }

    #[test]
    fn next_match_desde_posicion() {
        let m = find_all("a.a.a", "a"); // 0,2,4
        assert_eq!(next_match_from(&m, 0), Some(0));
        assert_eq!(next_match_from(&m, 1), Some(1)); // primer match con start>=1 es el de pos 2
        assert_eq!(next_match_from(&m, 3), Some(2));
        // Mas alla del ultimo: wrap al primero.
        assert_eq!(next_match_from(&m, 99), Some(0));
        assert_eq!(next_match_from(&[], 0), None);
    }
}
