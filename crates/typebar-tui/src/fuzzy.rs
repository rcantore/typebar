//! Fuzzy matching para el switcher de archivos (y, a futuro, la paleta de
//! comandos): scoring de subsecuencia, case-insensitive, con los indices de los
//! chars que matchearon para poder resaltarlos en la UI.
//!
//! La API (`FuzzyMatch`, `match_query`, `rank`) es el contrato que consume el
//! overlay del switcher; la heuristica se refina sin tocar las firmas.
//!
//! Enfoque del algoritmo: un alineamiento por programacion dinamica al estilo
//! fzy (O(n*m), n = chars de la query, m = chars del candidato). En vez de un
//! greedy left-most (que ignora bonus de boundary, por ejemplo `fb` sobre
//! `foo_bar` querria f@0 y b@4 tras el `_`, no el primer `b` que aparezca),
//! llenamos dos matrices: `d[i][j]` es el mejor score de un alineamiento de los
//! primeros i+1 chars de la query que TERMINA matcheando el char j del
//! candidato; `best[i][j]` es el mejor score de matchear esos i+1 chars usando
//! cualquier columna hasta j (haya o no match en j). Con ambas reconstruimos por
//! backtracking los indices del mejor alineamiento. Se premian boundaries,
//! matches consecutivos y el prefijo; se penalizan los gaps (mas fuerte el
//! inicial).

/// Resultado de matchear una query contra un candidato.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FuzzyMatch {
    /// Score; mayor es mejor. Premia boundaries, matches consecutivos y prefijo.
    pub score: i32,
    /// Indices (por `char`) del candidato que matchearon, en orden creciente,
    /// uno por cada char de la query, para resaltarlos al dibujar la lista.
    pub indices: Vec<usize>,
}

// Pesos del scoring. Un boundary y un consecutivo pesan mas que un match suelto
// (leen como "pegado a algo"). Los gaps restan, mas el inicial (leading).
const SCORE_MATCH: i32 = 16; // base por cada char que matchea
const BONUS_BOUNDARY: i32 = 30; // match justo tras un separador o en camelCase
const BONUS_CONSECUTIVE: i32 = 24; // match pegado al char anterior de la query
const BONUS_FIRST_CHAR: i32 = 8; // extra si el match es el primer char (prefijo)
const PENALTY_GAP_START: i32 = 4; // costo de abrir un gap (saltear chars)
const PENALTY_GAP_EXTEND: i32 = 2; // costo por cada char extra dentro del gap
const PENALTY_LEADING: i32 = 3; // costo por cada char saltado ANTES del 1er match

// Sentinela de "imposible" para celdas de la DP. Bien negativo, pero con margen
// para que sumarle penalties no haga overflow.
const NEG_INF: i32 = i32::MIN / 4;

