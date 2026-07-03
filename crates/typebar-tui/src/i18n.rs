//! Internacionalizacion in-house: ingles y espanol, sin dependencias externas.
//!
//! Diseno: una tabla `match` que dado un `Locale` y una `Key` devuelve el texto
//! correspondiente. Los textos son literales y por eso `&'static str`, asi se
//! pueden insertar directo en cualquier API que pida un str con vida estatica
//! (ej `Hint`, que guarda labels de la toolbar).
//!
//! Idioma activo: un `OnceLock<Locale>` global seteado UNA vez en `main` (via
//! `init`). El resto del programa lee con `t(key)`, que delega en la tabla pura
//! `t_for(locale, key)`. Antes de `init` o en tests, queda el default (`Es`).
//!
//! Cada string que ve el usuario es una `Key` aca. Para mensajes con formato
//! (ej errores de config que muestran un path), exponemos helpers especificos
//! que devuelven `String` aplicando `format!` sobre la traduccion.
//!
//! Agregar un idioma o una key: 1) sumar la variante al enum correspondiente,
//! 2) agregar el arm en `t_for` para CADA locale (el compilador te avisa con
//! `match` no exhaustivo si te olvidas alguno).

use std::path::Path;
use std::sync::OnceLock;

/// Idiomas soportados por la UI. `En` es el default (convencion de proyectos
/// open-source); los usuarios hispanohablantes obtienen `Es` automaticamente
/// via `from_env` cuando el sistema esta en español (`$LANG=es_*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Locale {
    #[default]
    En,
    Es,
}

impl Locale {
    /// Parsea un nombre canonico (`"es"`, `"en"`, etc.). Acepta tambien locales
    /// estilo `LANG` (`"es_AR.UTF-8"` -> `Es`) mirando solo el prefijo de 2
    /// chars. `None` si no matchea ninguno conocido.
    pub fn from_str(s: &str) -> Option<Locale> {
        let head: String = s.trim().chars().take(2).collect::<String>().to_lowercase();
        match head.as_str() {
            "es" => Some(Locale::Es),
            "en" => Some(Locale::En),
            _ => None,
        }
    }

    /// Adivina el locale desde la variable de entorno `LANG`/`LC_ALL` con
    /// fallback al default (`En`). `"C"` o `"POSIX"` cuentan como desconocidos.
    pub fn from_env() -> Locale {
        std::env::var("LC_ALL")
            .or_else(|_| std::env::var("LANG"))
            .ok()
            .and_then(|s| Locale::from_str(&s))
            .unwrap_or_default()
    }
}

