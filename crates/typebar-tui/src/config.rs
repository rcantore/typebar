//! Carga de configuracion de usuario desde un archivo TOML (esquema v1).
//!
//! El objetivo es que el usuario fije su preset de keybindings por defecto sin
//! tener que pasar `--keys` en cada invocacion. La precedencia final (resuelta
//! en `main`) es: flag CLI `--keys` > config file > default built-in.
//!
//! Decisiones de borde:
//! - Si el archivo no existe, NO es un error: se usan defaults en silencio. Es
//!   el caso comun (la mayoria de los usuarios nunca crea el config).
//! - Si el archivo existe pero esta mal formado o trae un preset invalido,
//!   avisamos por stderr y caemos al default en vez de crashear: el editor
//!   tiene que arrancar igual.
//!
//! La validacion del nombre de preset NO se duplica aca: se delega en
//! `keybinding::keymap_from_name`, que es la unica fuente de verdad de que
//! nombres existen.

use std::path::{Path, PathBuf};

use serde::Deserialize;

use crate::keybinding::keymap_from_name;
use crate::theme::DEFAULT_THEME;

/// Esquema v1 del archivo de configuracion.
#[derive(Debug, Deserialize, Default)]
pub struct Config {
    /// Seccion `[keybindings]`. Opcional: si falta, se usan los defaults de
    /// `KeybindingsConfig`.
    #[serde(default)]
    pub keybindings: KeybindingsConfig,
    /// Seccion `[ui]`. Opcional: si falta, se usan los defaults de `UiConfig`.
    #[serde(default)]
    pub ui: UiConfig,
}

/// Seccion `[keybindings]` del config.
#[derive(Debug, Deserialize, Default)]
pub struct KeybindingsConfig {
    /// Preset por defecto (`standard` | `vim` | `wordstar`). `None` cuando el
    /// usuario no fijo ninguno: en ese caso `main` deja el default built-in.
    pub preset: Option<String>,
    /// Overrides de teclas del usuario, como array de tablas `[[keybindings.bind]]`.
    /// Se aplican encima del preset (ver `crate::keybinding::CustomKeymap`). Cada
    /// entrada se valida en `main`; las invalidas se reportan y descartan sin
    /// abortar el arranque. Si falta, queda vacio (solo el preset base).
    #[serde(default)]
    pub bind: Vec<BindEntry>,
}

/// Una entrada del array `[[keybindings.bind]]`: la representacion cruda (sin
/// parsear) de un override de teclas. La validacion/traduccion a un binding real
/// la hace `crate::keybinding::parse_binding`.
#[derive(Debug, Deserialize)]
pub struct BindEntry {
    /// Secuencia de teclas, ej `"ctrl-s"` o `"ctrl-k ctrl-x"`.
    pub keys: String,
    /// Nombre de la accion en kebab-case, ej `"save"` o `"save-and-quit"`.
    pub action: String,
    /// Modo opcional (`normal` | `insert` | `visual`). Si falta, el binding
    /// aplica en todos los modos.
    #[serde(default)]
    pub mode: Option<String>,
}

/// Seccion `[ui]` del config: opciones de presentacion.
#[derive(Debug, Deserialize)]
pub struct UiConfig {
    /// Nombre del theme de colores (ver `crate::theme::Theme::by_name`). A
    /// diferencia del preset de keybindings, aca usamos `String` (no `Option`)
    /// con default `frappe`: `Theme::by_name` ya cae a `frappe` ante un nombre
    /// desconocido, asi que un theme invalido nunca rompe el arranque.
    #[serde(default = "default_theme")]
    pub theme: String,
    /// Idioma de la UI: `"es"` o `"en"`. `None` cuando el usuario no fijo
    /// ninguno: en ese caso `main` adivina desde `$LANG`/`$LC_ALL` (con fallback
    /// al default historico, `Es`).
    #[serde(default)]
    pub locale: Option<String>,
    /// Nivel WYSIWYG (`1` soft, `2` markers inline ocultos fuera de la linea
    /// activa). Default `2`. Cualquier otro valor se clampea a `2` en
    /// `resolved_wysiwyg_level()` para que un typo en el config no rompa el
    /// arranque.
    #[serde(default = "default_wysiwyg_level")]
    pub wysiwyg_level: u8,
    /// Captura del mouse. Default `false` (keyboard-first: deja la seleccion
    /// nativa del terminal intacta). Con `true` se habilita el click en la barra
    /// de tabs (y abre la puerta a mas interaccion con mouse a futuro).
    #[serde(default)]
    pub mouse: bool,
}

