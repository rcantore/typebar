//! Theme Engine: agrupa la paleta de colores del renderer en un struct `Theme`.
//!
//! Antes los colores vivian como constantes hardcodeadas al tope de `render.rs`
//! (Catppuccin Frappe). Sacarlos a un theme propio permite tener varias paletas
//! built-in seleccionables por nombre desde el config TOML (`[ui] theme = ...`).
//!
//! El renderer ya no conoce colores concretos: recibe un `&Theme` y lee sus
//! campos. La seleccion por nombre (`Theme::by_name`) cae a `frappe` ante un
//! nombre desconocido, igual que el resolver de keybindings cae a `standard`:
//! el editor tiene que arrancar siempre, nunca crashear por un theme invalido.

use ratatui::style::Color;

/// Nombre del theme por defecto. Se usa como fallback cuando el config pide un
/// theme desconocido y como default de la seccion `[ui]`.
pub const DEFAULT_THEME: &str = "frappe";

/// Paleta de colores del editor. Cada campo mapea 1:1 con las constantes que
/// antes vivian en `render.rs`. El tipo es `ratatui::style::Color` para que el
/// renderer use los valores directo, sin conversiones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Color del texto de un heading nivel 1 (`#`).
    pub heading_1: Color,
    /// Color del texto de un heading nivel 2 (`##`).
    pub heading_2: Color,
    /// Color del texto de headings de nivel 3+ (`###`, `####`, ...).
    pub heading_n: Color,
    /// Foreground del codigo inline (`` `code` ``).
    pub code_fg: Color,
    /// Background del codigo inline.
    pub code_bg: Color,
    /// Color de los marcadores/delimitadores de sintaxis dimmeados (`**`, `#`).
    pub marker: Color,
    /// Background del resalte de seleccion (solo pisa el `bg`, preserva el fg).
    pub selection_bg: Color,
    /// Background de una coincidencia de busqueda (todas las que matchean).
    pub search_match_bg: Color,
    /// Background de la coincidencia de busqueda ACTUAL (a la que salto el
    /// cursor), mas marcada que el resto para distinguirla.
    pub search_current_bg: Color,
}

impl Theme {
    /// Catppuccin Frappe: la paleta historica del spike, default del editor.
    /// Los valores RGB son EXACTAMENTE los que estaban hardcodeados en
    /// `render.rs`, asi el theme por defecto no cambia ni un pixel.
    pub fn frappe() -> Self {
        Theme {
            heading_1: Color::Rgb(0xca, 0x9e, 0xe6),    // mauve
            heading_2: Color::Rgb(0x99, 0xd1, 0xdb),    // sky
            heading_n: Color::Rgb(0xa6, 0xd1, 0x89),    // green
            code_fg: Color::Rgb(0xe7, 0x82, 0x84),      // red
            code_bg: Color::Rgb(0x41, 0x45, 0x59),      // surface0
            marker: Color::Rgb(0x73, 0x7a, 0x94),       // overlay0 (dimmeado)
            selection_bg: Color::Rgb(0x51, 0x57, 0x6d), // surface1 (resalte sutil)
            search_match_bg: Color::Rgb(0x8c, 0x73, 0x4a), // yellow apagado (match)
            search_current_bg: Color::Rgb(0xe5, 0xc8, 0x90), // yellow vivo (actual)
        }
    }

    /// Catppuccin Mocha: variante mas oscura/saturada. Sirve para demostrar que
    /// el motor de themes funciona (la seleccion por nombre cambia la paleta).
    pub fn mocha() -> Self {
        Theme {
            heading_1: Color::Rgb(0xcb, 0xa6, 0xf7),    // mauve
            heading_2: Color::Rgb(0x89, 0xdc, 0xeb),    // sky
            heading_n: Color::Rgb(0xa6, 0xe3, 0xa1),    // green
            code_fg: Color::Rgb(0xf3, 0x8b, 0xa8),      // red/pink
            code_bg: Color::Rgb(0x31, 0x32, 0x44),      // surface0
            marker: Color::Rgb(0x6c, 0x70, 0x86),       // overlay0 (dimmeado)
            selection_bg: Color::Rgb(0x45, 0x47, 0x5a), // surface1 (resalte sutil)
            search_match_bg: Color::Rgb(0x9a, 0x7e, 0x4e), // yellow apagado (match)
            search_current_bg: Color::Rgb(0xf9, 0xe2, 0xaf), // yellow vivo (actual)
        }
    }

    /// Resuelve un theme built-in por nombre. Cae a `frappe` ante un nombre
    /// desconocido: el config nunca debe poder romper el arranque del editor.
    pub fn by_name(name: &str) -> Theme {
        match name {
            "mocha" => Theme::mocha(),
            // `frappe` y cualquier otro nombre (incluido invalido) -> default.
            _ => Theme::frappe(),
        }
    }
}

/// El default del `Theme` es `frappe`, coherente con `DEFAULT_THEME`.
impl Default for Theme {
    fn default() -> Self {
        Theme::frappe()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn by_name_frappe_devuelve_la_paleta_historica() {
        // El default tiene que ser EXACTAMENTE la paleta del spike. Chequeamos
        // un par de colores clave (heading_1 y code_bg) ademas de la igualdad
        // total contra el constructor.
        let theme = Theme::by_name("frappe");
        assert_eq!(theme.heading_1, Color::Rgb(0xca, 0x9e, 0xe6));
        assert_eq!(theme.code_bg, Color::Rgb(0x41, 0x45, 0x59));
        assert_eq!(theme, Theme::frappe());
    }

    #[test]
    fn by_name_mocha_es_distinto_de_frappe() {
        // El segundo theme built-in tiene que existir y diferir del default,
        // probando que el motor efectivamente cambia de paleta.
        let mocha = Theme::by_name("mocha");
        assert_eq!(mocha, Theme::mocha());
        assert_ne!(mocha, Theme::frappe());
    }

    #[test]
    fn by_name_desconocido_cae_a_frappe() {
        // Un nombre que no existe no debe panickear: cae al default silencioso.
        assert_eq!(Theme::by_name("loquesea"), Theme::frappe());
        assert_eq!(Theme::by_name(""), Theme::frappe());
    }

    #[test]
    fn default_es_frappe() {
        assert_eq!(Theme::default(), Theme::frappe());
    }
}
