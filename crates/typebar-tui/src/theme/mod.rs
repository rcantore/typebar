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
    /// Background de cada boton de la barra de atajos (toolbar). Un surface algo
    /// mas claro que el fondo del editor, para que los atajos lean como chrome.
    pub toolbar_button_bg: Color,
    /// Fondo de toda la superficie del editor. `None` en los themes oscuros: NO
    /// pintan fondo y dejan pasar el del terminal (lindo para transparencia/
    /// ricing). Los themes claros (Latte) lo setean para tener un paperwhite real;
    /// sin un fondo propio, sus colores quedarian sobre el fondo oscuro del
    /// terminal. Lo pinta el post-pass `apply_theme_fill` en `main`.
    pub background: Option<Color>,
    /// Color del texto del cuerpo (lo que no es heading/code/marker). `None` en
    /// los oscuros: el texto usa el foreground default del terminal (claro). Los
    /// claros lo setean (texto oscuro) para que se lea sobre su `background`.
    pub text: Option<Color>,
}

impl Theme {
    /// Catppuccin Frappe: la paleta historica del spike, default del editor.
    /// Los valores RGB son EXACTAMENTE los que estaban hardcodeados en
    /// `render.rs`, asi el theme por defecto no cambia ni un pixel.
    pub fn frappe() -> Self {
        Theme {
            heading_1: Color::Rgb(0xca, 0x9e, 0xe6),         // mauve
            heading_2: Color::Rgb(0x99, 0xd1, 0xdb),         // sky
            heading_n: Color::Rgb(0xa6, 0xd1, 0x89),         // green
            code_fg: Color::Rgb(0xe7, 0x82, 0x84),           // red
            code_bg: Color::Rgb(0x41, 0x45, 0x59),           // surface0
            marker: Color::Rgb(0x73, 0x7a, 0x94),            // overlay0 (dimmeado)
            selection_bg: Color::Rgb(0x57, 0x6a, 0xa6),      // azul-gris (resalte visible)
            search_match_bg: Color::Rgb(0x8c, 0x73, 0x4a),   // yellow apagado (match)
            search_current_bg: Color::Rgb(0xe5, 0xc8, 0x90), // yellow vivo (actual)
            toolbar_button_bg: Color::Rgb(0x51, 0x57, 0x6d), // surface1 (boton)
            background: None, // oscuro: deja pasar el fondo del terminal
            text: None,       // oscuro: usa el fg del terminal
        }
    }

    /// Catppuccin Mocha: variante mas oscura/saturada. Sirve para demostrar que
    /// el motor de themes funciona (la seleccion por nombre cambia la paleta).
    pub fn mocha() -> Self {
        Theme {
            heading_1: Color::Rgb(0xcb, 0xa6, 0xf7),         // mauve
            heading_2: Color::Rgb(0x89, 0xdc, 0xeb),         // sky
            heading_n: Color::Rgb(0xa6, 0xe3, 0xa1),         // green
            code_fg: Color::Rgb(0xf3, 0x8b, 0xa8),           // red/pink
            code_bg: Color::Rgb(0x31, 0x32, 0x44),           // surface0
            marker: Color::Rgb(0x6c, 0x70, 0x86),            // overlay0 (dimmeado)
            selection_bg: Color::Rgb(0x4e, 0x5c, 0x9c),      // azul-gris (resalte visible)
            search_match_bg: Color::Rgb(0x9a, 0x7e, 0x4e),   // yellow apagado (match)
            search_current_bg: Color::Rgb(0xf9, 0xe2, 0xaf), // yellow vivo (actual)
            toolbar_button_bg: Color::Rgb(0x45, 0x47, 0x5a), // surface1 (boton)
            background: None, // oscuro: deja pasar el fondo del terminal
            text: None,       // oscuro: usa el fg del terminal
        }
    }