/// Default del campo `wysiwyg_level`: Nivel 2 (markers inline ocultos fuera
/// de la linea activa).
fn default_wysiwyg_level() -> u8 {
    2
}

impl UiConfig {
    /// Devuelve el nivel WYSIWYG valido: `1` o `2`. Cualquier otro valor se
    /// trata como `2` (default).
    pub fn resolved_wysiwyg_level(&self) -> u8 {
        match self.wysiwyg_level {
            1 => 1,
            _ => 2,
        }
    }
}

/// Default del campo `theme`: el theme por defecto del editor (`frappe`).
fn default_theme() -> String {
    DEFAULT_THEME.to_string()
}

/// El default de `UiConfig` reusa el default del campo para no duplicar el
/// nombre: si falta la seccion `[ui]`, queda el theme por defecto.
impl Default for UiConfig {
    fn default() -> Self {
        UiConfig {
            theme: default_theme(),
            locale: None,
            wysiwyg_level: default_wysiwyg_level(),
            mouse: false,
        }
    }
}

/// Resuelve el path del config file respetando `XDG_CONFIG_HOME` (via el crate
/// `dirs`, que ya aplica el fallback portable a `~/.config` en Unix y al dir
/// equivalente en otras plataformas). Devuelve `None` si no hay home conocido,
/// en cuyo caso `main` simplemente usa defaults.
pub fn config_path() -> Option<PathBuf> {
    dirs::config_dir().map(|dir| dir.join("typebar").join("config.toml"))
}

/// Carga la config desde `path`. Si el archivo no existe devuelve la config por
/// defecto en silencio. Si existe pero no se puede leer o parsear, avisa por
/// stderr y cae al default (nunca panickea).
pub fn load_from_path(path: &Path) -> Config {
    let raw = match std::fs::read_to_string(path) {
        Ok(raw) => raw,
        // Ausente es el caso esperado: defaults sin ruido. Otros errores de IO
        // (permisos, etc.) si los reportamos antes de caer al default.
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Config::default(),
        Err(err) => {
            eprintln!(
                "{}",
                typebar_core::i18n::error_config_read_failed(path, err)
            );
            return Config::default();
        }
    };
    parse_config(&raw, &path.display().to_string())
}

/// Parsea la config desde un string TOML. `origen` es solo para el mensaje de
/// error (un path o un nombre descriptivo). Aislamos el parseo del filesystem
/// para poder testearlo sin tocar el HOME real.
pub fn parse_config(raw: &str, origen: &str) -> Config {
    match toml::from_str::<Config>(raw) {
        Ok(config) => config,
        Err(err) => {
            eprintln!("{}", typebar_core::i18n::error_config_invalid(origen, err));
            Config::default()
        }
    }
}

/// Devuelve true si `name` es un preset de keybindings conocido. Reusa
/// `keymap_from_name` como unica fuente de verdad: ese resolver cae a
/// `standard` ante nombres desconocidos, asi que un nombre es valido sii el
/// preset resultante conserva el mismo nombre que pedimos.
pub fn is_known_preset(name: &str) -> bool {
    keymap_from_name(name).name() == name
}

