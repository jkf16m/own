use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph, Wrap},
    Terminal,
};
use std::fs;
use std::path::Path;

use crate::store::Store;
use crate::tags::TagStore;

// ─── Review State ────────────────────────────────────────────────────────────

struct ReviewState {
    file_name: String,
    lines: Vec<String>,
    cursor: usize,
    scroll_offset: usize,
    viewport_height: usize,
    store: Store,
    tags: TagStore,
    mode: Mode,
    input_buffer: String,
    tag_filter: String,
    tag_cursor: usize,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    InputNote,
    SelectTag,
}

impl ReviewState {
    fn new(file_path: &std::path::PathBuf) -> Result<Self> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let file_name = file_path.to_string_lossy().to_string();

        Ok(ReviewState {
            file_name,
            lines,
            cursor: 0,
            scroll_offset: 0,
            viewport_height: 20,
            store: Store::load()?,
            tags: TagStore::load(),
            mode: Mode::Normal,
            input_buffer: String::new(),
            tag_filter: String::new(),
            tag_cursor: 0,
        })
    }

    fn line_number(&self) -> usize {
        self.cursor + 1
    }

    fn get_line_tags(&self) -> Vec<String> {
        let ownership = self.store.get_file(&self.file_name);
        ownership
            .entries
            .get(&(self.line_number()))
            .map(|e| e.tags.clone())
            .unwrap_or_default()
    }

    fn set_line_tags(&mut self, tags: Vec<String>, note: Option<String>) {
        let line_num = self.line_number();
        self.store
            .get_file_mut(&self.file_name)
            .set_line(line_num, tags, note);
    }

    fn toggle_tag(&mut self, tag: &str) {
        let mut current = self.get_line_tags();
        if current.contains(&tag.to_string()) {
            current.retain(|t| t != tag);
        } else {
            current.push(tag.to_string());
        }
        self.set_line_tags(current, None);
    }

    fn get_filtered_tags(&self) -> Vec<String> {
        let filter = self.tag_filter.to_lowercase();
        self.tags
            .tags
            .keys()
            .filter(|name| name.to_lowercase().contains(&filter))
            .cloned()
            .collect()
    }

    fn move_cursor(&mut self, delta: i32) {
        let new_pos = self.cursor as i32 + delta;
        self.cursor = new_pos.max(0).min(self.lines.len() as i32 - 1) as usize;
        self.update_scroll();
    }

    fn update_scroll(&mut self) {
        if self.cursor < self.scroll_offset {
            self.scroll_offset = self.cursor;
        }
        if self.cursor >= self.scroll_offset + self.viewport_height {
            self.scroll_offset = self.cursor - self.viewport_height + 1;
        }
    }
}

// ─── TUI ─────────────────────────────────────────────────────────────────────

pub fn run(file_path: &Path) -> Result<()> {
    if !file_path.exists() {
        anyhow::bail!("File not found: {}", file_path.display());
    }

    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = ReviewState::new(&file_path.to_path_buf())?;
    let result = run_app(&mut terminal, &mut state);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    state.store.save()?;

    if let Err(err) = result {
        eprintln!("Error: {}", err);
    }

    Ok(())
}

