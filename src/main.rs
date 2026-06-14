//! typebar — spike tecnico read-only.
//!
//! Abre un .md hardcodeado, lo parsea con tree-sitter-md y lo renderiza con
//! ratatui aplicando estilos inline (soft WYSIWYG). Sin edicion ni cursor.
//! `q` para salir.

mod render;

use std::fs;

use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::widgets::{Block, Paragraph};

const SAMPLE_PATH: &str = "examples/sample.md";

fn main() -> std::io::Result<()> {
    let source = fs::read_to_string(SAMPLE_PATH)?;
    let lines = render::render(&source);

    let mut terminal = ratatui::init();
    let result = run(&mut terminal, lines);
    ratatui::restore();
    result
}

fn run(
    terminal: &mut ratatui::DefaultTerminal,
    lines: Vec<ratatui::text::Line<'static>>,
) -> std::io::Result<()> {
    let title = format!(" typebar · spike · {SAMPLE_PATH} · (q para salir) ");
    loop {
        terminal.draw(|frame| {
            let block = Block::bordered().title(title.as_str());
            let paragraph = Paragraph::new(lines.clone()).block(block);
            frame.render_widget(paragraph, frame.area());
        })?;

        if let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press && key.code == KeyCode::Char('q') {
                return Ok(());
            }
    }
}