    /// Catppuccin Latte: la variante CLARA de la familia. Espeja los mismos
    /// roles de paleta que `frappe` (heading_1 = mauve, code_bg = surface0,
    /// etc.), pero con los hex oficiales de Latte. Los tres backgrounds de
    /// resalte (`selection_bg`, `search_match_bg`, `search_current_bg`) en
    /// frappe son blends custom pensados para fondo oscuro, no roles puros: en
    /// un theme claro un fondo oscuro de resalte queda mal, asi que aca usamos
    /// surfaces/tints claros del propio Latte que cumplen el mismo proposito
    /// (pisar el bg dejando legible el texto oscuro encima).
    pub fn latte() -> Self {
        Theme {
            heading_1: Color::Rgb(0x88, 0x39, 0xef),         // mauve
            heading_2: Color::Rgb(0x04, 0xa5, 0xe5),         // sky
            heading_n: Color::Rgb(0x40, 0xa0, 0x2b),         // green
            code_fg: Color::Rgb(0xd2, 0x0f, 0x39),           // red
            code_bg: Color::Rgb(0xcc, 0xd0, 0xda),           // surface0
            marker: Color::Rgb(0x9c, 0xa0, 0xb0),            // overlay0 (dimmeado)
            selection_bg: Color::Rgb(0xac, 0xb0, 0xbe),      // surface2 (resalte claro)
            search_match_bg: Color::Rgb(0xdf, 0x8e, 0x1d),   // yellow (match)
            search_current_bg: Color::Rgb(0xfe, 0x64, 0x0b), // peach (actual, mas vivo)
            toolbar_button_bg: Color::Rgb(0xbc, 0xc0, 0xcc), // surface1 (boton)
            background: Some(Color::Rgb(0xdc, 0xe0, 0xe8)), // crust (off-white suave, menos blanco que base)
            text: Some(Color::Rgb(0x4c, 0x4f, 0x69)),       // text (oscuro, legible sobre el fondo)
        }
    }

    /// Theme del modo whitepaper: tinta negra sobre papel. A diferencia de Latte
    /// (claro pero con colores), este es MONOCROMO: headings, codigo y cuerpo van
    /// todos en el color de tinta (la jerarquia se mantiene por peso, porque el
    /// renderer aplica BOLD/ITALIC como modificadores, no como color). El codigo
    /// inline no lleva caja (su `code_bg` es el propio papel) y los markers quedan
    /// tenues. Pensado para escribir sin distraccion visual, no para leer sintaxis.
    pub fn paper() -> Self {
        let ink = Color::Rgb(0x4c, 0x4f, 0x69); // text Latte (tinta)
        let sheet = Color::Rgb(0xdc, 0xe0, 0xe8); // crust Latte (papel, off-white suave)
        Theme {
            heading_1: ink,
            heading_2: ink,
            heading_n: ink,
            code_fg: ink,
            code_bg: sheet, // sin caja: el codigo es tinta plana sobre el papel
            marker: Color::Rgb(0x9c, 0xa0, 0xb0), // overlay0: markers apenas visibles
            selection_bg: Color::Rgb(0xac, 0xb0, 0xbe), // surface2: resalte gris claro
            search_match_bg: Color::Rgb(0xdf, 0x8e, 0x1d), // yellow (la busqueda si destaca)
            search_current_bg: Color::Rgb(0xfe, 0x64, 0x0b), // peach
            toolbar_button_bg: Color::Rgb(0xbc, 0xc0, 0xcc), // (el chrome esta oculto en papel)
            background: Some(sheet),
            text: Some(ink),
        }
    }

