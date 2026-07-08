//! Paleta de comandos estilo M-x (Emacs) / Command Palette: un overlay que
//! fuzzy-filtra los comandos del editor por su nombre legible y ejecuta el
//! elegido. Vive aparte (como el switcher) porque opera a nivel del loop
//! (`run`): al aceptar, NO toca el documento directamente sino que devuelve el
//! `Action` elegido, que `run` despacha por el mismo camino que el keymap.
//!
//! El catalogo de comandos (`CATALOG`) es la lista curada de `Action` "de
//! comando", AGRUPADA en secciones (archivo, edicion, formato, vista, navegacion,
//! modo): los utiles desde una paleta (guardar, buscar, formato, navegacion,
//! etc.). Quedan EXCLUIDOS los de tipeo/cursor (InsertChar, Backspace,
//! Cursor*/Select*, ...) que no tienen sentido invocar por nombre, y el propio
//! `OpenPalette` (para no recursar). Con query vacia la paleta muestra las
//! secciones con headers; al filtrar cae a una lista plana rankeada. El nombre
//! visible sale de las keys i18n; el atajo se descubre PROBANDO el keymap
//! (`shortcut_map`), asi refleja el binding real de cada comando (preset +
//! remapeos), incluidos los que no viven en la toolbar.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::keybinding::{Action, Keymap};
use crate::switcher::{
    dim_area, picker_block, picker_content, picker_popup, picker_prompt, scroll_window,
};
use crate::theme::Theme;
use typebar_core::fuzzy;
use typebar_core::i18n::{self, Key};

/// Que debe hacer `run` tras pasarle una tecla a la paleta.
pub enum PaletteOutcome {
    /// Seguir abierta (siguio tipeando o navegando).
    Stay,
    /// Cerrar sin elegir (Esc).
    Cancel,
    /// Ejecutar este `Action` y cerrar (Enter sobre un resultado). `run` lo
    /// despacha por el mismo camino que un action resuelto por el keymap.
    Accept(Action),
}

/// Catalogo de comandos de la paleta, AGRUPADO en secciones. Cada seccion es
/// `(key i18n del header, comandos en orden)`. El orden define la vista por
/// defecto (query vacia, agrupada) y el desempate del ranking. NO incluye
/// `OpenPalette` (evita recursar) ni los de tipeo/cursor (no tienen sentido
/// invocarse por nombre).
const CATALOG: &[(Key, &[(Action, Key)])] = &[
    (
        Key::SectionFile,
        &[
            (Action::Save, Key::HintSave),
            (Action::SaveAndQuit, Key::HintSaveQuit),
            (Action::Quit, Key::HintQuit),
            (Action::NewBuffer, Key::HintNew),
            (Action::OpenSwitcher, Key::HintSwitcher),
            (Action::CloseBuffer, Key::HintCloseBuffer),
            (Action::ExportHtml, Key::HintExportHtml),
        ],
    ),
    (
        Key::SectionEdit,
        &[
            (Action::Search, Key::HintSearch),
            (Action::Replace, Key::HintReplace),
            (Action::Undo, Key::HintUndo),
            (Action::Redo, Key::HintRedo),
            (Action::Yank, Key::HintYank),
            (Action::Paste, Key::HintPaste),
        ],
    ),
    (
        Key::SectionFormat,
        &[
            (Action::ToggleBold, Key::HintBold),
            (Action::ToggleItalic, Key::HintItalic),
            (Action::ToggleCode, Key::HintCode),
        ],
    ),
    (
        Key::SectionView,
        &[
            (Action::OpenThemePicker, Key::HintTheme),
            (Action::ToggleZen, Key::HintZen),
            (Action::ToggleWhitepaper, Key::HintWhitepaper),
            (Action::CycleKeymapPreset, Key::HintCycleKeymap),
        ],
    ),
    (
        Key::SectionNav,
        &[
            (Action::LineStart, Key::HintLineStart),
            (Action::LineEnd, Key::HintLineEnd),
            (Action::DocStart, Key::HintDocStart),
            (Action::DocEnd, Key::HintDocEnd),
            (Action::PageUp, Key::HintPageUp),
            (Action::PageDown, Key::HintPageDown),
        ],
    ),
    (
        Key::SectionMode,
        &[
            (Action::EnterInsert, Key::HintInsert),
            (Action::EnterNormal, Key::HintNormal),
            (Action::EnterVisual, Key::HintVisual),
        ],
    ),
];

