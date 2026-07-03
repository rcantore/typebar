//! Backend de la GUI de escritorio de typebar (spike, Tauri v2).
//!
//! La logica de edicion y de markdown vive en `typebar-core`; aca solo esta la
//! capa fina de IPC que conecta el frontend estatico (ui/) con ese nucleo:
//! leer y escribir archivos, segmentar el documento en bloques renderizados
//! para el editor WYSIWYG, y abrir los dialogos nativos de archivo. Nada de
//! logica de markdown en este crate.

use std::fs;
use std::sync::{Mutex, mpsc};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, WebviewWindow};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogResult};

/// Estado compartido: si el documento tiene cambios sin guardar. El unico que
/// conoce el contenido en vivo es el frontend (el markdown vive en JS), asi que
/// es el frontend quien informa este flag via `set_dirty`. Lo lee el guard de
/// cierre en `on_window_event` para decidir si preguntar antes de cerrar.
struct DirtyFlag(Mutex<bool>);

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
/// nucleo. Lo mantiene por si hace falta un preview standalone; el editor
/// WYSIWYG por bloques usa `render_blocks`.
#[tauri::command]
fn render_html(markdown: String) -> Result<String, String> {
    Ok(typebar_core::export::to_html(&markdown, "typebar"))
}

/// Un bloque de nivel superior tal como lo consume el frontend: el `source`
/// crudo (para editar) y su `html` renderizado (para mostrar). Espeja a
/// `typebar_core::blocks::Block`.
#[derive(Serialize)]
struct GuiBlock {
    source: String,
    html: String,
}

/// Parte `markdown` en la columna de bloques del editor WYSIWYG. Es una capa
/// fina sobre `typebar_core::blocks::html_blocks`: la segmentacion y el render
/// los hace el nucleo, aca solo serializamos para el IPC.
///
/// Es el mismo concepto del "Nivel 2" de la TUI (linea activa cruda, resto
/// renderizado) subido de linea a bloque: la GUI muestra cada bloque renderizado
/// y swapea a `source` el que tiene el foco.
#[tauri::command]
fn render_blocks(markdown: String) -> Result<Vec<GuiBlock>, String> {
    let blocks = typebar_core::blocks::html_blocks(&markdown)
        .into_iter()
        .map(|b| GuiBlock {
            source: b.source,
            html: b.html,
        })
        .collect();
    Ok(blocks)
}

/// Un tramo estilizado del source de un bloque para el frontend. `start`/`end`
/// van en unidades UTF-16 (las que usa el `String` de JS) y `kind` es el nombre
/// estable de la categoria, que el frontend usa como clase CSS `md-<kind>`.
/// Espeja a `typebar_core::markdown::StyleSpan`.
#[derive(Serialize, Debug)]
struct GuiSpan {
    start: usize,
    end: usize,
    kind: &'static str,
}

/// Rango COMPLETO de un elemento revelable (con sus marcadores adentro), en
/// unidades UTF-16. Lo usa el "Nivel 2" del frontend para decidir que marcadores
/// revelar segun donde este el caret. Espeja a
/// `typebar_core::markdown::StyleElement`.
#[derive(Serialize, Debug)]
struct GuiElement {
    start: usize,
    end: usize,
    kind: &'static str,
}

/// Respuesta unificada de estilo para el frontend: los tramos (Nivel 1) y los
/// rangos de elementos (Nivel 2), en una sola llamada. Espeja a
/// `typebar_core::markdown::StyleInfo`.
#[derive(Serialize, Debug)]
struct GuiStyleInfo {
    spans: Vec<GuiSpan>,
    elements: Vec<GuiElement>,
}

/// Devuelve, en UNA llamada, los tramos de estilo y los rangos de elementos del
/// `source` markdown, para que el frontend pinte el bloque en edicion con el
/// markdown crudo estilizado (Nivel 1) y oculte/revele marcadores segun el caret
/// (Nivel 2). Capa fina sobre `typebar_core::markdown::style_info`: la logica de
/// markdown vive en el nucleo; aca solo serializamos para el IPC.
#[tauri::command]
fn style_info(source: String) -> GuiStyleInfo {
    let info = typebar_core::markdown::style_info(&source);
    GuiStyleInfo {
        spans: info
            .spans
            .into_iter()
            .map(|s| GuiSpan {
                start: s.start,
                end: s.end,
                kind: s.kind.as_str(),
            })
            .collect(),
        elements: info
            .elements
            .into_iter()
            .map(|e| GuiElement {
                start: e.start,
                end: e.end,
                kind: e.kind.as_str(),
            })
            .collect(),
    }
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

/// Actualiza el titulo de la ventana. Lo maneja Rust (no el JS) para no depender
/// del permiso `core:window:allow-set-title` del lado del frontend: el titulo
/// refleja el estado del documento (`nombre.md`, `● nombre.md` con cambios, o
/// "typebar" sin archivo) y lo arma el frontend, que solo pasa el texto ya hecho.
#[tauri::command]
fn update_title(window: WebviewWindow, title: String) -> Result<(), String> {
    window.set_title(&title).map_err(|e| e.to_string())
}

/// El frontend informa aca si hay cambios sin guardar. Lo guardamos en el estado
/// para que el guard de cierre (`on_window_event`) sepa si preguntar.
#[tauri::command]
fn set_dirty(state: tauri::State<'_, DirtyFlag>, dirty: bool) {
    if let Ok(mut flag) = state.0.lock() {
        *flag = dirty;
    }
}

/// Cierra la ventana de verdad (sin volver a disparar el guard de cierre, que
/// solo escucha `CloseRequested`). Lo usa el flujo "Guardar y cerrar": el JS
/// guarda y, si lo logra, llama a este comando para terminar de cerrar.
#[tauri::command]
fn close_window(window: WebviewWindow) -> Result<(), String> {
    window.destroy().map_err(|e| e.to_string())
}

/// Muestra un dialogo nativo de confirmacion (Descartar / Cancelar) y devuelve
/// `true` si el usuario acepta descartar los cambios. Lo usa el guard de "abrir
/// otro archivo" con cambios sin guardar. Mismo patron de canal que los pickers:
/// al ser comando async, el `recv` bloqueante corre en un worker del runtime y
/// no congela la ventana.
#[tauri::command]
async fn confirm_discard(app: AppHandle, message: String) -> Result<bool, String> {
    let (tx, rx) = mpsc::channel();
    app.dialog()
        .message(message)
        .title("Cambios sin guardar")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Descartar".to_string(),
            "Cancelar".to_string(),
        ))
        .show(move |accepted| {
            // `accepted` es true cuando se presiona el primer boton ("Descartar").
            let _ = tx.send(accepted);
        });
    rx.recv().map_err(|e| e.to_string())
}