    /// Dracula: uno de los themes oscuros mas populares. A diferencia de los
    /// Catppuccin oscuros (transparentes), los themes "de marca" que se suman aca
    /// (Dracula, Tokyo Night, Nord, Gruvbox, Solarized) SI pintan su fondo
    /// caracteristico: su identidad es justamente su paleta completa, y el theme
    /// picker se luce mostrando cada uno "de verdad". `apply_theme_fill` los pinta
    /// (fondo + texto) igual que a los claros.
    pub fn dracula() -> Self {
        Theme {
            heading_1: Color::Rgb(0xbd, 0x93, 0xf9),         // purple
            heading_2: Color::Rgb(0x8b, 0xe9, 0xfd),         // cyan (acento)
            heading_n: Color::Rgb(0x50, 0xfa, 0x7b),         // green
            code_fg: Color::Rgb(0xff, 0x55, 0x55),           // red
            code_bg: Color::Rgb(0x44, 0x47, 0x5a),           // current line
            marker: Color::Rgb(0x62, 0x72, 0xa4),            // comment
            selection_bg: Color::Rgb(0x44, 0x47, 0x5a),      // current line
            search_match_bg: Color::Rgb(0x72, 0x6c, 0x3f),   // yellow apagado (match)
            search_current_bg: Color::Rgb(0xf1, 0xfa, 0x8c), // yellow vivo (actual)
            toolbar_button_bg: Color::Rgb(0x44, 0x47, 0x5a), // current line (surface)
            background: Some(Color::Rgb(0x28, 0x2a, 0x36)),  // bg
            text: Some(Color::Rgb(0xf8, 0xf8, 0xf2)),        // foreground
        }
    }

    /// Tokyo Night: theme oscuro azulado muy popular (variante "night").
    pub fn tokyo_night() -> Self {
        Theme {
            heading_1: Color::Rgb(0xbb, 0x9a, 0xf7),         // magenta
            heading_2: Color::Rgb(0x7d, 0xcf, 0xff),         // cyan (acento)
            heading_n: Color::Rgb(0x9e, 0xce, 0x6a),         // green
            code_fg: Color::Rgb(0xf7, 0x76, 0x8e),           // red
            code_bg: Color::Rgb(0x29, 0x2e, 0x42),           // bg highlight
            marker: Color::Rgb(0x56, 0x5f, 0x89),            // comment
            selection_bg: Color::Rgb(0x33, 0x46, 0x7c),      // selection
            search_match_bg: Color::Rgb(0x6b, 0x5a, 0x34),   // yellow apagado
            search_current_bg: Color::Rgb(0xe0, 0xaf, 0x68), // yellow vivo
            toolbar_button_bg: Color::Rgb(0x29, 0x2e, 0x42), // surface
            background: Some(Color::Rgb(0x1a, 0x1b, 0x26)),  // bg
            text: Some(Color::Rgb(0xc0, 0xca, 0xf5)),        // fg
        }
    }

    /// Nord: theme oscuro frio (polar night + frost + aurora), muy usado.
    pub fn nord() -> Self {
        Theme {
            heading_1: Color::Rgb(0xb4, 0x8e, 0xad), // aurora purple (nord15)
            heading_2: Color::Rgb(0x88, 0xc0, 0xd0), // frost (nord8, acento)
            heading_n: Color::Rgb(0xa3, 0xbe, 0x8c), // aurora green (nord14)
            code_fg: Color::Rgb(0xbf, 0x61, 0x6a),   // aurora red (nord11)
            code_bg: Color::Rgb(0x3b, 0x42, 0x52),   // polar night (nord1)
            marker: Color::Rgb(0x4c, 0x56, 0x6a),    // polar night (nord3)
            selection_bg: Color::Rgb(0x43, 0x4c, 0x5e), // polar night (nord2)
            search_match_bg: Color::Rgb(0x7d, 0x6f, 0x4a), // yellow apagado
            search_current_bg: Color::Rgb(0xeb, 0xcb, 0x8b), // aurora yellow (nord13)
            toolbar_button_bg: Color::Rgb(0x43, 0x4c, 0x5e), // polar night (nord2)
            background: Some(Color::Rgb(0x2e, 0x34, 0x40)), // polar night (nord0)
            text: Some(Color::Rgb(0xd8, 0xde, 0xe9)), // snow storm (nord4)
        }
    }