/// Compara dos chars ignorando mayusculas (aproximacion suficiente para nombres
/// de archivo y de comandos; el refinamiento Unicode completo puede venir luego).
fn chars_eq_ci(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

/// Indica si el char en `pos` del candidato es un "boundary": el primer char, o
/// el que sigue a un separador (`/ _ - . espacio`), o una transicion camelCase
/// (minuscula seguida de Mayuscula). En esos lugares conviene anclar el match.
fn is_boundary(chars: &[char], pos: usize) -> bool {
    if pos == 0 {
        return true;
    }
    let prev = chars[pos - 1];
    let cur = chars[pos];
    if matches!(prev, '/' | '_' | '-' | '.' | ' ' | '\\') {
        return true;
    }
    // camelCase: minuscula -> Mayuscula. `is_lowercase`/`is_uppercase` ya cubren
    // letras no ASCII (por ejemplo "ñ" -> "Ñ").
    prev.is_lowercase() && cur.is_uppercase()
}

/// Bonus posicional de matchear el char `pos` del candidato (independiente de si
/// el anterior de la query matcheo justo antes; lo consecutivo se suma aparte).
fn position_bonus(chars: &[char], pos: usize) -> i32 {
    let mut b = 0;
    if is_boundary(chars, pos) {
        b += BONUS_BOUNDARY;
    }
    if pos == 0 {
        b += BONUS_FIRST_CHAR;
    }
    b
}

/// Matchea `query` contra `candidate` (case-insensitive). `query` debe ser
/// subsecuencia de `candidate`. Devuelve `None` si no matchea. Una query vacia
/// matchea todo, con score 0 e indices vacios.
///
/// El score sale del mejor alineamiento (DP), no del primero que aparezca.
pub fn match_query(query: &str, candidate: &str) -> Option<FuzzyMatch> {
    let q: Vec<char> = query.chars().collect();
    if q.is_empty() {
        return Some(FuzzyMatch {
            score: 0,
            indices: Vec::new(),
        });
    }

    let c: Vec<char> = candidate.chars().collect();
    let n = q.len();
    let m = c.len();
    if m < n {
        return None; // la query no puede ser subsecuencia de algo mas corto
    }

    // Matriz de "matchea?" para no recalcular en la DP ni en el backtracking.
    // eq[i][j] = q[i] coincide (CI) con c[j].
    let mut eq = vec![false; n * m];
    for i in 0..n {
        for j in 0..m {
            eq[i * m + j] = chars_eq_ci(c[j], q[i]);
        }
    }
    // Verificacion rapida de subsecuencia: si algun char de la query no aparece
    // en orden, no hay match y evitamos correr la DP.
    {
        let mut qi = 0;
        for j in 0..m {
            if qi < n && eq[qi * m + j] {
                qi += 1;
            }
        }
        if qi != n {
            return None;
        }
    }

    // d[i][j]: mejor score de alinear q[0..=i] terminando en un match en c[j].
    // best[i][j]: mejor score de alinear q[0..=i] usando columnas 0..=j (con o
    // sin match en j); sirve para "arrastrar" el optimo y para el backtracking.
    let mut d = vec![NEG_INF; n * m];
    let mut best = vec![NEG_INF; n * m];

    for i in 0..n {
        let mut prev_best = NEG_INF; // best[i][j-1], para arrastrar el optimo
        for j in 0..m {
            // Score de cerrar el alineamiento de q[0..=i] con un match en c[j].
            let match_here = if eq[i * m + j] {
                if i == 0 {
                    // Primer char de la query: el costo de "arrancar" aca son los
                    // j chars salteados antes (leading), mas el bonus posicional.
                    let leading = (j as i32) * PENALTY_LEADING;
                    SCORE_MATCH + position_bonus(&c, j) - leading
                } else if j == 0 {
                    NEG_INF // q[i>0] no puede matchear en c[0] (no hay prefijo previo)
                } else {
                    // Dos formas de llegar a un match en (i, j):
                    //  a) consecutivo: q[i-1] matcheo en c[j-1] -> d[i-1][j-1].
                    //  b) tras un gap: q[i-1] matcheo en alguna col < j-1, tomamos
                    //     best[i-1][j-1] y cobramos la apertura del gap.
                    let consec = d[(i - 1) * m + (j - 1)];
                    let consec = if consec > NEG_INF / 2 {
                        consec + SCORE_MATCH + position_bonus(&c, j) + BONUS_CONSECUTIVE
                    } else {
                        NEG_INF
                    };
                    let gapped = best[(i - 1) * m + (j - 1)];
                    let gapped = if gapped > NEG_INF / 2 {
                        gapped + SCORE_MATCH + position_bonus(&c, j) - PENALTY_GAP_START
                    } else {
                        NEG_INF
                    };
                    consec.max(gapped)
                }
            } else {
                NEG_INF
            };
            d[i * m + j] = match_here;

            // best[i][j] = max(match aca, arrastrar best[i][j-1] cobrando que el
            // char j queda sin usar para este prefijo de la query, es decir un
            // char mas de gap pendiente).
            let carried = if prev_best > NEG_INF / 2 {
                prev_best - PENALTY_GAP_EXTEND
            } else {
                NEG_INF
            };
            let cell_best = match_here.max(carried);
            best[i * m + j] = cell_best;
            prev_best = cell_best;
        }
    }

    // El score final es el mejor alineamiento de TODA la query que termina en un
    // match (los chars del candidato tras el ultimo match no cobran penalty: solo
    // importa donde anclo cada char de la query). Es el maximo de la ultima fila
    // de `d`; guardamos su columna como arranque del backtracking.
    let last = n - 1;
    let mut final_score = NEG_INF;
    let mut end_col = 0usize;
    for j in 0..m {
        let dj = d[last * m + j];
        if dj > final_score {
            final_score = dj;
            end_col = j;
        }
    }
    if final_score <= NEG_INF / 2 {
        return None;
    }

    // Backtracking fiel al camino: desde (last, end_col) reconstruimos columna a
    // columna. En cada match decidimos si el predecesor fue consecutivo (rama
    // `d[i-1][col-1]`) o tras un gap (rama `best[i-1][col-1]`) viendo cual
    // reproduce el `d[i][col]` guardado. En la rama con gap, ubicamos la columna
    // real de q[i-1] retrocediendo y descontando la extension de gap acumulada.
    let mut indices = vec![0usize; n];
    let mut col = end_col;
    for i in (0..n).rev() {
        indices[i] = col;
        if i == 0 {
            break;
        }
        let here = d[i * m + col];
        let pos = SCORE_MATCH + position_bonus(&c, col);
        // Rama consecutiva: el char anterior matcheo justo en col-1.
        let consec_pred = d[(i - 1) * m + (col - 1)];
        let from_consec =
            consec_pred > NEG_INF / 2 && consec_pred + pos + BONUS_CONSECUTIVE == here;
        if from_consec {
            col -= 1;
            continue;
        }
        // Rama con gap: `best[i-1][col-1]` arrastra el optimo de q[i-1] en alguna
        // columna <= col-1; lo ubicamos buscando el `d` que coincide.
        let target = best[(i - 1) * m + (col - 1)];
        let mut k = col - 1;
        let mut acc = target; // best en la columna k (col-1), con gaps ya cobrados
        loop {
            if d[(i - 1) * m + k] == acc {
                break;
            }
            if k == 0 {
                // No deberia pasar: la DP garantiza un predecesor valido.
                break;
            }
            // Al retroceder una columna, `best` valia PENALTY_GAP_EXTEND mas (lo
            // que se le resto al arrastrarlo hacia la derecha).
            acc += PENALTY_GAP_EXTEND;
            k -= 1;
        }
        col = k;
    }

    Some(FuzzyMatch {
        score: final_score,
        indices,
    })
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
    // Score desc; ante empate, el que venia antes en `items` (orden estable).
    out.sort_by(|a, b| b.1.score.cmp(&a.1.score).then(a.0.cmp(&b.0)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- Subsecuencia: matchea / no matchea ---

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
    fn no_matchea_si_falta_un_char() {
        // 'z' no esta en el candidato: aunque "ma" si sea subsecuencia.
        assert!(match_query("maz", "main.rs").is_none());
    }

    #[test]
    fn no_matchea_si_candidato_mas_corto_que_query() {
        assert!(match_query("largo", "ab").is_none());
    }

    #[test]
    fn matchea_query_igual_al_candidato() {
        let m = match_query("main", "main").unwrap();
        assert_eq!(m.indices, vec![0, 1, 2, 3]);
    }

    // --- Case-insensitivity en ambas direcciones ---

    #[test]
    fn es_case_insensitive_query_mayus() {
        let m = match_query("MA", "main.rs").unwrap();
        assert_eq!(m.indices, vec![0, 1]);
    }

    #[test]
    fn es_case_insensitive_candidato_mayus() {
        let m = match_query("ma", "MAIN.RS").unwrap();
        assert_eq!(m.indices, vec![0, 1]);
    }

    // --- Query vacia ---

    #[test]
    fn query_vacia_matchea_todo() {
        let m = match_query("", "lo que sea").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.indices.is_empty());
    }

    #[test]
    fn query_vacia_matchea_candidato_vacio() {
        let m = match_query("", "").unwrap();
        assert_eq!(m.score, 0);
        assert!(m.indices.is_empty());
    }

    #[test]
    fn query_no_vacia_no_matchea_candidato_vacio() {
        assert!(match_query("a", "").is_none());
    }

    // --- Bonus de boundary ---

    #[test]
    fn boundary_puntua_mas_que_no_boundary() {
        // Mismo char 'b' matcheado, una vez en boundary (tras '_') y otra en el
        // medio de una palabra. El boundary debe puntuar mas alto.
        let en_boundary = match_query("b", "foo_bar").unwrap();
        let en_medio = match_query("b", "foobar").unwrap();
        assert!(
            en_boundary.score > en_medio.score,
            "boundary {} deberia superar a medio {}",
            en_boundary.score,
            en_medio.score
        );
    }

    #[test]
    fn boundary_camelcase_puntua_mas() {
        // 'B' en "fooBar" es boundary camelCase (o->B); en "foobar" no.
        let camel = match_query("b", "fooBar").unwrap();
        let plano = match_query("b", "fooxbar").unwrap();
        assert!(camel.score > plano.score);
    }

    #[test]
    fn boundary_tras_separadores_varios() {
        for cand in ["a/bc", "a-bc", "a.bc", "a bc", "a_bc"] {
            let m = match_query("b", cand).unwrap();
            // 'b' esta en la posicion 2 (tras el separador en la 1).
            assert_eq!(m.indices, vec![2], "fallo con {cand}");
        }
    }

    // --- Bonus de consecutivos ---

    #[test]
    fn consecutivos_puntuan_mas_que_dispersos() {
        // "ab" pegados vs separados por gaps.
        let pegados = match_query("ab", "abxxxx").unwrap();
        let dispersos = match_query("ab", "axxxxb").unwrap();
        assert!(
            pegados.score > dispersos.score,
            "pegados {} vs dispersos {}",
            pegados.score,
            dispersos.score
        );
    }

    // --- Bonus de prefijo (primer char) ---

    #[test]
    fn prefijo_puntua_mas_que_match_interno() {
        // 'm' al inicio (prefijo + boundary) vs 'm' interno.
        let prefijo = match_query("m", "main").unwrap();
        let interno = match_query("m", "xmain").unwrap();
        assert!(
            prefijo.score > interno.score,
            "prefijo {} vs interno {}",
            prefijo.score,
            interno.score
        );
    }

    // --- Correccion de indices cuando el mejor alineamiento NO es leftmost ---

    #[test]
    fn indices_eligen_boundary_no_leftmost() {
        // query "fb" sobre "foo_bar": el greedy leftmost tomaria f@0 y el primer
        // 'b' que aparezca. Aca el unico 'b' esta en la 4 (boundary tras '_'),
        // pero el caso clave es que el mejor alineamiento ancla en ese boundary.
        let m = match_query("fb", "foo_bar").unwrap();
        assert_eq!(m.indices, vec![0, 4]); // f@0, b@4 (tras '_')
    }

    #[test]
    fn indices_prefieren_alineamiento_en_boundary_no_leftmost() {
        // query "rc" sobre "src/rcfile": s0 r1 c2 /3 r4 c5 ...
        // El greedy leftmost tomaria r@1 y c@2 (consecutivos, pero en medio de
        // "src"). El mejor alineamiento ancla en el boundary tras '/': r@4 (tras
        // separador) y c@5 (consecutivo), que puntua bastante mas alto.
        let m = match_query("rc", "src/rcfile").unwrap();
        assert_eq!(
            m.indices,
            vec![4, 5],
            "deberia anclar en el boundary tras '/', no en el leftmost"
        );
    }

    #[test]
    fn indices_son_crecientes_y_uno_por_char() {
        let m = match_query("abc", "axbxcx").unwrap();
        assert_eq!(m.indices.len(), 3);
        assert!(m.indices.windows(2).all(|w| w[0] < w[1]));
    }

    // --- rank: orden por score y desempate estable ---

    #[test]
    fn rank_filtra_y_ordena_por_score() {
        let items = ["xmainx", "main"];
        let ranked = rank("main", &items);
        // Ambos matchean; "main" puntua mas alto (prefijo + consecutivos).
        assert_eq!(ranked.len(), 2);
        assert_eq!(ranked[0].0, 1); // indice de "main" va primero
    }

    #[test]
    fn rank_descarta_los_que_no_matchean() {
        let items = ["foo", "bar"];
        let ranked = rank("z", &items);
        assert!(ranked.is_empty());
    }

    #[test]
    fn rank_desempate_estable_por_orden_original() {
        // Tres candidatos con el MISMO score (mismo match): deben salir en el
        // orden original de `items`.
        let items = ["aaa", "bbb", "ccc"];
        // query vacia: todos score 0, empate total.
        let ranked = rank("", &items);
        assert_eq!(
            ranked.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
    }

    #[test]
    fn rank_devuelve_indice_original() {
        let items = ["zzz", "alpha", "beta"];
        let ranked = rank("a", &items);
        // "alpha" (idx 1) y "beta" (idx 2) matchean; "zzz" (idx 0) no.
        let idxs: Vec<usize> = ranked.iter().map(|(i, _)| *i).collect();
        assert!(idxs.contains(&1) && idxs.contains(&2) && !idxs.contains(&0));
    }

    // --- No-ASCII ---

    #[test]
    fn matchea_nombre_con_no_ascii() {
        // Indices por CHAR, no por byte: "ñ" es 1 char aunque ocupe 2 bytes. El
        // candidato en chars es:
        //   m  a  ñ  a  n  a  _  n  o  t  a  s
        //   0  1  2  3  4  5  6  7  8  9 10 11
        // Query "ano". El mejor alineamiento ancla la 'n' en el boundary tras '_'
        // (col 7) y la 'o' consecutiva (col 8), no en la 'n' interna (col 4).
        let m = match_query("ano", "mañana_notas").unwrap();
        assert_eq!(m.indices, vec![1, 7, 8]);
        // Y los indices caen sobre los chars correctos del candidato original.
        let chars: Vec<char> = "mañana_notas".chars().collect();
        assert_eq!((chars[1], chars[7], chars[8]), ('a', 'n', 'o'));
    }

    #[test]
    fn case_insensitive_no_ascii() {
        // "Ñ" debe matchear "ñ".
        assert!(match_query("Ñ", "mañana").is_some());
    }

    #[test]
    fn boundary_camelcase_no_ascii() {
        // Transicion minuscula->Mayuscula con "ñ"->"Ñ" cuenta como boundary.
        let camel = match_query("ñ", "fooÑar").unwrap();
        let plano = match_query("ñ", "fooxñar").unwrap();
        assert!(camel.score > plano.score);
    }
}