/// Una entrada de la paleta ya resuelta: el `Action` a ejecutar, su nombre
/// visible (label i18n) y el atajo actual segun el keymap (vacio si no tiene).
struct Command {
    action: Action,
    name: String,
    /// Atajo actual del comando (ej `^S`, `^K X`), o vacio si el preset no lo
    /// bindea. Se muestra alineado a la derecha en el render.
    shortcut: String,
}

/// Una fila del display de la paleta. Con query vacia se intercalan `Header` y
/// `Spacer` entre los `Command` (vista agrupada por seccion); al filtrar, el
/// display es solo `Command` rankeados. Solo los `Command` son navegables.
enum Row {
    /// Header de seccion (no navegable): el texto ya viene en MAYUSCULA.
    Header(String),
    /// Espaciador entre secciones (no navegable).
    Spacer,
    /// Un comando: indice en `commands` y el match del fuzzy (`None` en la vista
    /// agrupada, donde no se resalta nada).
    Command(usize, Option<fuzzy::FuzzyMatch>),
}

/// Estado de la paleta: el catalogo resuelto (comandos + rangos de seccion), el
/// texto tipeado, el display actual (filas a dibujar) y el seleccionado.
pub struct Palette {
    /// Comandos disponibles (catalogo resuelto contra el keymap activo, aplanado
    /// en orden de `CATALOG`).
    commands: Vec<Command>,
    /// Secciones paralelas a `CATALOG`: `(header en mayuscula, start, len)`, donde
    /// `commands[start..start+len]` son los comandos de esa seccion.
    sections: Vec<(String, usize, usize)>,
    /// Texto tipeado.
    query: String,
    /// Filas a mostrar: agrupadas (headers + comandos + espaciadores) con query
    /// vacia, o el ranking plano al filtrar.
    display: Vec<Row>,
    /// Indice EN `display` del seleccionado (siempre una fila `Command`).
    selected: usize,
}

impl Palette {
    /// Construye la paleta resolviendo el catalogo contra `keymap`: cada comando
    /// toma su nombre i18n y su atajo actual. El atajo se descubre PROBANDO el
    /// keymap (`shortcut_map`), no leyendo la toolbar: asi refleja el binding real
    /// y activo (preset + remapeos) de TODO comando, incluidos los que no viven en
    /// la barra (ir a archivo, undo, formato...). La query arranca vacia.
    pub fn new(keymap: &dyn Keymap) -> Self {
        let shortcuts = crate::keybinding::shortcut_map(keymap);
        let mut commands: Vec<Command> = Vec::new();
        let mut sections: Vec<(String, usize, usize)> = Vec::new();
        for (skey, cmds) in CATALOG {
            let start = commands.len();
            for &(action, key) in *cmds {
                commands.push(Command {
                    action,
                    name: i18n::t(key).to_string(),
                    shortcut: shortcuts
                        .iter()
                        .find(|(a, _)| *a == action)
                        .map(|(_, keys)| keys.clone())
                        .unwrap_or_default(),
                });
            }
            sections.push((i18n::t(*skey).to_uppercase(), start, cmds.len()));
        }
        let mut p = Palette {
            commands,
            sections,
            query: String::new(),
            display: Vec::new(),
            selected: 0,
        };
        p.recompute();
        p
    }

    /// Reconstruye el display y reposiciona la seleccion al primer comando. Con
    /// query vacia arma la vista AGRUPADA (header + comandos por seccion, con un
    /// espaciador entre secciones); con query, el ranking plano del fuzzy.
    fn recompute(&mut self) {
        self.display.clear();
        if self.query.is_empty() {
            for (i, (header, start, len)) in self.sections.iter().enumerate() {
                // Espaciador entre secciones (no antes de la primera).
                if i > 0 {
                    self.display.push(Row::Spacer);
                }
                self.display.push(Row::Header(header.clone()));
                for ci in *start..(*start + *len) {
                    self.display.push(Row::Command(ci, None));
                }
            }
        } else {
            let refs: Vec<&str> = self.commands.iter().map(|c| c.name.as_str()).collect();
            for (ci, m) in fuzzy::rank(&self.query, &refs) {
                self.display.push(Row::Command(ci, Some(m)));
            }
        }
        // La seleccion arranca en el primer comando (saltando el header inicial).
        self.selected = self.command_rows().first().copied().unwrap_or(0);
    }