    /// Gruvbox (variante oscura): calido, alto contraste, retro. Muy popular.
    pub fn gruvbox() -> Self {
        Theme {
            heading_1: Color::Rgb(0xd3, 0x86, 0x9b),         // purple
            heading_2: Color::Rgb(0x83, 0xa5, 0x98),         // blue (acento)
            heading_n: Color::Rgb(0xb8, 0xbb, 0x26),         // green
            code_fg: Color::Rgb(0xfb, 0x49, 0x34),           // red
            code_bg: Color::Rgb(0x3c, 0x38, 0x36),           // bg1
            marker: Color::Rgb(0x92, 0x83, 0x74),            // gray
            selection_bg: Color::Rgb(0x50, 0x49, 0x45),      // bg2
            search_match_bg: Color::Rgb(0x7c, 0x6f, 0x1a),   // yellow apagado
            search_current_bg: Color::Rgb(0xfa, 0xbd, 0x2f), // yellow vivo
            toolbar_button_bg: Color::Rgb(0x50, 0x49, 0x45), // bg2
            background: Some(Color::Rgb(0x28, 0x28, 0x28)),  // bg0
            text: Some(Color::Rgb(0xeb, 0xdb, 0xb2)),        // fg
        }
    }

    /// Solarized (variante oscura): la paleta clasica de Ethan Schoonover.
    pub fn solarized() -> Self {
        Theme {
            heading_1: Color::Rgb(0x6c, 0x71, 0xc4),         // violet
            heading_2: Color::Rgb(0x26, 0x8b, 0xd2),         // blue (acento)
            heading_n: Color::Rgb(0x85, 0x99, 0x00),         // green
            code_fg: Color::Rgb(0xdc, 0x32, 0x2f),           // red
            code_bg: Color::Rgb(0x07, 0x36, 0x42),           // base02
            marker: Color::Rgb(0x58, 0x6e, 0x75),            // base01
            selection_bg: Color::Rgb(0x0d, 0x4a, 0x57),      // base02 algo mas claro (resalte)
            search_match_bg: Color::Rgb(0x7a, 0x5f, 0x1a),   // yellow apagado
            search_current_bg: Color::Rgb(0xb5, 0x89, 0x00), // yellow vivo
            toolbar_button_bg: Color::Rgb(0x07, 0x36, 0x42), // base02
            background: Some(Color::Rgb(0x00, 0x2b, 0x36)),  // base03
            text: Some(Color::Rgb(0x83, 0x94, 0x96)),        // base0
        }
    }

    /// Resuelve un theme built-in por nombre. Cae a `frappe` ante un nombre
    /// desconocido: el config nunca debe poder romper el arranque del editor.
    pub fn by_name(name: &str) -> Theme {
        match name {
            "mocha" => Theme::mocha(),
            "latte" => Theme::latte(),
            "dracula" => Theme::dracula(),
            "tokyo-night" | "tokyonight" => Theme::tokyo_night(),
            "nord" => Theme::nord(),
            "gruvbox" => Theme::gruvbox(),
            "solarized" => Theme::solarized(),
            // `frappe` y cualquier otro nombre (incluido invalido) -> default.
            _ => Theme::frappe(),
        }
    }
}

