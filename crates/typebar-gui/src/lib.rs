//! Backend de la GUI de escritorio de typebar (spike, Tauri v2).
//!
//! La logica de edicion y de markdown vive en `typebar-core`; aca solo esta la
//! capa fina de IPC que conecta el frontend estatico (ui/) con ese nucleo:
//! leer y escribir archivos, renderizar el HTML del preview, y abrir los
//! dialogos nativos de archivo. Nada de logica de markdown en este crate.

use std::fs;
use std::sync::mpsc;

use tauri::AppHandle;
use tauri_plugin_dialog::DialogExt;

/// Extensiones de archivo que ofrecemos en los dialogos de abrir y guardar.
const MD_EXTENSIONS: &[&str] = &["md", "markdown", "mdown", "mkd", "txt"];

/// Lee el archivo de `path` y devuelve su contenido como texto UTF-8.
#[tauri::command]
fn load_file(path: String) -> Result<String, String> {
    fs::read_to_string(&path).map_err(|e| format!("No se pudo leer {path}: {e}"))
}

/// Escribe `contents` en `path`, creando o truncando el archivo.
#[tauri::command]
fn save_file(path: String, contents: String) -> Result<(), String> {
    fs::write(&path, contents).map_err(|e| format!("No se pudo guardar {path}: {e}"))
}

/// Renderiza `markdown` a un documento HTML standalone usando el export del
/// nucleo. El frontend extrae el `<body>` para el panel de preview.
#[tauri::command]
fn render_html(markdown: String) -> Result<String, String> {
    Ok(typebar_core::export::to_html(&markdown, "typebar"))
}

/// Abre el dialogo nativo para elegir un archivo a abrir. Devuelve la ruta
/// elegida, o `None` si el usuario cancela.
///
/// El dialogo es asincrono (callback en el hilo del event loop); bloqueamos
/// sobre un canal para exponer una API simple al frontend. Al ser un comando
/// async, la espera ocurre en un worker del runtime y no congela la ventana.
#[tauri::command]
async fn pick_open_path(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Markdown", MD_EXTENSIONS)
        .set_title("Abrir documento")
        .pick_file(move |picked| {
            let _ = tx.send(picked);
        });
    let picked = rx.recv().map_err(|e| e.to_string())?;
    Ok(picked.map(|fp| fp.to_string()))
}

/// Abre el dialogo nativo de guardar y devuelve la ruta de destino elegida, o
/// `None` si el usuario cancela. No escribe: eso lo hace `save_file`.
#[tauri::command]
async fn pick_save_path(app: AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .file()
        .add_filter("Markdown", MD_EXTENSIONS)
        .set_title("Guardar documento")
        .set_file_name("documento.md")
        .save_file(move |picked| {
            let _ = tx.send(picked);
        });
    let picked = rx.recv().map_err(|e| e.to_string())?;
    Ok(picked.map(|fp| fp.to_string()))
}

/// Punto de entrada del backend: registra el plugin de dialogos y los comandos
/// IPC, y arranca la ventana definida en tauri.conf.json.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            load_file,
            save_file,
            render_html,
            pick_open_path,
            pick_save_path,
        ])
        .run(tauri::generate_context!())
        .expect("error al arrancar la aplicacion typebar-gui");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_html_delega_en_el_nucleo() {
        // El comando debe devolver el HTML del nucleo, no reimplementar nada.
        let html = render_html("# Hola".to_string()).unwrap();
        assert!(html.contains("<h1>Hola</h1>"), "html: {html}");
    }

    #[test]
    fn load_file_falla_con_ruta_inexistente() {
        let err = load_file("/no/existe/typebar-xyz.md".to_string()).unwrap_err();
        assert!(err.contains("No se pudo leer"), "err: {err}");
    }

    #[test]
    fn save_y_load_hacen_roundtrip() {
        let dir = std::env::temp_dir();
        let path = dir.join("typebar-gui-roundtrip-test.md");
        let path_str = path.to_string_lossy().to_string();
        save_file(path_str.clone(), "contenido de prueba".to_string()).unwrap();
        let leido = load_file(path_str).unwrap();
        assert_eq!(leido, "contenido de prueba");
        let _ = std::fs::remove_file(path);
    }
}
