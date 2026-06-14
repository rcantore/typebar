//! typebar — editor de Markdown WYSIWYG para terminal (milestone editable minimo).
//!
//! Render "soft WYSIWYG": los marcadores (`**`, `*`, backticks, `#`) siempre
//! quedan visibles y dimmeados, asi el cursor se mueve 1:1 sobre el texto y la
//! posicion (linea, col) del buffer mapea directo a la columna en pantalla.
//! Edicion estilo Vim minima: modos Normal/Insert. `Ctrl-s` guarda.

mod document;
mod render;

use document::{Document, Mode};

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout, Position};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};

const DEFAULT_PATH: &str = "scratch.md";

fn main() -> std::io::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| DEFAULT_PATH.to_string());
    let document = Document::open(&path)?;

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, document);
    ratatui::restore();
    result
}

fn run(terminal: &mut ratatui::DefaultTerminal, mut doc: Document) -> std::io::Result<()> {
    // Offset vertical de scroll: primera linea visible del documento.
    let mut scroll: usize = 0;

    loop {
        terminal.draw(|frame| draw(frame, &doc, &mut scroll))?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
            && handle_key(&mut doc, key)?
        {
            return Ok(());
        }
    }
}

/// Dibuja el editor. Devuelve via `scroll` (mut) el offset usado, ajustado para
/// mantener el cursor visible.
fn draw(frame: &mut ratatui::Frame, doc: &Document, scroll: &mut usize) {
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
    let lines = render::render(&doc.text());
    let paragraph = Paragraph::new(lines)
        .block(block)
        .scroll((*scroll as u16, 0));
    frame.render_widget(paragraph, editor_area);

    frame.render_widget(status_bar(doc), status_area);

    // Cursor real de terminal: +1,+1 por el borde del Block, y restando scroll.
    if doc.line >= *scroll {
        let cursor_x = editor_area.x + 1 + doc.col as u16;
        let cursor_y = editor_area.y + 1 + (doc.line - *scroll) as u16;
        frame.set_cursor_position(Position::new(cursor_x, cursor_y));
    }
}

/// Construye la barra de estado: modo, path, dirty y linea:col.
fn status_bar(doc: &Document) -> Line<'static> {
    let mode = match doc.mode {
        Mode::Normal => "NORMAL",
        Mode::Insert => "INSERT",
    };
    let dirty = if doc.dirty { " [+]" } else { "" };
    let left = format!(" {} · {}{} ", mode, doc.path.display(), dirty);
    let right = format!(" {}:{} ", doc.line + 1, doc.col + 1);
    Line::from(vec![
        Span::styled(left, Style::default().add_modifier(Modifier::REVERSED)),
        Span::raw(" "),
        Span::raw(right),
    ])
}

/// Procesa una tecla. Devuelve `Ok(true)` cuando hay que salir.
fn handle_key(doc: &mut Document, key: KeyEvent) -> std::io::Result<bool> {
    // Ctrl-s guarda en cualquier modo.
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('s') {
        doc.save()?;
        return Ok(false);
    }

    match doc.mode {
        Mode::Normal => handle_normal(doc, key),
        Mode::Insert => {
            handle_insert(doc, key);
            Ok(false)
        }
    }
}

fn handle_normal(doc: &mut Document, key: KeyEvent) -> std::io::Result<bool> {
    match key.code {
        KeyCode::Char('q') => return Ok(true),
        KeyCode::Char('h') | KeyCode::Left => doc.move_left(),
        KeyCode::Char('l') | KeyCode::Right => doc.move_right(),
        KeyCode::Char('k') | KeyCode::Up => doc.move_up(),
        KeyCode::Char('j') | KeyCode::Down => doc.move_down(),
        KeyCode::Char('i') => doc.mode = Mode::Insert,
        KeyCode::Char('a') => {
            doc.move_right_for_append();
            doc.mode = Mode::Insert;
        }
        KeyCode::Char('x') => doc.delete_char(),
        KeyCode::Char('o') => {
            doc.open_line_below();
            doc.mode = Mode::Insert;
        }
        _ => {}
    }
    Ok(false)
}

fn handle_insert(doc: &mut Document, key: KeyEvent) {
    match key.code {
        KeyCode::Esc => doc.mode = Mode::Normal,
        KeyCode::Enter => doc.insert_newline(),
        KeyCode::Backspace => doc.backspace(),
        KeyCode::Char(c) => doc.insert_char(c),
        _ => {}
    }
}