/// Cada string user-facing del editor: labels de la toolbar, nombres de modo,
/// minibuffer de los overlays y mensajes de error. Una variante por string;
/// agregar uno nuevo es agregar una variante aca y un arm en `t_for`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    // --- Toolbar: acciones de edicion --------------------------------------
    HintSave,
    HintSearch,
    HintReplace,
    HintUndo,
    /// Rehacer la ultima edicion deshecha (sobre todo para la paleta de
    /// comandos: los presets no exponen un hint de redo en la toolbar).
    HintRedo,
    HintYank,
    HintPaste,
    HintBold,
    HintItalic,
    HintCode,
    HintQuit,
    HintSaveQuit,
    HintDelete,
    HintInsert,
    HintVisual,
    HintNormal,
    HintLineStart,
    HintLineEnd,
    HintDocStart,
    HintDocEnd,
    /// Avanzar una pagina hacia arriba (para la paleta de comandos).
    HintPageUp,
    /// Avanzar una pagina hacia abajo (para la paleta de comandos).
    HintPageDown,
    /// Prefijo de chord de formato ("Formato…" / "Format…").
    HintFormatPrefix,
    /// Prefijo del submenu "view" ("Vista…" / "View…").
    HintViewPrefix,
    /// Toggle de zen/focus mode dentro del submenu "view".
    HintZen,
    /// Toggle del theme claro (Latte) dentro del submenu "view".
    HintLight,
    /// Toggle del modo whitepaper dentro del submenu "view".
    HintWhitepaper,
    /// Exportar el buffer actual a HTML (paleta de comandos).
    HintExportHtml,
    /// Nuevo archivo / buffer vacio ("Nuevo" / "New").
    HintNew,
    /// Cerrar el buffer activo ("Cerrar buffer" / "Close buffer").
    HintCloseBuffer,
    /// Switcher de archivos ("Ir a…" / "Go to…").
    HintSwitcher,
    /// Prompt del box del switcher de archivos ("ir a archivo:" / "go to file:").
    SwitcherPrompt,
    /// Estado vacio del switcher cuando nada matchea ("(sin resultados)" /
    /// "(no matches)").
    SwitcherEmpty,
    /// Paleta de comandos ("Comandos…" / "Commands…").
    HintPalette,
    /// Prompt del box de la paleta de comandos ("comando:" / "command:").
    PalettePrompt,

    // --- Status bar: nombres de modo (en MAYUSCULAS) -----------------------
    ModeNormal,
    ModeInsert,
    ModeVisual,

    // --- Minibuffer de los overlays ---------------------------------------
    MinibufferSearchPrompt,
    MinibufferReplacePrompt,
    /// Linea de ayuda al pie del overlay de reemplazo.
    MinibufferReplaceHelp,
    /// Prompt de confirmacion al cerrar un buffer con cambios sin guardar. Las
    /// teclas (`s`/`d`/`c`) son fijas; el texto entre corchetes las anuncia.
    ConfirmCloseUnsaved,

    // --- Mensajes de error (defaults) -------------------------------------
    UsingDefaults,
    Ignored,
}

/// Locale activo, seteado UNA vez al arrancar `main`. Antes (o en tests) cae a
/// `Locale::default()` (`En`). Usar `OnceLock` (no `RwLock`) deja explicito que
/// es inmutable post-init.
static LOCALE: OnceLock<Locale> = OnceLock::new();

/// Fija el locale activo para el resto del proceso. Llamarse a si misma o
/// llamarla dos veces es NO-OP (el segundo set se ignora silenciosamente, como
/// quiere `OnceLock::set`).
pub fn init(locale: Locale) {
    let _ = LOCALE.set(locale);
}

/// Locale efectivo en este momento.
pub fn locale() -> Locale {
    LOCALE.get().copied().unwrap_or_default()
}

/// Traduce una key al idioma activo. Conveniencia para el codigo de runtime.
pub fn t(key: Key) -> &'static str {
    t_for(locale(), key)
}

