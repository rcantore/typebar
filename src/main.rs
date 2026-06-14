//! typebar — editor de Markdown WYSIWYG para terminal (milestone editable minimo).
//!
//! Render "soft WYSIWYG": los marcadores (`**`, `*`, backticks, `#`) siempre
//! quedan visibles y dimmeados, asi el cursor se mueve 1:1 sobre el texto y la
//! posicion (linea, col) del buffer mapea directo a la columna en pantalla.
//!
//! El teclado se maneja por presets intercambiables (ver `keybinding`): por
//! default `standard` (modeless, flechas), con `vim` (modal) y `wordstar`
//! (modeless con chords tipo `Ctrl-K S`) opt-in via el flag `--keys`. El loop
//! acumula teclas en un buffer `pending` para resolver secuencias multi-tecla.

mod document;
mod keybinding;
mod markdown;
mod render;
mod text;

use document::{Document, Mode};
use keybinding::{Action, Keymap, Resolve, keymap_from_name};
use markdown::InlineKind;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

const DEFAULT_PATH: &str = "scratch.md";
const DEFAULT_PRESET: &str = "standard";

/// Args parseados de la linea de comandos.
struct Args {
    path: String,
    preset: String,
}

/// Parsea los argumentos a mano (sin clap). Soporta `--keys <nombre>` y
/// `--keys=<nombre>` en cualquier posicion; el primer posicional (no-flag) es
/// el path del archivo. Defaults: path `scratch.md`, preset `standard`.
fn parse_args(raw: impl Iterator<Item = String>) -> Args {
    let mut path: Option<String> = None;
    let mut preset: Option<String> = None;
    let mut args = raw.peekable();

    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--keys=") {
            preset = Some(value.to_string());
        } else if arg == "--keys" {
            // Tomar el siguiente token como valor (si lo hay).
            preset = args.next();
        } else if !arg.starts_with('-') && path.is_none() {
            path = Some(arg);
        }
        // Cualquier otro flag desconocido se ignora silenciosamente.
    }

    Args {
        path: path.unwrap_or_else(|| DEFAULT_PATH.to_string()),
        preset: preset.unwrap_or_else(|| DEFAULT_PRESET.to_string()),
    }
}

fn main() -> std::io::Result<()> {
    // Saltar argv[0] (nombre del binario).
    let args = parse_args(std::env::args().skip(1));
    let keymap = keymap_from_name(&args.preset);

    let mut document = Document::open(&args.path)?;
    document.mode = keymap.initial_mode();

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, document, keymap.as_ref());
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    mut doc: Document,
    keymap: &dyn Keymap,
) -> std::io::Result<()> {
    // Offset vertical de scroll: primera linea visible del documento.
    let mut scroll: usize = 0;
    // Buffer de teclas de un chord en curso (vacio si no hay nada pendiente).
    let mut pending: Vec<KeyEvent> = Vec::new();

    loop {
        terminal.draw(|frame| draw(frame, &doc, keymap, &pending, &mut scroll))?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            pending.push(key);
            match keymap.resolve(doc.mode, &pending) {
                Resolve::Action(action) => {
                    pending.clear();
                    if apply_action(&mut doc, action)? {
                        return Ok(());
                    }
                }
                // La secuencia es prefijo de un chord: esperar mas teclas.
                Resolve::Pending => {}
                // Secuencia no bindeada: cancela el chord (o un Esc tras un
                // prefijo) limpiando el buffer pendiente.
                Resolve::None => pending.clear(),
            }
        }
    }
}