/// Persiste el theme elegido en el theme picker en el config del usuario,
/// editando SOLO la clave `[ui] theme` y dejando el resto del archivo intacto
/// (keybindings, comentarios, formato, otras claves). Crea el archivo (y su dir)
/// si no existe. Best-effort: devuelve `Err` si no hay dir de config o falla el
/// IO (permisos), y el caller lo muestra en el flash sin romper nada.
pub fn persist_theme(id: &str) -> std::io::Result<PathBuf> {
    let path = config_path().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no hay directorio de config conocido",
        )
    })?;
    persist_theme_to(&path, id)?;
    Ok(path)
}

/// Nucleo de `persist_theme` parametrizado por `path` (asi se testea el ciclo
/// read-modify-write completo sin tocar el config real del usuario). Lee el
/// archivo (o arranca de vacio si no existe), le fija el theme y lo reescribe,
/// creando el directorio padre si hace falta.
fn persist_theme_to(path: &Path, id: &str) -> std::io::Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let updated = set_ui_theme(&existing, id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, updated)
}

/// Devuelve el contenido TOML de `raw` con `theme = "<id>"` bajo la seccion
/// `[ui]`, tocando lo MINIMO: si la clave ya existe en `[ui]` reemplaza su valor;
/// si existe la seccion `[ui]` pero no la clave, la inserta tras el header; si no
/// hay `[ui]`, la agrega al final. El resto del archivo (otras secciones,
/// comentarios, formato) queda igual. Trabaja por lineas para no depender de un
/// editor TOML format-preserving.
fn set_ui_theme(raw: &str, id: &str) -> String {
    let new_line = format!("theme = \"{id}\"");
    let mut out: Vec<String> = Vec::new();
    let mut in_ui = false;
    let mut ui_header_idx: Option<usize> = None;
    let mut replaced = false;

    for line in raw.lines() {
        let trimmed = line.trim_start();
        // Header de seccion: `[algo]` (no `[[algo]]`, que es array-of-tables).
        if trimmed.starts_with('[') && !trimmed.starts_with("[[") {
            in_ui = trimmed.trim_end() == "[ui]";
            if in_ui {
                ui_header_idx = Some(out.len());
            }
        }
        // Dentro de `[ui]`, una clave `theme =` (no comentada) se reemplaza.
        if in_ui
            && !replaced
            && !trimmed.starts_with('#')
            && trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == "theme")
        {
            // Preservar la indentacion original de la linea.
            let indent = &line[..line.len() - trimmed.len()];
            out.push(format!("{indent}{new_line}"));
            replaced = true;
            continue;
        }
        out.push(line.to_string());
    }

    if !replaced {
        match ui_header_idx {
            // Hay `[ui]` pero sin `theme`: insertar la clave justo tras el header.
            Some(idx) => out.insert(idx + 1, new_line),
            // No hay `[ui]`: agregar la seccion al final (con una linea en blanco
            // de separacion si el archivo no estaba vacio).
            None => {
                if !out.is_empty() && out.last().is_some_and(|l| !l.trim().is_empty()) {
                    out.push(String::new());
                }
                out.push("[ui]".to_string());
                out.push(new_line);
            }
        }
    }

    let mut result = out.join("\n");
    // Terminar en newline (convencion de archivos de texto).
    if !result.ends_with('\n') {
        result.push('\n');
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_toml_valido() {
        let raw = r#"
            [keybindings]
            preset = "vim"
        "#;
        let config = parse_config(raw, "<test>");
        assert_eq!(config.keybindings.preset.as_deref(), Some("vim"));
    }

    /// Cada resultado de `set_ui_theme` debe seguir parseando al theme esperado.
    fn theme_de(raw: &str) -> String {
        parse_config(raw, "<test>").ui.theme
    }

    #[test]
    fn set_theme_en_archivo_vacio_crea_la_seccion() {
        let out = set_ui_theme("", "dracula");
        assert_eq!(out, "[ui]\ntheme = \"dracula\"\n");
        assert_eq!(theme_de(&out), "dracula");
    }

    #[test]
    fn set_theme_reemplaza_la_clave_existente() {
        let raw = "[ui]\ntheme = \"frappe\"\nwysiwyg_level = 1\n";
        let out = set_ui_theme(raw, "nord");
        // Se cambio el theme y NO se toco wysiwyg_level.
        assert_eq!(theme_de(&out), "nord");
        assert!(
            out.contains("wysiwyg_level = 1"),
            "no debia tocar otras claves"
        );
        // Una sola linea de theme (no duplico).
        assert_eq!(out.matches("theme =").count(), 1);
    }

    #[test]
    fn set_theme_inserta_en_ui_sin_theme() {
        let raw = "[ui]\nwysiwyg_level = 2\n";
        let out = set_ui_theme(raw, "gruvbox");
        assert_eq!(theme_de(&out), "gruvbox");
        assert!(out.contains("wysiwyg_level = 2"));
    }

    #[test]
    fn set_theme_preserva_otras_secciones_y_agrega_ui() {
        // Con [keybindings] pero sin [ui]: se agrega [ui] sin tocar keybindings.
        let raw = "[keybindings]\npreset = \"vim\"\n";
        let out = set_ui_theme(raw, "tokyo-night");
        let config = parse_config(&out, "<test>");
        assert_eq!(config.ui.theme, "tokyo-night");
        assert_eq!(config.keybindings.preset.as_deref(), Some("vim"));
        assert!(out.contains("[ui]"));
    }

    #[test]
    fn set_theme_no_matchea_una_linea_comentada() {
        // Un `# theme = ...` comentado NO cuenta: se inserta la clave real y el
        // comentario queda intacto.
        let raw = "[ui]\n# theme = \"frappe\"\n";
        let out = set_ui_theme(raw, "solarized");
        assert_eq!(theme_de(&out), "solarized");
        assert!(
            out.contains("# theme = \"frappe\""),
            "el comentario debia quedar"
        );
    }

    #[test]
    fn set_theme_solo_toca_la_clave_theme_de_la_seccion_ui() {
        // Un `theme` en OTRA seccion no debe confundirse con el de [ui].
        let raw = "[otra]\ntheme = \"x\"\n\n[ui]\ntheme = \"frappe\"\n";
        let out = set_ui_theme(raw, "mocha");
        assert!(
            out.contains("[otra]\ntheme = \"x\""),
            "no debia tocar [otra]"
        );
        assert_eq!(theme_de(&out), "mocha");
    }

    #[test]
    fn persist_y_recargar_devuelve_el_theme_guardado() {
        // El escenario del usuario: cambiar el theme, "salir" y volver a cargar el
        // config debe devolver el theme elegido (persiste el ciclo completo). Se
        // guarda en un archivo temporal propio para no tocar el config real.
        let dir = std::env::temp_dir().join("typebar-test-persist-theme");
        let _ = std::fs::remove_dir_all(&dir); // limpiar restos de corridas previas
        let path = dir.join("config.toml");

        // Sin archivo previo: se crea con el theme.
        persist_theme_to(&path, "nord").unwrap();
        assert_eq!(load_from_path(&path).ui.theme, "nord");

        // Segundo cambio: reemplaza (no duplica) y sigue cargando bien.
        persist_theme_to(&path, "dracula").unwrap();
        assert_eq!(load_from_path(&path).ui.theme, "dracula");
        assert_eq!(
            std::fs::read_to_string(&path)
                .unwrap()
                .matches("theme =")
                .count(),
            1
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn persist_preserva_keybindings_del_usuario() {
        // Persistir el theme NO debe pisar la config de keybindings del usuario.
        let dir = std::env::temp_dir().join("typebar-test-persist-kb");
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.toml");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, "[keybindings]\npreset = \"vim\"\n").unwrap();

        persist_theme_to(&path, "gruvbox").unwrap();
        let config = load_from_path(&path);
        assert_eq!(config.ui.theme, "gruvbox");
        assert_eq!(config.keybindings.preset.as_deref(), Some("vim"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn toml_sin_seccion_keybindings_usa_default() {
        // Un archivo vacio es valido: la seccion es opcional.
        let config = parse_config("", "<test>");
        assert_eq!(config.keybindings.preset, None);
        // Sin `[[keybindings.bind]]` la lista de overrides queda vacia.
        assert!(config.keybindings.bind.is_empty());
    }

    #[test]
    fn parsea_array_de_bindings() {
        let raw = r#"
            [keybindings]
            preset = "vim"

            [[keybindings.bind]]
            keys = "ctrl-s"
            action = "save"

            [[keybindings.bind]]
            keys = "ctrl-k ctrl-x"
            action = "save-and-quit"
            mode = "normal"
        "#;
        let config = parse_config(raw, "<test>");
        assert_eq!(config.keybindings.bind.len(), 2);
        assert_eq!(config.keybindings.bind[0].keys, "ctrl-s");
        assert_eq!(config.keybindings.bind[0].action, "save");
        assert_eq!(config.keybindings.bind[0].mode, None);
        assert_eq!(config.keybindings.bind[1].mode.as_deref(), Some("normal"));
    }

    #[test]
    fn parsea_seccion_ui_theme() {
        // La seccion `[ui]` con `theme` se parsea al campo correspondiente.
        let raw = r#"
            [ui]
            theme = "mocha"
        "#;
        let config = parse_config(raw, "<test>");
        assert_eq!(config.ui.theme, "mocha");
    }

    #[test]
    fn toml_sin_seccion_ui_usa_theme_default() {
        // Sin `[ui]`, el theme cae al default (`frappe`), no queda vacio.
        let config = parse_config("", "<test>");
        assert_eq!(config.ui.theme, DEFAULT_THEME);
        assert_eq!(config.ui.theme, "frappe");
    }

    #[test]
    fn wysiwyg_level_default_es_2() {
        // Sin `[ui]` o sin `wysiwyg_level`, el default es Nivel 2 (markers
        // inline ocultos fuera de la linea activa).
        let config = parse_config("", "<test>");
        assert_eq!(config.ui.resolved_wysiwyg_level(), 2);
    }

    #[test]
    fn wysiwyg_level_1_explicito() {
        let raw = r#"
            [ui]
            wysiwyg_level = 1
        "#;
        let config = parse_config(raw, "<test>");
        assert_eq!(config.ui.resolved_wysiwyg_level(), 1);
    }

    #[test]
    fn wysiwyg_level_invalido_cae_a_2() {
        // Un valor fuera de {1, 2} no rompe: cae al default Nivel 2.
        let raw = r#"
            [ui]
            wysiwyg_level = 9
        "#;
        let config = parse_config(raw, "<test>");
        assert_eq!(config.ui.resolved_wysiwyg_level(), 2);
    }

    #[test]
    fn toml_invalido_cae_a_default_sin_panic() {
        // Sintaxis rota: no debe panickear, devuelve la config por defecto.
        let config = parse_config("esto no es = = toml [", "<test>");
        assert_eq!(config.keybindings.preset, None);
    }

    #[test]
    fn archivo_ausente_devuelve_default() {
        // Un path que no existe no es error: defaults en silencio.
        let path = Path::new("/typebar/ruta/que/no/existe/config.toml");
        let config = load_from_path(path);
        assert_eq!(config.keybindings.preset, None);
    }

    #[test]
    fn carga_desde_archivo_real() {
        // Escribimos un config temporal y lo leemos via load_from_path, sin
        // depender del HOME del sistema.
        let dir = std::env::temp_dir().join(format!("typebar-cfg-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, "[keybindings]\npreset = \"wordstar\"\n").unwrap();

        let config = load_from_path(&path);
        assert_eq!(config.keybindings.preset.as_deref(), Some("wordstar"));

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn presets_conocidos_y_desconocidos() {
        assert!(is_known_preset("standard"));
        assert!(is_known_preset("vim"));
        assert!(is_known_preset("wordstar"));
        assert!(!is_known_preset("loquesea"));
    }
}
