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