/// Tabla pura locale -> key -> texto. La unica fuente de verdad; los tests
/// trabajan contra esta funcion para no depender del estado global.
pub fn t_for(locale: Locale, key: Key) -> &'static str {
    match (locale, key) {
        // --- Toolbar ------------------------------------------------------
        (Locale::Es, Key::HintSave) => "Guardar",
        (Locale::En, Key::HintSave) => "Save",
        (Locale::Es, Key::HintSearch) => "Buscar",
        (Locale::En, Key::HintSearch) => "Search",
        (Locale::Es, Key::HintReplace) => "Reemplazar",
        (Locale::En, Key::HintReplace) => "Replace",
        (Locale::Es, Key::HintUndo) => "Deshacer",
        (Locale::En, Key::HintUndo) => "Undo",
        (Locale::Es, Key::HintRedo) => "Rehacer",
        (Locale::En, Key::HintRedo) => "Redo",
        (Locale::Es, Key::HintYank) => "Copiar",
        (Locale::En, Key::HintYank) => "Copy",
        (Locale::Es, Key::HintPaste) => "Pegar",
        (Locale::En, Key::HintPaste) => "Paste",
        (Locale::Es, Key::HintBold) => "Negrita",
        (Locale::En, Key::HintBold) => "Bold",
        (Locale::Es, Key::HintItalic) => "Italica",
        (Locale::En, Key::HintItalic) => "Italic",
        (Locale::Es, Key::HintCode) => "Codigo",
        (Locale::En, Key::HintCode) => "Code",
        (Locale::Es, Key::HintQuit) => "Salir",
        (Locale::En, Key::HintQuit) => "Quit",
        (Locale::Es, Key::HintSaveQuit) => "Guardar+Salir",
        (Locale::En, Key::HintSaveQuit) => "Save+Quit",
        (Locale::Es, Key::HintDelete) => "Borrar",
        (Locale::En, Key::HintDelete) => "Delete",
        (Locale::Es, Key::HintInsert) => "Insertar",
        (Locale::En, Key::HintInsert) => "Insert",
        (Locale::Es, Key::HintVisual) => "Visual",
        (Locale::En, Key::HintVisual) => "Visual",
        (Locale::Es, Key::HintNormal) => "Normal",
        (Locale::En, Key::HintNormal) => "Normal",
        (Locale::Es, Key::HintLineStart) => "Inicio linea",
        (Locale::En, Key::HintLineStart) => "Line start",
        (Locale::Es, Key::HintLineEnd) => "Fin linea",
        (Locale::En, Key::HintLineEnd) => "Line end",
        (Locale::Es, Key::HintDocStart) => "Inicio doc",
        (Locale::En, Key::HintDocStart) => "Doc start",
        (Locale::Es, Key::HintDocEnd) => "Fin doc",
        (Locale::En, Key::HintDocEnd) => "Doc end",
        (Locale::Es, Key::HintPageUp) => "Pagina arriba",
        (Locale::En, Key::HintPageUp) => "Page up",
        (Locale::Es, Key::HintPageDown) => "Pagina abajo",
        (Locale::En, Key::HintPageDown) => "Page down",
        (Locale::Es, Key::HintFormatPrefix) => "Formato…",
        (Locale::En, Key::HintFormatPrefix) => "Format…",
        (Locale::Es, Key::HintViewPrefix) => "Vista…",
        (Locale::En, Key::HintViewPrefix) => "View…",
        (Locale::Es, Key::HintZen) => "Zen",
        (Locale::En, Key::HintZen) => "Zen",
        (Locale::Es, Key::HintLight) => "Claro",
        (Locale::En, Key::HintLight) => "Light",
        (Locale::Es, Key::HintWhitepaper) => "Papel",
        (Locale::En, Key::HintWhitepaper) => "Paper",
        (Locale::Es, Key::HintExportHtml) => "Exportar HTML",
        (Locale::En, Key::HintExportHtml) => "Export HTML",
        (Locale::Es, Key::HintNew) => "Nuevo",
        (Locale::En, Key::HintNew) => "New",
        (Locale::Es, Key::HintCloseBuffer) => "Cerrar buffer",
        (Locale::En, Key::HintCloseBuffer) => "Close buffer",
        (Locale::Es, Key::HintSwitcher) => "Ir a…",
        (Locale::En, Key::HintSwitcher) => "Go to…",
        (Locale::Es, Key::SwitcherPrompt) => "ir a archivo:",
        (Locale::En, Key::SwitcherPrompt) => "go to file:",
        (Locale::Es, Key::SwitcherEmpty) => "(sin resultados)",
        (Locale::En, Key::SwitcherEmpty) => "(no matches)",
        (Locale::Es, Key::HintPalette) => "Comandos…",
        (Locale::En, Key::HintPalette) => "Commands…",
        (Locale::Es, Key::PalettePrompt) => "comando:",
        (Locale::En, Key::PalettePrompt) => "command:",

        // --- Status bar (nombres de modo) ---------------------------------
        (Locale::Es, Key::ModeNormal) => "NORMAL",
        (Locale::En, Key::ModeNormal) => "NORMAL",
        (Locale::Es, Key::ModeInsert) => "INSERTAR",
        (Locale::En, Key::ModeInsert) => "INSERT",
        (Locale::Es, Key::ModeVisual) => "VISUAL",
        (Locale::En, Key::ModeVisual) => "VISUAL",

        // --- Minibuffer ---------------------------------------------------
        (Locale::Es, Key::MinibufferSearchPrompt) => "buscar:",
        (Locale::En, Key::MinibufferSearchPrompt) => "find:",
        (Locale::Es, Key::MinibufferReplacePrompt) => "reemplazar:",
        (Locale::En, Key::MinibufferReplacePrompt) => "replace:",
        (Locale::Es, Key::MinibufferReplaceHelp) => "Tab cambia campo · Enter reemplaza todo",
        (Locale::En, Key::MinibufferReplaceHelp) => "Tab switches field · Enter replaces all",
        (Locale::Es, Key::ConfirmCloseUnsaved) => {
            "cambios sin guardar — [s] guardar y cerrar · [d] descartar · [c] cancelar"
        }
        (Locale::En, Key::ConfirmCloseUnsaved) => {
            "unsaved changes — [s] save & close · [d] discard · [c] cancel"
        }

        // --- Errores ------------------------------------------------------
        (Locale::Es, Key::UsingDefaults) => "usando defaults",
        (Locale::En, Key::UsingDefaults) => "using defaults",
        (Locale::Es, Key::Ignored) => "ignorado",
        (Locale::En, Key::Ignored) => "ignored",
    }
}