/// Punto de entrada del backend: registra el plugin de dialogos y los comandos
/// IPC, y arranca la ventana definida en tauri.conf.json.
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(DirtyFlag(Mutex::new(false)))
        .invoke_handler(tauri::generate_handler![
            load_file,
            save_file,
            render_html,
            render_blocks,
            style_info,
            pick_open_path,
            pick_save_path,
            update_title,
            set_dirty,
            close_window,
            confirm_discard,
        ])
        // Guard de cierre: si el documento tiene cambios sin guardar, frenamos el
        // cierre de la ventana y preguntamos con un dialogo nativo de tres
        // opciones. "Guardar" no puede resolverse aca (el contenido vive en JS),
        // asi que emitimos un evento para que el frontend guarde y, si lo logra,
        // vuelva a cerrar via `close_window`. Es la variante mas simple que NO
        // pierde datos: "Cancelar" deja la ventana abierta con todo intacto y
        // "Descartar" es una decision explicita del usuario.
        .on_window_event(|window, event| {
            let tauri::WindowEvent::CloseRequested { api, .. } = event else {
                return;
            };
            let dirty = window
                .state::<DirtyFlag>()
                .0
                .lock()
                .map(|flag| *flag)
                .unwrap_or(false);
            if !dirty {
                return; // sin cambios: dejamos cerrar sin molestar.
            }
            api.prevent_close();
            let win = window.clone();
            window
                .dialog()
                .message("El documento tiene cambios sin guardar.")
                .title("Cerrar typebar")
                .buttons(MessageDialogButtons::YesNoCancelCustom(
                    "Guardar".to_string(),
                    "Descartar".to_string(),
                    "Cancelar".to_string(),
                ))
                .show_with_result(move |result| match result {
                    MessageDialogResult::Custom(label) if label == "Guardar" => {
                        // El frontend guarda y, si tiene exito, llama close_window.
                        let _ = win.emit("typebar://save-and-close", ());
                    }
                    MessageDialogResult::Custom(label) if label == "Descartar" => {
                        let _ = win.destroy();
                    }
                    // "Cancelar", el boton Cancel nativo o cerrar el dialogo: no
                    // cerramos y no perdemos nada.
                    _ => {}
                });
        })
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
    fn render_blocks_delega_en_el_nucleo() {
        // Debe segmentar por bloques usando el nucleo: un heading y un parrafo.
        let doc = "# Hola\n\nmundo\n".to_string();
        let blocks = render_blocks(doc).unwrap();
        assert_eq!(blocks.len(), 2, "esperaba dos bloques");
        assert!(blocks[0].html.contains("<h1>Hola</h1>"));
        assert!(blocks[1].html.contains("<p>mundo</p>"));
        // La concatenacion de sources reconstruye el documento (round-trip).
        let rejoined: String = blocks.iter().map(|b| b.source.as_str()).collect();
        assert_eq!(rejoined, "# Hola\n\nmundo\n");
    }

    #[test]
    fn style_info_delega_en_el_nucleo() {
        // Debe devolver los tramos del nucleo con su kind como string estable, y
        // los rangos de elementos en la misma llamada.
        let info = style_info("# Hola".to_string());
        assert!(
            info.spans.iter().any(|s| s.kind == "heading"),
            "esperaba un tramo heading; spans: {:?}",
            info.spans
        );
        assert!(info.spans.iter().any(|s| s.kind == "marker"));
        assert!(
            info.elements.iter().any(|e| e.kind == "heading"),
            "esperaba un elemento heading; elements: {:?}",
            info.elements
        );
    }

    #[test]
    fn style_info_texto_plano_no_da_tramos_ni_elementos() {
        let info = style_info("texto sin formato".to_string());
        assert!(info.spans.is_empty());
        assert!(info.elements.is_empty());
    }

    #[test]
    fn style_info_negrita_da_elemento_que_cubre_marcadores() {
        // "**hola**": el elemento Bold abarca los `**` (0..8) para que el Nivel 2
        // sepa a que elemento pertenecen los marcadores.
        let info = style_info("**hola**".to_string());
        let bold = info
            .elements
            .iter()
            .find(|e| e.kind == "bold")
            .expect("elemento bold");
        assert_eq!((bold.start, bold.end), (0, 8));
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