fn run_app(
    terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
    state: &mut ReviewState,
) -> Result<()> {
    loop {
        let size = terminal.size()?;
        state.viewport_height = size.height.saturating_sub(4) as usize;

        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(3), // Header
                    Constraint::Min(1),   // Content
                    Constraint::Length(3), // Footer
                ])
                .split(f.area());

            // Header
            let line_num = state.line_number();
            let tags = state.get_line_tags();
            let tags_str = if tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", tags.join(","))
            };

            let header = Paragraph::new(Line::from(vec![
                Span::styled(
                    format!(" Line {}/{}", line_num, state.lines.len()),
                    Style::default().fg(Color::White),
                ),
                Span::styled(tags_str, Style::default().fg(Color::Cyan)),
                Span::raw("  "),
                Span::styled(
                    state.file_name.clone(),
                    Style::default().fg(Color::DarkGray),
                ),
            ]))
            .block(Block::default().borders(Borders::ALL));
            f.render_widget(header, chunks[0]);

            // Content
            let visible: Vec<Line> = state
                .lines
                .iter()
                .enumerate()
                .skip(state.scroll_offset)
                .take(state.viewport_height)
                .map(|(i, line)| {
                    let line_num = i + 1;
                    let is_current = line_num == state.line_number();

                    // Get tags for this line
                    let ownership = state.store.get_file(&state.file_name);
                    let entry = ownership.entries.get(&line_num);
                    let has_tags = entry.map(|e| !e.tags.is_empty()).unwrap_or(false);
                    let note = entry.and_then(|e| e.note.as_ref());

                    // Build marker
                    let marker = if has_tags {
                        let tag_str = entry.unwrap().tags.join(",");
                        Span::styled(
                            format!("{:>3}", tag_str),
                            Style::default().fg(Color::Cyan),
                        )
                    } else {
                        Span::raw("   ")
                    };

                    let line_style = if is_current {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };

                    let mut spans = vec![
                        Span::styled(format!("{:>4} ", line_num), line_style),
                        marker,
                        Span::raw(" │ "),
                        Span::styled(line.clone(), line_style),
                    ];

                    if let Some(note) = note {
                        spans.push(Span::styled(
                            format!(" ← {}", note),
                            Style::default().fg(Color::Magenta),
                        ));
                    }

                    if is_current {
                        spans.insert(0, Span::styled("▸ ", Style::default().fg(Color::Cyan)));
                    }

                    Line::from(spans)
                })
                .collect();

            let content = Paragraph::new(visible)
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                .wrap(Wrap { trim: false });
            f.render_widget(content, chunks[1]);

            // Footer
            let footer = if state.mode == Mode::InputNote {
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "Type note, ",
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::raw(" to save, "),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::raw(" cancel"),
                ]))
            } else if state.mode == Mode::SelectTag {
                Paragraph::new(Line::from(vec![
                    Span::styled(
                        "Type to filter, ",
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled("↑↓", Style::default().fg(Color::Yellow)),
                    Span::raw(" select, "),
                    Span::styled("Enter", Style::default().fg(Color::Yellow)),
                    Span::raw(" toggle, "),
                    Span::styled("Esc", Style::default().fg(Color::Yellow)),
                    Span::raw(" close"),
                ]))
            } else {
                Paragraph::new(Line::from(vec![
                    Span::styled("j/k", Style::default().fg(Color::Yellow)),
                    Span::raw(" move  "),
                    Span::styled("t", Style::default().fg(Color::Green)),
                    Span::raw(" tag  "),
                    Span::styled("n", Style::default().fg(Color::Cyan)),
                    Span::raw(" note  "),
                    Span::styled("d", Style::default().fg(Color::Red)),
                    Span::raw(" delete  "),
                    Span::styled("s", Style::default().fg(Color::DarkGray)),
                    Span::raw(" save  "),
                    Span::styled("q", Style::default().fg(Color::DarkGray)),
                    Span::raw(" quit"),
                ]))
            };

            let footer = footer.block(Block::default().borders(Borders::ALL));
            f.render_widget(footer, chunks[2]);

            // Tag selection popup
            if state.mode == Mode::SelectTag {
                let filtered = state.get_filtered_tags();
                let popup_height = (filtered.len() as u16 + 2).min(15);
                let popup_width = 40;

                let area = f.area();
                let popup_x = (area.width - popup_width) / 2;
                let popup_y = (area.height - popup_height) / 2;
                let popup_area = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

                f.render_widget(Clear, popup_area);

                let tag_lines: Vec<Line> = filtered
                    .iter()
                    .enumerate()
                    .map(|(i, name)| {
                        let style = if i == state.tag_cursor {
                            Style::default()
                                .fg(Color::White)
                                .add_modifier(Modifier::BOLD)
                        } else {
                            Style::default()
                        };

                        // Check if tag is on current line
                        let current_tags = state.get_line_tags();
                        let marker = if current_tags.contains(name) {
                            "✓ "
                        } else {
                            "  "
                        };

                        Line::from(vec![
                            Span::styled(marker, Style::default().fg(Color::Green)),
                            Span::styled(name.clone(), style),
                        ])
                    })
                    .collect();

                let tag_list = Paragraph::new(tag_lines)
                    .block(Block::default().title("Tags").borders(Borders::ALL));
                f.render_widget(tag_list, popup_area);
            }
        })?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match state.mode {
                        Mode::Normal => match key.code {
                            KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                            KeyCode::Char('j') | KeyCode::Down => state.move_cursor(1),
                            KeyCode::Char('k') | KeyCode::Up => state.move_cursor(-1),
                            KeyCode::Char('g') => {
                                state.cursor = 0;
                                state.scroll_offset = 0;
                            }
                            KeyCode::Char('G') => {
                                state.cursor = state.lines.len().saturating_sub(1);
                                state.update_scroll();
                            }
                            KeyCode::Char('s') => state.store.save()?,
                            KeyCode::Char('d') => {
                                let line_num = state.line_number();
                                state.store.get_file_mut(&state.file_name).entries.remove(&line_num);
                            }
                            KeyCode::Char('t') => {
                                state.mode = Mode::SelectTag;
                                state.tag_filter.clear();
                                state.tag_cursor = 0;
                            }
                            KeyCode::Char('n') => {
                                state.mode = Mode::InputNote;
                                state.input_buffer.clear();
                            }
                            _ => {}
                        },
                        Mode::SelectTag => match key.code {
                            KeyCode::Esc => {
                                state.mode = Mode::Normal;
                            }
                            KeyCode::Enter => {
                                let filtered = state.get_filtered_tags();
                                if let Some(tag) = filtered.get(state.tag_cursor) {
                                    state.toggle_tag(tag);
                                }
                            }
                            KeyCode::Up => {
                                state.tag_cursor = state.tag_cursor.saturating_sub(1);
                            }
                            KeyCode::Down => {
                                let max = state.get_filtered_tags().len().saturating_sub(1);
                                state.tag_cursor = (state.tag_cursor + 1).min(max);
                            }
                            KeyCode::Char(c) => {
                                state.tag_filter.push(c);
                                state.tag_cursor = 0;
                            }
                            KeyCode::Backspace => {
                                state.tag_filter.pop();
                                state.tag_cursor = 0;
                            }
                            _ => {}
                        },
                        Mode::InputNote => match key.code {
                            KeyCode::Esc => {
                                state.mode = Mode::Normal;
                                state.input_buffer.clear();
                            }
                            KeyCode::Enter => {
                                let note = state.input_buffer.clone();
                                let tags = state.get_line_tags();
                                state.set_line_tags(tags, Some(note));
                                state.mode = Mode::Normal;
                                state.input_buffer.clear();
                            }
                            KeyCode::Char(c) => {
                                state.input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                state.input_buffer.pop();
                            }
                            _ => {}
                        },
                    }
                }
            }
        }
    }
}