// --- Helpers para mensajes con formato --------------------------------------
//
// Cada error que muestra parametros (path, nombre de preset, etc.) tiene su
// propia funcion. Devuelven `String` ya formateado: las traducciones viven aca,
// los call sites del editor quedan limpios.

/// "typebar: no se pudo leer la config en {path}: {err}; usando defaults"
pub fn error_config_read_failed(path: &Path, err: impl std::fmt::Display) -> String {
    match locale() {
        Locale::Es => format!(
            "typebar: no se pudo leer la config en {}: {err}; {}",
            path.display(),
            t(Key::UsingDefaults),
        ),
        Locale::En => format!(
            "typebar: could not read config at {}: {err}; {}",
            path.display(),
            t(Key::UsingDefaults),
        ),
    }
}

/// "typebar: config invalida en {origen}: {err}; usando defaults"
pub fn error_config_invalid(origen: &str, err: impl std::fmt::Display) -> String {
    match locale() {
        Locale::Es => format!(
            "typebar: config invalida en {origen}: {err}; {}",
            t(Key::UsingDefaults),
        ),
        Locale::En => format!(
            "typebar: invalid config at {origen}: {err}; {}",
            t(Key::UsingDefaults),
        ),
    }
}

/// "typebar: preset desconocido en la config: {name}; usando {default}"
pub fn error_unknown_preset(name: &str, default: &str) -> String {
    match locale() {
        Locale::Es => {
            format!("typebar: preset desconocido en la config: {name:?}; usando {default}")
        }
        Locale::En => format!("typebar: unknown preset in config: {name:?}; using {default}"),
    }
}

/// "typebar: keybinding invalido {keys} -> {action}: {err}; ignorado"
pub fn error_invalid_keybinding(keys: &str, action: &str, err: impl std::fmt::Display) -> String {
    match locale() {
        Locale::Es => format!(
            "typebar: keybinding invalido {keys:?} -> {action:?}: {err}; {}",
            t(Key::Ignored),
        ),
        Locale::En => format!(
            "typebar: invalid keybinding {keys:?} -> {action:?}: {err}; {}",
            t(Key::Ignored),
        ),
    }
}

/// "exported to {path}" / "exportado a {path}": confirmacion (en la status bar y
/// en el stderr del export por CLI) de que el HTML se escribio.
pub fn exported_to(path: &Path) -> String {
    match locale() {
        Locale::Es => format!("exportado a {}", path.display()),
        Locale::En => format!("exported to {}", path.display()),
    }
}

/// "export failed: {err}" / "fallo el export: {err}": el export in-editor no
/// pudo escribir el archivo (permisos/disco). No tumba el editor: va al flash.
pub fn export_failed(err: impl std::fmt::Display) -> String {
    match locale() {
        Locale::Es => format!("fallo el export: {err}"),
        Locale::En => format!("export failed: {err}"),
    }
}

