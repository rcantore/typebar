//! Descubrimiento de archivos para el switcher: recorre un directorio
//! recursivamente y devuelve los paths (relativos al root) de los archivos,
//! salteando ruido (entradas ocultas y dirs como `.git`, `target`,
//! `node_modules`). Sin dependencias: recursion manual sobre `read_dir`.
//!
//! Los symlinks a directorios NO se siguen (se tratan como hoja), asi que no hay
//! riesgo de loops; ademas un tope `MAX_FILES` acota arboles enormes y un tope
//! `MAX_DEPTH` acota arboles patologicamente profundos (la recursion es por
//! nivel, asi que sin tope un arbol muy hondo podria desbordar el stack).

use std::path::{Path, PathBuf};

/// Directorios que nunca se recorren (ruido tipico de proyectos).
const SKIP_DIRS: &[&str] = &[".git", "target", "node_modules"];

/// Tope de archivos a juntar (corta arboles gigantes para que el switcher siga
/// respondiendo). Si se alcanza, la lista queda truncada (silenciosamente).
const MAX_FILES: usize = 5000;

/// Tope de profundidad de recursion (corta arboles patologicamente profundos
/// para no desbordar el stack). Al alcanzarlo se deja de bajar y el subarbol
/// mas hondo queda truncado (silenciosamente), igual que con `MAX_FILES`.
const MAX_DEPTH: usize = 32;

/// Descubre los archivos bajo `root` (recursivo, filtrado), como paths relativos
/// a `root`, ordenados alfabeticamente.
pub fn discover(root: impl AsRef<Path>) -> Vec<PathBuf> {
    let root = root.as_ref();
    let mut out = Vec::new();
    walk(root, root, 0, &mut out);
    out.sort();
    out
}

fn walk(root: &Path, dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    if out.len() >= MAX_FILES || depth >= MAX_DEPTH {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // dir ilegible: lo salteamos en silencio
    };
    for entry in entries.flatten() {
        if out.len() >= MAX_FILES {
            return;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        // Ocultos (dotfiles y dotdirs): fuera.
        if name.starts_with('.') {
            continue;
        }
        // `file_type` no sigue symlinks: un symlink a dir da `is_dir() == false`,
        // asi que se trata como hoja y no se recorre (evita loops).
        let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
        let path = entry.path();
        if is_dir {
            if SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            walk(root, &path, depth + 1, out);
        } else {
            let rel = path.strip_prefix(root).unwrap_or(&path).to_path_buf();
            out.push(rel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// Arma un arbol temporal, corre `discover` y limpia. Usa un nombre bajo el
    /// temp dir del SO sufijado con el PID (lo borra al entrar y salir para ser
    /// idempotente; el PID evita colisiones entre corridas concurrentes).
    #[test]
    fn descubre_filtrando_ocultos_y_dirs_de_ruido() {
        let root = std::env::temp_dir().join(format!(
            "typebar_files_test_discover_{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(root.join(".git")).unwrap();
        fs::create_dir_all(root.join("target")).unwrap();
        fs::write(root.join("a.md"), "a").unwrap();
        fs::write(root.join("sub/b.rs"), "b").unwrap();
        fs::write(root.join(".oculto"), "x").unwrap();
        fs::write(root.join(".git/config"), "x").unwrap();
        fs::write(root.join("target/out.bin"), "x").unwrap();

        let found: Vec<String> = discover(&root)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();

        let _ = fs::remove_dir_all(&root);

        // Solo los archivos "limpios", como paths relativos.
        assert!(found.contains(&"a.md".to_string()));
        assert!(found.contains(&"sub/b.rs".to_string()));
        // Ocultos y dirs de ruido quedaron afuera.
        assert!(!found.iter().any(|f| f.contains(".oculto")));
        assert!(!found.iter().any(|f| f.contains(".git")));
        assert!(!found.iter().any(|f| f.contains("target")));
        assert_eq!(found.len(), 2);
    }

    /// Un arbol mas profundo que `MAX_DEPTH` no debe paniquear (ni desbordar el
    /// stack): la recursion corta al alcanzar el tope y trunca el subarbol mas
    /// hondo. Verificamos que `discover` termina y que el archivo enterrado mas
    /// alla del tope queda afuera, mientras los superficiales se descubren.
    #[test]
    fn arbol_mas_profundo_que_el_tope_no_paniquea_y_trunca() {
        let root =
            std::env::temp_dir().join(format!("typebar_files_test_depth_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // Cadena de dirs anidados bastante mas profunda que MAX_DEPTH.
        let mut deep = root.clone();
        for i in 0..(MAX_DEPTH + 10) {
            deep = deep.join(format!("d{i}"));
        }
        fs::create_dir_all(&deep).unwrap();
        // Archivo superficial (dentro del tope) y archivo enterrado (mas alla).
        fs::write(root.join("superficial.md"), "x").unwrap();
        fs::write(deep.join("enterrado.md"), "x").unwrap();

        let found: Vec<String> = discover(&root)
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();

        let _ = fs::remove_dir_all(&root);

        // No paniqueo: el superficial esta, el enterrado quedo truncado.
        assert!(found.iter().any(|f| f == "superficial.md"));
        assert!(!found.iter().any(|f| f.contains("enterrado.md")));
    }
}
