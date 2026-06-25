//! Fuzzy matching para el switcher de archivos (y, a futuro, la paleta de
//! comandos): scoring de subsecuencia, case-insensitive, con los indices de los
//! chars que matchearon para poder resaltarlos en la UI.
//!
//! La API (`FuzzyMatch`, `match_query`, `rank`) es el contrato que consume el
//! overlay del switcher. La heuristica de scoring puede refinarse sin tocar las
//! firmas.

// El consumidor (el overlay del switcher de archivos) todavia no esta cableado;
// se quita este allow cuando el switcher lo use. Mantener mientras tanto.
#![allow(dead_code)]

/// Resultado de matchear una query contra un candidato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Score; mayor es mejor. Premia matches consecutivos y al inicio.
    pub score: i32,
    /// Indices (por `char`) del candidato que matchearon, en orden, para
    /// resaltarlos al dibujar la lista.
    pub indices: Vec<usize>,
}

/// Compara dos chars ignorando mayusculas (aproximacion suficiente para nombres
/// de archivo y de comandos; el refinamiento Unicode completo puede venir luego).
fn chars_eq_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Matchea `query` contra `candidate` (case-insensitive). `query` debe ser
/// subsecuencia de `candidate`. Devuelve `None` si no matchea. Una query vacia
/// matchea todo, con score 0 e indices vacios.
pub fn match_query(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let ql: Vec<char> = query.chars().collect();
    if ql.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            indices: Vec::new(),
        });
    }

    let mut qi = 0;
    let mut indices = Vec::with_capacity(ql.len());
    let mut score = 0i32;
    let mut prev: Option<usize> = None;

    for (ci, ch) in candidate.chars().enumerate() {
        if qi >= ql.len() {
            break;
        }
        if chars_eq_ci(ch, ql[qi]) {
            // Bonus por match consecutivo (chars pegados leen mejor); bonus extra
            // si el match arranca al inicio del candidato (prefijo).
            if prev == Some(ci.wrapping_sub(1)) {
                score += 5;
            } else {
                score += 1;
            }
            if ci == 0 {
                score += 3;
            }
            indices.push(ci);
            prev = Some(ci);
            qi += 1;
        }
    }

    if qi == ql.len() {
        Some(FuzzyMatch { score, indices })
    } else {
        None
    }
}

/// Rankea `items` por su match contra `query`: devuelve solo los que matchean,
/// ordenados por score descendente (desempate estable por orden original). Cada
/// elemento trae su indice original en `items` y su `FuzzyMatch`.
pub fn rank(query: &str, items: &[&str]) -> Vec<(usize, FuzzyMatch)> {
    let mut out: Vec<(usize, FuzzyMatch)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, s)| match_query(query, s).map(|m| (i, m)))
        .collect();
    // Score desc; ante empate, el que venia antes en `items`.
    out.sort_by(|a, b| b.1.score.cmp(&a.1.score).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matchea_subsecuencia_y_devuelve_indices() {
        let m = match_query("mn", "main.rs").unwrap();
        assert_eq!(m.indices, vec![0, 3]); // m@0, n@3
    }

    #[test]
    fn no_matchea_si_no_es_subsecuencia() {
        assert!(match_query("xyz", "main.rs").is_none());
    }

    #[test]
    fn query_vacia_matchea_todo() {
        let m = match_query("", "lo que sea").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.indices.is_empty());
    }

    #[test]
    fn es_case_insensitive() {
        assert!(match_query("MA", "main.rs").is_some());
        assert!(match_query("ma", "MAIN.RS").is_some());
    }

    #[test]
    fn rank_filtra_y_ordena_por_score() {
        let items = ["xmainx", "main"];
        let ranked = rank("main", &items);
        // Ambos matchean; "main" puntua mas alto (consecutivo + prefijo).
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0, 1); // indice de "main" va primero
    }

    #[test]
    fn rank_descarta_los_que_no_matchean() {
        let items = ["foo", "bar"];
        let ranked = rank("z", &items);
        assert!(ranked.is_empty());
    }
}