// --- Contador de palabras (status bar) --------------------------------------

/// Label "word(s)" / "palabra(s)" segun locale y singular/plural. Privado: el
/// formateo completo va por `words_count` / `words_count_selection`.
fn words_label_for(locale: Locale, n: usize) -> &'static str {
    match (locale, n) {
        (Locale::Es, 1) => "palabra",
        (Locale::Es, _) => "palabras",
        (Locale::En, 1) => "word",
        (Locale::En, _) => "words",
    }
}

/// "342 words" / "1 palabra": cantidad de palabras del documento.
pub fn words_count(n: usize) -> String {
    format!("{n} {}", words_label_for(locale(), n))
}

/// "12/342 words": palabras seleccionadas sobre el total. El plural lo decide el
/// total (es el numero que nombra el label).
pub fn words_count_selection(selected: usize, total: usize) -> String {
    format!("{selected}/{total} {}", words_label_for(locale(), total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_ingles() {
        // Convencion OSS: el default es ingles. Los usuarios hispanohablantes
        // obtienen `Es` automaticamente via `from_env` cuando $LANG arranca con
        // `es_*`. Esto no rompe el caso 'sistema en español': se usa Es igual.
        assert_eq!(Locale::default(), Locale::En);
    }

    #[test]
    fn from_str_parsea_canonicos_y_locales_completos() {
        assert_eq!(Locale::from_str("es"), Some(Locale::Es));
        assert_eq!(Locale::from_str("en"), Some(Locale::En));
        // Locales estilo LANG: solo importan los 2 primeros chars.
        assert_eq!(Locale::from_str("es_AR.UTF-8"), Some(Locale::Es));
        assert_eq!(Locale::from_str("en_US.UTF-8"), Some(Locale::En));
        // Mayusculas / espacios.
        assert_eq!(Locale::from_str("ES"), Some(Locale::Es));
        assert_eq!(Locale::from_str("  en  "), Some(Locale::En));
        // Desconocidos.
        assert_eq!(Locale::from_str("fr"), None);
        assert_eq!(Locale::from_str(""), None);
        assert_eq!(Locale::from_str("C"), None);
    }

    #[test]
    fn t_for_devuelve_textos_correctos() {
        assert_eq!(t_for(Locale::Es, Key::HintSave), "Guardar");
        assert_eq!(t_for(Locale::En, Key::HintSave), "Save");
        assert_eq!(t_for(Locale::Es, Key::HintFormatPrefix), "Formato…");
        assert_eq!(t_for(Locale::En, Key::HintFormatPrefix), "Format…");
        assert_eq!(
            t_for(Locale::Es, Key::MinibufferReplaceHelp),
            "Tab cambia campo · Enter reemplaza todo"
        );
        assert_eq!(
            t_for(Locale::En, Key::MinibufferReplaceHelp),
            "Tab switches field · Enter replaces all"
        );
    }

    #[test]
    fn words_label_singular_y_plural() {
        assert_eq!(words_label_for(Locale::En, 0), "words");
        assert_eq!(words_label_for(Locale::En, 1), "word");
        assert_eq!(words_label_for(Locale::En, 2), "words");
        assert_eq!(words_label_for(Locale::Es, 0), "palabras");
        assert_eq!(words_label_for(Locale::Es, 1), "palabra");
        assert_eq!(words_label_for(Locale::Es, 42), "palabras");
    }

    #[test]
    fn modos_son_iguales_en_ambos_locales() {
        // Los nombres de modo de Vim son convencion universal (no se traducen
        // NORMAL/VISUAL). El INSERT/INSERTAR si difiere por consistencia con
        // los labels de los hints.
        assert_eq!(t_for(Locale::Es, Key::ModeNormal), "NORMAL");
        assert_eq!(t_for(Locale::En, Key::ModeNormal), "NORMAL");
        assert_eq!(t_for(Locale::Es, Key::ModeVisual), "VISUAL");
        assert_eq!(t_for(Locale::En, Key::ModeVisual), "VISUAL");
    }
}