/// Catalogo de themes que ofrece el theme picker en runtime, como `(id, nombre
/// visible)`. El `id` es el que entiende `by_name` y el que se persiste en el
/// config `[ui] theme`. Excluye `paper` a proposito: no es un theme suelto sino el
/// theme del MODO whitepaper (`^O W`), que orquesta ademas zen + columna centrada.
pub const PICKER_THEMES: &[(&str, &str)] = &[
    ("frappe", "Frappé"),
    ("mocha", "Mocha"),
    ("latte", "Latte"),
    ("dracula", "Dracula"),
    ("tokyo-night", "Tokyo Night"),
    ("nord", "Nord"),
    ("gruvbox", "Gruvbox"),
    ("solarized", "Solarized"),
];

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
    fn by_name_latte_es_el_theme_claro() {
        // El theme claro tiene que existir y diferir del default. Chequeamos un
        // campo distintivo: en frappe el `code_bg` es un surface oscuro
        // (#414559), mientras que en Latte es un surface claro (#ccd0da).
        let latte = Theme::by_name("latte");
        assert_eq!(latte, Theme::latte());
        assert_ne!(latte, Theme::frappe());
        assert_eq!(latte.code_bg, Color::Rgb(0xcc, 0xd0, 0xda));
        assert_eq!(latte.heading_1, Color::Rgb(0x88, 0x39, 0xef)); // mauve Latte
        // El claro pinta fondo y texto (paperwhite); los oscuros no (transparentes).
        assert_eq!(latte.background, Some(Color::Rgb(0xdc, 0xe0, 0xe8)));
        assert_eq!(latte.text, Some(Color::Rgb(0x4c, 0x4f, 0x69)));
        assert_eq!(Theme::frappe().background, None);
        assert_eq!(Theme::frappe().text, None);
    }

    #[test]
    fn paper_es_monocromo_tinta_sobre_papel() {
        // En el theme de papel headings, codigo y cuerpo van todos en el mismo
        // color de tinta (la jerarquia la da el peso/modificador, no el color), el
        // codigo no lleva caja (code_bg == fondo) y hay fondo y texto seteados.
        let p = Theme::paper();
        let ink = p.text.expect("el papel deberia tener color de tinta");
        assert_eq!(p.heading_1, ink);
        assert_eq!(p.heading_2, ink);
        assert_eq!(p.heading_n, ink);
        assert_eq!(p.code_fg, ink);
        assert_eq!(
            Some(p.code_bg),
            p.background,
            "el codigo no deberia tener caja: su bg es el papel"
        );
        assert!(p.background.is_some(), "el papel deberia pintar fondo");
        // Y los markers NO son tinta plena: quedan mas tenues que el cuerpo.
        assert_ne!(p.marker, ink, "los markers deberian quedar atenuados");
    }

    #[test]
    fn by_name_resuelve_los_themes_de_marca() {
        // Los themes populares que suma la card 237 resuelven por su id, difieren
        // del default y pintan su fondo (son opacos, a diferencia de los Catppuccin
        // oscuros que dejan pasar el fondo del terminal).
        for (id, ctor) in [
            ("dracula", Theme::dracula as fn() -> Theme),
            ("tokyo-night", Theme::tokyo_night),
            ("nord", Theme::nord),
            ("gruvbox", Theme::gruvbox),
            ("solarized", Theme::solarized),
        ] {
            assert_eq!(Theme::by_name(id), ctor(), "by_name({id}) deberia matchear");
            assert_ne!(ctor(), Theme::frappe(), "{id} deberia diferir del default");
            assert!(ctor().background.is_some(), "{id} deberia pintar fondo");
            assert!(ctor().text.is_some(), "{id} deberia pintar texto");
        }
        // Alias sin guion.
        assert_eq!(Theme::by_name("tokyonight"), Theme::tokyo_night());
    }

    #[test]
    fn picker_themes_todos_resuelven() {
        // Todo id del catalogo del picker tiene que resolver a un theme built-in
        // real (no caer al fallback por un id mal escrito) y no repetirse.
        use std::collections::HashSet;
        let mut ids = HashSet::new();
        for (id, display) in PICKER_THEMES {
            assert!(ids.insert(*id), "id repetido en PICKER_THEMES: {id}");
            assert!(!display.is_empty(), "{id} sin nombre visible");
            // frappe es el fallback; para el resto, un id que cae a frappe estaria
            // mal escrito. frappe se testea con su propio id.
            if *id != "frappe" {
                assert_ne!(
                    Theme::by_name(id),
                    Theme::frappe(),
                    "el id '{id}' del picker cae al fallback: no resuelve"
                );
            }
        }
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