/// Dibuja el editor. Devuelve via `scroll` (mut) el offset usado, ajustado para
/// mantener el cursor visible.
fn draw(
    frame: &mut ratatui::Frame,
    doc: &Document,
    keymap: &dyn Keymap,
    pending: &[KeyEvent],
    scroll: &mut usize,
) {
    // Partir la pantalla: area de editor (resto) + 1 linea de status.
    let [editor_area, status_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(1)]).areas(frame.area());

    // Alto util dentro del borde del Block (resta 2: arriba y abajo).
    let viewport_height = editor_area.height.saturating_sub(2) as usize;

    // Ajustar scroll para que el cursor quede dentro del viewport.
    if viewport_height > 0 {
        if doc.line < *scroll {
            *scroll = doc.line;
        } else if doc.line >= *scroll + viewport_height {
            *scroll = doc.line + 1 - viewport_height;
        }
    }

    let block = Block::bordered().title(format!(" typebar · {} ", doc.path.display()));
    let lines = render::render(&doc.text(), doc.selection_byte_range());
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((*scroll as u16, 0));
    frame.render_widget(paragraph, editor_area);

    frame.render_widget(status_bar(doc, keymap, pending), status_area);

    // Cursor real de terminal: +1,+1 por el borde del Block, y restando scroll.
    // La X es la columna *visual* (celdas), no el indice de char: asi cae sobre
    // el glifo que dibujo el render aunque haya CJK/emoji de doble ancho.
    if doc.line >= *scroll {
        let cursor_x = editor_area.x + 1 + doc.display_col() as u16;
        let cursor_y = editor_area.y + 1 + (doc.line - *scroll) as u16;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

/// Construye la barra de estado: preset, modo (solo si es modal), path, dirty,
/// chord en curso y linea:col.
fn status_bar(doc: &Document, keymap: &dyn Keymap, pending: &[KeyEvent]) -> Line<'static> {
    // El modo solo tiene sentido en presets modales (Vim); en modeless no se
    // muestra "NORMAL/INSERT" porque no existen.
    let left = if keymap.is_modal() {
        let mode = match doc.mode {
            Mode::Normal => "NORMAL",
            Mode::Insert => "INSERT",
            Mode::Visual => "VISUAL",
        };
        format!(" {} · {} · {} ", keymap.name(), mode, doc.path.display())
    } else {
        // En modeless no hay modo; si hay una seleccion activa lo indicamos con
        // un sufijo SEL (en Vim eso ya lo cubre VISUAL).
        let sel = if doc.selection_range().is_some() {
            " · SEL"
        } else {
            ""
        };
        format!(" {} · {}{} ", keymap.name(), doc.path.display(), sel)
    };
    let dirty = if doc.dirty { "[+] " } else { "" };
    let left = format!("{}{}", left, dirty);
    let right = format!(" {}:{} ", doc.line + 1, doc.display_col() + 1);
    let mut spans = vec![
        Span::styled(left, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(" "),
    ];
    // Indicador de chord en curso (prefijo esperando la proxima tecla), tipo
    // "^K" / "^Q", para que el usuario sepa que esta en medio de una secuencia.
    if let Some(chord) = chord_indicator(pending) {
        spans.push(Span::styled(
            format!(" {} ", chord),
            Style::default().add_modifier(Modifier::REVERSED),
        ));
        spans.push(Span::raw(" "));
    }
    spans.push(Span::raw(right));
    Line::from(spans)
}

/// Representa el chord pendiente como texto (ej `^K`), o `None` si no hay
/// teclas pendientes. Se queda con la primer tecla del prefijo, que es la que
/// identifica el chord.
fn chord_indicator(pending: &[KeyEvent]) -> Option<String> {
    let first = pending.first()?;
    match first.code {
        KeyCode::Char(c) if first.modifiers.contains(KeyModifiers::CONTROL) => {
            Some(format!("^{}", c.to_ascii_uppercase()))
        }
        _ => None,
    }
}

/// Aplica una accion semantica sobre el documento. Devuelve `Ok(true)` si hay
/// que salir del editor (Quit).
fn apply_action(doc: &mut Document, action: Action) -> std::io::Result<bool> {
    match action {
        Action::CursorLeft => doc.move_left(),
        Action::CursorRight => doc.move_right(),
        Action::CursorUp => doc.move_up(),
        Action::CursorDown => doc.move_down(),
        Action::InsertChar(c) => doc.insert_char(c),
        Action::InsertNewline => doc.insert_newline(),
        Action::Backspace => doc.backspace(),
        Action::DeleteChar => doc.delete_char(),
        Action::EnterInsert => doc.mode = Mode::Insert,
        Action::EnterNormal => doc.mode = Mode::Normal,
        Action::InsertAfter => {
            doc.move_right_for_append();
            doc.mode = Mode::Insert;
        }
        Action::OpenLineBelow => {
            doc.open_line_below();
            doc.mode = Mode::Insert;
        }
        Action::LineStart => doc.move_to_line_start(),
        Action::LineEnd => doc.move_to_line_end(),
        Action::DocStart => doc.move_to_doc_start(),
        Action::DocEnd => doc.move_to_doc_end(),
        Action::Save => doc.save()?,
        Action::SaveAndQuit => {
            doc.save()?;
            return Ok(true);
        }
        Action::Quit => return Ok(true),
        Action::ToggleBold => toggle_inline_action(doc, InlineKind::Bold),
        Action::ToggleItalic => toggle_inline_action(doc, InlineKind::Italic),
        Action::ToggleCode => toggle_inline_action(doc, InlineKind::Code),
        Action::EnterVisual => {
            doc.mode = Mode::Visual;
            doc.start_selection();
        }
        Action::SelectLeft => doc.extend_left(),
        Action::SelectRight => doc.extend_right(),
        Action::SelectUp => doc.extend_up(),
        Action::SelectDown => doc.extend_down(),
        Action::DeleteSelection => {
            doc.delete_selection();
            // Si veniamos de Visual (Vim), la seleccion se consumio: volver a
            // Normal. En modeless no hay Visual, asi que esto no aplica.
            if doc.mode == Mode::Visual {
                doc.mode = Mode::Normal;
            }
        }
    }
    Ok(false)
}

/// Aplica un toggle de estilo inline. Si veniamos del modo Visual de Vim, la
/// seleccion se consume con el toggle y volvemos a Normal.
fn toggle_inline_action(doc: &mut Document, kind: InlineKind) {
    doc.toggle_inline(kind);
    if doc.mode == Mode::Visual {
        doc.mode = Mode::Normal;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_args_defaults() {
        let a = parse_args(Vec::<String>::new().into_iter());
        assert_eq!(a.path, DEFAULT_PATH);
        assert_eq!(a.preset, DEFAULT_PRESET);
    }

    #[test]
    fn parse_args_posicional_es_path() {
        let a = parse_args(vec!["notas.md".to_string()].into_iter());
        assert_eq!(a.path, "notas.md");
        assert_eq!(a.preset, DEFAULT_PRESET);
    }

    #[test]
    fn parse_args_keys_separado() {
        let a = parse_args(vec!["--keys".to_string(), "vim".to_string()].into_iter());
        assert_eq!(a.preset, "vim");
        assert_eq!(a.path, DEFAULT_PATH);
    }

    #[test]
    fn parse_args_keys_con_igual() {
        let a = parse_args(vec!["--keys=vim".to_string(), "notas.md".to_string()].into_iter());
        assert_eq!(a.preset, "vim");
        assert_eq!(a.path, "notas.md");
    }

    #[test]
    fn parse_args_keys_despues_del_path() {
        let a = parse_args(
            vec![
                "notas.md".to_string(),
                "--keys".to_string(),
                "vim".to_string(),
            ]
            .into_iter(),
        );
        assert_eq!(a.preset, "vim");
        assert_eq!(a.path, "notas.md");
    }
}
