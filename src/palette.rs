//! Paleta de comandos estilo M-x (Emacs) / Command Palette: un overlay que
//! fuzzy-filtra los comandos del editor por su nombre legible y ejecuta el
//! elegido. Vive aparte (como el switcher) porque opera a nivel del loop
//! (`run`): al aceptar, NO toca el documento directamente sino que devuelve el
//! `Action` elegido, que `run` despacha por el mismo camino que el keymap.
//!
//! El catalogo de comandos (`COMMANDS`) es la lista curada de `Action` "de
//! comando": los utiles desde una paleta (guardar, buscar, formato, navegacion,
//! etc.). Quedan EXCLUIDOS los de tipeo/cursor (InsertChar, Backspace,
//! Cursor*/Select*, ...) que no tienen sentido invocar por nombre, y el propio
//! `OpenPalette` (para no recursar). El nombre visible sale de las keys i18n; el
//! atajo actual se resuelve por reverse-lookup contra los hints del keymap, asi
//! refleja el preset activo y los remapeos del usuario.

use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

use crate::document::Mode;
use crate::fuzzy;
use crate::i18n::{self, Key};
use crate::keybinding::{Action, Keymap};
use crate::switcher::scroll_window;
use crate::theme::Theme;

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

/// Catalogo de comandos de la paleta: cada entrada empareja un `Action` con la
/// key i18n de su nombre visible. El orden define el orden por defecto (query
/// vacia) y el desempate del ranking. NO incluye `OpenPalette` (evita recursar)
/// ni los de tipeo/cursor (no tienen sentido invocarse por nombre).
const COMMANDS: &[(Action, Key)] = &[
    (Action::Save, Key::HintSave),
    (Action::SaveAndQuit, Key::HintSaveQuit),
    (Action::Quit, Key::HintQuit),
    (Action::Search, Key::HintSearch),
    (Action::Replace, Key::HintReplace),
    (Action::Undo, Key::HintUndo),
    (Action::Redo, Key::HintRedo),
    (Action::Yank, Key::HintYank),
    (Action::Paste, Key::HintPaste),
    (Action::ToggleBold, Key::HintBold),
    (Action::ToggleItalic, Key::HintItalic),
    (Action::ToggleCode, Key::HintCode),
    (Action::ToggleZen, Key::HintZen),
    (Action::ToggleWhitepaper, Key::HintWhitepaper),
    (Action::ExportHtml, Key::HintExportHtml),
    (Action::NewBuffer, Key::HintNew),
    (Action::CloseBuffer, Key::HintCloseBuffer),
    (Action::OpenSwitcher, Key::HintSwitcher),
    (Action::LineStart, Key::HintLineStart),
    (Action::LineEnd, Key::HintLineEnd),
    (Action::DocStart, Key::HintDocStart),
    (Action::DocEnd, Key::HintDocEnd),
    (Action::PageUp, Key::HintPageUp),
    (Action::PageDown, Key::HintPageDown),
    (Action::EnterInsert, Key::HintInsert),
    (Action::EnterNormal, Key::HintNormal),
    (Action::EnterVisual, Key::HintVisual),
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

/// Estado de la paleta: el catalogo resuelto, el texto tipeado, los resultados
/// rankeados (indices en `commands` + match para resaltar) y el seleccionado.
pub struct Palette {
    /// Comandos disponibles (catalogo resuelto contra el keymap activo).
    commands: Vec<Command>,
    /// Texto tipeado.
    query: String,
    /// Resultados rankeados: indices en `commands` con su match (para resaltar).
    results: Vec<(usize, fuzzy::FuzzyMatch)>,
    /// Item seleccionado dentro de `results`.
    selected: usize,
}

impl Palette {
    /// Construye la paleta resolviendo el catalogo contra `keymap`: cada comando
    /// toma su nombre i18n y su atajo actual (reverse-lookup en los hints de
    /// TODOS los modos). La query arranca vacia (todos los comandos matchean).
    pub fn new(keymap: &dyn Keymap) -> Self {
        let shortcuts = shortcut_table(keymap);
        let commands: Vec<Command> = COMMANDS
            .iter()
            .map(|&(action, key)| Command {
                action,
                name: i18n::t(key).to_string(),
                shortcut: shortcuts
                    .iter()
                    .find(|(a, _)| *a == action)
                    .map(|(_, keys)| keys.clone())
                    .unwrap_or_default(),
            })
            .collect();
        let mut p = Palette {
            commands,
            query: String::new(),
            results: Vec::new(),
            selected: 0,
        };
        p.recompute();
        p
    }

    /// Recalcula el ranking de los nombres contra la query y resetea la seleccion
    /// al tope.
    fn recompute(&mut self) {
        let refs: Vec<&str> = self.commands.iter().map(|c| c.name.as_str()).collect();
        self.results = fuzzy::rank(&self.query, &refs);
        self.selected = 0;
    }

    /// Mueve la seleccion `delta` filas, con clamp a los limites.
    fn move_selection(&mut self, delta: isize) {
        if self.results.is_empty() {
            return;
        }
        let last = (self.results.len() - 1) as isize;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
    }

    /// Procesa una tecla y dice que hacer. Tipear/borrar refiltra; flechas (o
    /// Ctrl-N/Ctrl-P) navegan; Enter ejecuta el seleccionado; Esc cancela.
    pub fn handle_key(&mut self, key: KeyEvent) -> PaletteOutcome {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        match key.code {
            KeyCode::Esc => PaletteOutcome::Cancel,
            KeyCode::Enter => match self.results.get(self.selected) {
                Some(&(ci, _)) => PaletteOutcome::Accept(self.commands[ci].action),
                None => PaletteOutcome::Cancel,
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

    /// Cantidad de comandos que matchean la query actual.
    pub fn result_count(&self) -> usize {
        self.results.len()
    }

    /// Lineas de resultados a dibujar (hasta `max_rows`), con scroll para mantener
    /// visible el seleccionado, los chars del nombre que matchearon acentuados, el
    /// atajo actual alineado a la derecha y la fila seleccionada marcada (prefijo
    /// `>` y reverse). `width` es el ancho util del box para alinear el atajo.
    pub fn result_lines(&self, theme: &Theme, max_rows: usize, width: usize) -> Vec<Line<'static>> {
        // Ventana de scroll: que el seleccionado siempre entre en `max_rows`
        // (logica compartida con el switcher).
        let std::ops::Range { start, end } =
            scroll_window(self.selected, self.results.len(), max_rows);
        if start == end {
            return Vec::new();
        }

        let accent = Style::default()
            .fg(theme.heading_2)
            .add_modifier(Modifier::BOLD);
        let normal = Style::default();
        // El atajo va dimmeado (es secundario al nombre del comando).
        let shortcut_style = Style::default().add_modifier(Modifier::DIM);

        let mut lines = Vec::with_capacity(end - start);
        for ri in start..end {
            let (ci, m) = &self.results[ri];
            let cmd = &self.commands[*ci];
            let selected = ri == self.selected;

            // Un prefijo marca el seleccionado (se nota aunque el terminal no
            // pinte el reverse).
            let marker = if selected { "> " } else { "  " };
            let mut spans: Vec<Span<'static>> = vec![Span::styled(marker.to_string(), normal)];
            // Nombre char a char: acento si el indice matcheo, normal si no.
            for (i, ch) in cmd.name.chars().enumerate() {
                let mut style = if m.indices.contains(&i) {
                    accent
                } else {
                    normal
                };
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
    /// switcher, para que sean consistentes): limpia toda la pantalla y pinta un
    /// box con borde en el centro. El titulo lleva el prompt, lo tipeado (con `_`
    /// de cursor) y el conteo; adentro va la lista rankeada (con scroll, resaltado
    /// del match y el atajo de cada comando).
    pub fn render(&self, frame: &mut ratatui::Frame, theme: &Theme) {
        use ratatui::layout::Rect;
        use ratatui::widgets::{Block, Clear, Paragraph};

        let area = frame.area();
        // Popup centrado: ~70% del area, acotado (mismo criterio que el switcher).
        let w = (area.width * 7 / 10).clamp(40.min(area.width), area.width);
        let h = (area.height * 7 / 10).clamp(3.min(area.height), area.height);
        let popup = Rect {
            x: area.x + (area.width - w) / 2,
            y: area.y + (area.height - h) / 2,
            width: w,
            height: h,
        };

        let prompt = i18n::t(Key::PalettePrompt);
        let title = format!(" {prompt} {}_   ({}) ", self.query(), self.result_count());
        let block = Block::bordered().title(title);
        // Alto y ancho utiles dentro del borde del box (restan 2: ambos lados).
        let rows = popup.height.saturating_sub(2) as usize;
        let inner_width = popup.width.saturating_sub(2) as usize;
        let lines = self.result_lines(theme, rows, inner_width);
        // `Clear` limpia TODA la pantalla (el editor de fondo); despues el box.
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(lines).block(block), popup);
    }
}

/// Construye la tabla `Action -> atajo` recorriendo los hints del keymap en
/// TODOS los modos. Se queda con la primera ocurrencia de cada accion (los hints
/// de un mismo comando comparten su tecla entre modos). Refleja el preset activo
/// y los remapeos del usuario (que ya vienen aplicados en `keymap.hints`).
fn shortcut_table(keymap: &dyn Keymap) -> Vec<(Action, String)> {
    let mut table: Vec<(Action, String)> = Vec::new();
    for mode in [Mode::Normal, Mode::Insert, Mode::Visual] {
        for hint in keymap.hints(mode) {
            let Some(action) = hint.action else { continue };
            if !table.iter().any(|(a, _)| *a == action) {
                table.push((action, hint.keys));
            }
        }
    }
    table
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

    #[test]
    fn arranca_con_todos_los_comandos() {
        let km = StandardKeymap;
        let p = Palette::new(&km);
        assert_eq!(p.result_count(), COMMANDS.len());
    }

    #[test]
    fn no_lista_open_palette() {
        // OpenPalette se excluye del catalogo para no recursar.
        assert!(!COMMANDS.iter().any(|(a, _)| *a == Action::OpenPalette));
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
        assert!(p.result_count() < COMMANDS.len());
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
        // Bajar una vez y aceptar: debe ser el SEGUNDO comando del catalogo (con
        // query vacia el orden es el de `COMMANDS`).
        p.handle_key(ctrl(KeyCode::Char('n')));
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => assert_eq!(action, COMMANDS[1].0),
            _ => panic!("esperaba Accept"),
        }
        // Y Ctrl-P vuelve arriba (clampea en 0).
        let mut p = Palette::new(&km);
        p.handle_key(ctrl(KeyCode::Char('n')));
        p.handle_key(ctrl(KeyCode::Char('p')));
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => assert_eq!(action, COMMANDS[0].0),
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
        for _ in 0..(COMMANDS.len() + 10) {
            p.handle_key(key(KeyCode::Down));
        }
        match p.handle_key(key(KeyCode::Enter)) {
            PaletteOutcome::Accept(action) => {
                assert_eq!(action, COMMANDS[COMMANDS.len() - 1].0)
            }
            _ => panic!("esperaba Accept"),
        }
    }
}