    /// Indices EN `display` de las filas navegables (los `Command`), en orden.
    fn command_rows(&self) -> Vec<usize> {
        self.display
            .iter()
            .enumerate()
            .filter(|(_, r)| matches!(r, Row::Command(..)))
            .map(|(i, _)| i)
            .collect()
    }

    /// Mueve la seleccion `delta` comandos, SALTEANDO headers y espaciadores, con
    /// clamp a los limites.
    fn move_selection(&mut self, delta: isize) {
        let rows = self.command_rows();
        if rows.is_empty() {
            return;
        }
        let cur = rows.iter().position(|&i| i == self.selected).unwrap_or(0);
        let next = (cur as isize + delta).clamp(0, rows.len() as isize - 1) as usize;
        self.selected = rows[next];
    }

    /// Procesa una tecla y dice que hacer. Tipear/borrar refiltra; flechas (o
    /// Ctrl-N/Ctrl-P) navegan; Enter ejecuta el seleccionado; Esc cancela.
    pub fn handle_key(&mut self, key: KeyEvent) -> PaletteOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => PaletteOutcome::Cancel,
            KeyCode::Enter => match self.display.get(self.selected) {
                Some(Row::Command(ci, _)) => PaletteOutcome::Accept(self.commands[*ci].action),
                _ => PaletteOutcome::Cancel,
            },
            KeyCode::Down => {
                self.move_selection(1);
                PaletteOutcome::Stay
            }
            KeyCode::Up => {
                self.move_selection(-1);
                PaletteOutcome::Stay
            }
            KeyCode::Char('n') if ctrl => {
                self.move_selection(1);
                PaletteOutcome::Stay
            }
            KeyCode::Char('p') if ctrl => {
                self.move_selection(-1);
                PaletteOutcome::Stay
            }
            KeyCode::Backspace => {
                self.query.pop();
                self.recompute();
                PaletteOutcome::Stay
            }
            KeyCode::Char(c) if !ctrl => {
                self.query.push(c);
                self.recompute();
                PaletteOutcome::Stay
            }
            _ => PaletteOutcome::Stay,
        }
    }

    /// El texto tipeado (para el prompt del box).
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Cantidad de COMANDOS que matchean la query actual (sin contar headers ni
    /// espaciadores). Es el numero que muestra el prompt.
    pub fn result_count(&self) -> usize {
        self.display
            .iter()
            .filter(|r| matches!(r, Row::Command(..)))
            .count()
    }

    /// Cantidad total de FILAS del display (comandos + headers + espaciadores). Es
    /// lo que hay que dimensionar en el box (a diferencia de `result_count`, que
    /// solo cuenta comandos para el prompt).
    fn display_len(&self) -> usize {
        self.display.len()
    }

    /// Lineas de resultados a dibujar (hasta `max_rows`), con scroll para mantener
    /// visible el seleccionado, los chars del nombre que matchearon acentuados, el
    /// atajo actual alineado a la derecha y la fila seleccionada marcada (prefijo
    /// `>` y reverse). `width` es el ancho util del box para alinear el atajo.
    pub fn result_lines(&self, theme: &Theme, max_rows: usize, width: usize) -> Vec<Line<'static>> {
        // Ventana de scroll sobre TODAS las filas del display (comandos, headers y
        // espaciadores), para que el seleccionado siempre entre en `max_rows`.
        let std::ops::Range { start, end } =
            scroll_window(self.selected, self.display.len(), max_rows);
        if start == end {
            return Vec::new();
        }

        let accent = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::BOLD);
        let normal = Style::default();
        // El atajo va dimmeado (es secundario al nombre del comando).
        let shortcut_style = Style::default().add_modifier(Modifier::DIM);
        // Header de seccion: tenue y en negrita, para que ordene sin competir con
        // los comandos.
        let header_style = Style::default()
            .fg(theme.marker)
            .add_modifier(Modifier::BOLD);

        let mut lines = Vec::with_capacity(end - start);
        for ri in start..end {
            let (ci, m) = match &self.display[ri] {
                Row::Spacer => {
                    lines.push(Line::from(""));
                    continue;
                }
                Row::Header(h) => {
                    lines.push(Line::from(Span::styled(h.clone(), header_style)));
                    continue;
                }
                Row::Command(ci, m) => (*ci, m),
            };
            let cmd = &self.commands[ci];
            let selected = ri == self.selected;

            // Un prefijo marca el seleccionado (se nota aunque el terminal no
            // pinte el reverse).
            let marker = if selected { "> " } else { "  " };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(marker.to_string(), normal)];
            // Nombre char a char: acento si el indice matcheo (solo hay match al
            // filtrar; en la vista agrupada `m` es None y no se resalta nada).
            for (i, ch) in cmd.name.chars().enumerate() {
                let matched = m.as_ref().is_some_and(|mm| mm.indices.contains(&i));
                let mut style = if matched { accent } else { normal };
                if selected {
                    style = style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(ch.to_string(), style));
            }
            // Atajo alineado a la derecha: rellenamos con espacios hasta dejar el
            // atajo pegado al borde derecho del box. El ancho ya usado es el del
            // marcador (2) mas el nombre.
            if !cmd.shortcut.is_empty() {
                let used = 2 + cmd.name.chars().count();
                let shortcut_len = cmd.shortcut.chars().count();
                // +1 de respiro minimo entre nombre y atajo.
                let pad = width.saturating_sub(used + shortcut_len).max(1);
                let mut gap_style = normal;
                let mut sc_style = shortcut_style;
                if selected {
                    gap_style = gap_style.add_modifier(Modifier::REVERSED);
                    sc_style = sc_style.add_modifier(Modifier::REVERSED);
                }
                spans.push(Span::styled(" ".repeat(pad), gap_style));
                spans.push(Span::styled(cmd.shortcut.clone(), sc_style));
            }
            lines.push(Line::from(spans));
        }
        lines
    }

    /// Renderiza la paleta como un popup CENTRADO flotante (igual que el
    /// switcher, para que sean consistentes): atenua el editor de fondo y pinta un
    /// box con borde redondeado (acento del theme) en el centro, con un footer de
    /// atajos al pie. El titulo lleva el prompt, lo tipeado (con `_` de cursor) y
    /// el conteo; adentro va la lista rankeada (con scroll, resaltado del match y
    /// el atajo de cada comando).
    pub fn render(&self, frame: &mut ratatui::Frame, theme: &Theme) {
        use ratatui::widgets::{Clear, Paragraph};

        let area = frame.area();
        // Geometria compartida con el switcher: box ajustado al contenido, centrado.
        // Se dimensiona por el total de filas del display (incluye headers y
        // espaciadores de la vista agrupada), no solo por los comandos.
        let (popup, inner_width, shown) = picker_popup(area, self.display_len());
        // Filas de resultados (exactamente `shown`): sin matches, una linea tenue.
        let rows = if self.result_count() == 0 {
            vec![Line::from(Span::styled(
                i18n::t(Key::SwitcherEmpty).to_string(),
                Style::default().add_modifier(Modifier::DIM),
            ))]
        } else {
            self.result_lines(theme, shown, inner_width)
        };
        let prompt = picker_prompt(
            theme,
            i18n::t(Key::PalettePrompt),
            self.query(),
            self.result_count(),
        );
        let content = picker_content(prompt, rows);
        // Atenuar el editor de fondo (ya dibujado) en vez de borrarlo; despues
        // limpiar SOLO el rect del popup para que el box quede nitido encima.
        dim_area(frame.buffer_mut(), area);
        frame.render_widget(Clear, popup);
        frame.render_widget(Paragraph::new(content).block(picker_block(theme)), popup);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keybinding::StandardKeymap;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    /// Comandos del catalogo aplanados en orden (espeja el aplanado de
    /// `Palette::new`), para los asserts que dependen de la posicion o el total.
    fn flat() -> Vec<Action> {
        CATALOG
            .iter()
            .flat_map(|(_, cmds)| cmds.iter().map(|(a, _)| *a))
            .collect()
    }

    #[test]
    fn resuelve_atajos_de_comandos_fuera_de_la_toolbar() {
        // El probe descubre el atajo REAL de cada comando desde el keymap, no solo
        // de los que estan en la toolbar: undo, pegar, negrita directa y hasta los
        // chords (`^P I`) y los saltos por tecla especial (Home). Antes salian todos
        // en blanco porque el reverse-lookup solo miraba los hints de la barra.
        let km = StandardKeymap;
        let p = Palette::new(&km);
        let sc = |a: Action| {
            p.commands
                .iter()
                .find(|c| c.action == a)
                .unwrap_or_else(|| panic!("falta {a:?}"))
                .shortcut
                .clone()
        };
        assert_eq!(sc(Action::Undo), "^Z");
        assert_eq!(sc(Action::Paste), "^V");
        assert_eq!(sc(Action::ToggleBold), "^B");
        assert_eq!(sc(Action::ToggleItalic), "^P I", "chord de formato");
        assert_eq!(sc(Action::ToggleZen), "^O Z", "chord de vista");
        assert_eq!(sc(Action::LineStart), "Home", "tecla especial");
    }

    #[test]
    fn arranca_con_todos_los_comandos() {
        let km = StandardKeymap;
        let p = Palette::new(&km);
        assert_eq!(p.result_count(), flat().len());
    }

    #[test]
    fn no_lista_open_palette() {
        // OpenPalette se excluye del catalogo para no recursar.
        assert!(!flat().contains(&Action::OpenPalette));
    }

    #[test]
    fn tipear_filtra() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        // "save" deberia matchear "Save" y "Save+Quit" (locale En por default en
        // tests), pero no, por ejemplo, "Undo".
        for c in "save".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        assert!(p.result_count() >= 1);
        assert!(p.result_count() < flat().len());
    }

    #[test]
    fn enter_acepta_el_action_correcto() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        // Filtrar a "Undo" (label En) y aceptar: debe devolver Action::Undo.
        for c in "undo".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => assert_eq!(action, Action::Undo),
            _ => panic!("esperaba Accept(Undo)"),
        }
    }

    #[test]
    fn esc_cancela() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        assert!(matches!(
            p.handle_key(key(KeyCode::Esc)),
            PaletteOutcome::Cancel
        ));
    }

    #[test]
    fn enter_sin_resultados_cancela() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        for c in "zzzqqq".chars() {
            p.handle_key(key(KeyCode::Char(c)));
        }
        assert_eq!(p.result_count(), 0);
        assert!(matches!(
            p.handle_key(key(KeyCode::Enter)),
            PaletteOutcome::Cancel
        ));
    }

    #[test]
    fn ctrl_n_y_ctrl_p_navegan() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        // Bajar una vez y aceptar: debe ser el SEGUNDO comando (con query vacia la
        // vista es agrupada y el primer par de comandos vive en la misma seccion,
        // asi que Ctrl-N pasa de flat()[0] a flat()[1] sin cruzar un header).
        p.handle_key(ctrl(KeyCode::Char('n')));
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => assert_eq!(action, flat()[1]),
            _ => panic!("esperaba Accept"),
        }
        // Y Ctrl-P vuelve arriba (clampea en el primer comando).
        let mut p = Palette::new(&km);
        p.handle_key(ctrl(KeyCode::Char('n')));
        p.handle_key(ctrl(KeyCode::Char('p')));
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => assert_eq!(action, flat()[0]),
            _ => panic!("esperaba Accept"),
        }
    }

    #[test]
    fn resuelve_el_atajo_actual_del_preset() {
        // En standard, Save se bindea a ^S: la paleta debe mostrarlo.
        let km = StandardKeymap;
        let p = Palette::new(&km);
        let save = p
            .commands
            .iter()
            .find(|c| c.action == Action::Save)
            .expect("deberia estar Save");
        assert_eq!(save.shortcut, "^S");
    }

    #[test]
    fn muestra_el_atajo_del_switcher() {
        // Regresion: "Ir a archivo" (OpenSwitcher) esta bindeado a ^E en standard
        // y debe mostrar ese atajo en la paleta (antes salia vacio porque ^E no
        // era un hint de la toolbar y el reverse-lookup no lo encontraba).
        let km = StandardKeymap;
        let p = Palette::new(&km);
        let go = p
            .commands
            .iter()
            .find(|c| c.action == Action::OpenSwitcher)
            .expect("deberia estar OpenSwitcher");
        assert_eq!(go.shortcut, "^E");
    }

    #[test]
    fn comando_sin_atajo_queda_vacio() {
        // En standard no hay hint de DocStart en la toolbar: su atajo queda vacio
        // (pero el comando igual aparece en la paleta).
        let km = StandardKeymap;
        let p = Palette::new(&km);
        let doc_start = p
            .commands
            .iter()
            .find(|c| c.action == Action::DocStart)
            .expect("deberia estar DocStart");
        assert!(doc_start.shortcut.is_empty());
    }

    #[test]
    fn la_seleccion_clampea_hacia_abajo() {
        let km = StandardKeymap;
        let mut p = Palette::new(&km);
        // Bajar mucho mas que la cantidad de comandos: no debe pasar del ultimo.
        for _ in 0..(flat().len() + 10) {
            p.handle_key(key(KeyCode::Down));
        }
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => {
                assert_eq!(action, *flat().last().unwrap())
            }
            _ => panic!("esperaba Accept"),
        }
    }
}
