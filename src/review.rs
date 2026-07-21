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
    last_tag: Option<String>,
    select_start: Option<usize>,
    select_end: Option<usize>,
    is_stale: bool,
    file_path: std::path::PathBuf,
    author: String,
}

#[derive(PartialEq)]
enum Mode {
    Normal,
    InputNote,
    SelectTag,
    SelectTagRemove,
    Selecting,
    Command,
    ViewAnnotation,
}

impl ReviewState {
    fn new(file_path: &std::path::PathBuf) -> Result<Self> {
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        let lines: Vec<String> = content.lines().map(|l| l.to_string()).collect();
        let file_name = file_path.to_string_lossy().to_string();

        let mut store = Store::load()?;
        
        // Check if file has changed since last review and re-anchor if needed
        let is_stale = {
            let ownership = store.get_file(&file_name);
            ownership.is_stale(file_path)
        };
        
        if is_stale {
            let ownership = store.get_file_mut(&file_name);
            ownership.reanchor(&lines);
        }
        
        Ok(ReviewState {
            file_name,
            lines,
            cursor: 0,
            scroll_offset: 0,
            viewport_height: 20,
            store,
            tags: TagStore::load(),
            mode: Mode::Normal,
            input_buffer: String::new(),
            tag_filter: String::new(),
            tag_cursor: 0,
            last_tag: None,
            select_start: None,
            select_end: None,
            is_stale,
            file_path: file_path.clone(),
            author: std::env::var("USER").unwrap_or_else(|_| "unknown".to_string()),
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

    fn set_line_tags(&mut self, tags: Vec<String>) {
        let line_num = self.line_number();
        let content = self.lines.get(line_num - 1).map(|s| s.as_str());
        let author = self.author.clone();
        self.store
            .get_file_mut(&self.file_name)
            .set_line(line_num, tags, content, &author);
    }

    fn toggle_tag(&mut self, tag: &str) {
        let mut current = self.get_line_tags();
        if current.contains(&tag.to_string()) {
            current.retain(|t| t != tag);
        } else {
            current.push(tag.to_string());
        }
        self.set_line_tags(current);
    }

    fn remove_tag(&mut self, tag: &str) {
        let mut current = self.get_line_tags();
        current.retain(|t| t != tag);
        self.set_line_tags(current);
    }

    fn sync_tag_on_selection(&mut self, tag: &str) {
        let start = self.select_start.unwrap_or(self.line_number());
        let end = self.select_end.unwrap_or(self.line_number());
        let min = start.min(end);
        let max = start.max(end);

        // Check if ALL lines have the tag
        let all_have = (min..=max).all(|line_num| {
            let entry = self.store.get_file(&self.file_name).entries.get(&line_num);
            entry.map(|e| e.tags.contains(&tag.to_string())).unwrap_or(false)
        });

        // If all have it, remove from all. Otherwise, add to all.
        for line_num in min..=max {
            let entry = self.store.get_file(&self.file_name).entries.get(&line_num);
            let mut tags = entry.map(|e| e.tags.clone()).unwrap_or_default();
            let content = self.lines.get(line_num - 1).map(|s| s.as_str());
            let author = self.author.clone();
            
            if all_have {
                tags.retain(|t| t != tag);
            } else if !tags.contains(&tag.to_string()) {
                tags.push(tag.to_string());
            }
            
            self.store.get_file_mut(&self.file_name).set_line(line_num, tags, content, &author);
        }
    }

    fn set_annotation(&mut self, note: String) {
        let start = self.select_start.unwrap_or(self.line_number());
        let end = self.select_end.unwrap_or(self.line_number());
        let min = start.min(end);
        let max = start.max(end);
        self.store.get_file_mut(&self.file_name).set_annotation(min, max, note);
    }

    fn get_annotation(&self, line: usize) -> Option<&crate::store::Annotation> {
        self.store.get_file(&self.file_name).get_annotation(line)
    }

    fn is_in_selection(&self, line_num: usize) -> bool {
        match (self.select_start, self.select_end) {
            (Some(s), Some(e)) => {
                let min = s.min(e);
                let max = s.max(e);
                line_num >= min && line_num <= max
            }
            _ => false,
        }
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

fn parse_color(color: &str) -> Color {
    if color.starts_with('#') && color.len() == 7 {
        let r = u8::from_str_radix(&color[1..3], 16).unwrap_or(0);
        let g = u8::from_str_radix(&color[3..5], 16).unwrap_or(0);
        let b = u8::from_str_radix(&color[5..7], 16).unwrap_or(0);
        Color::Rgb(r, g, b)
    } else if color.starts_with("hsl(") {
        // Simple HSL to RGB conversion
        let parts: Vec<&str> = color.trim_start_matches("hsl(").trim_end_matches(')').split(',').collect();
        if parts.len() == 3 {
            let h: f64 = parts[0].trim().trim_end_matches('°').parse().unwrap_or(0.0);
            let s: f64 = parts[1].trim().trim_end_matches('%').parse().unwrap_or(0.0) / 100.0;
            let l: f64 = parts[2].trim().trim_end_matches('%').parse().unwrap_or(0.0) / 100.0;
            let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
            let x = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
            let m = l - c / 2.0;
            let (r, g, b) = if h < 60.0 { (c, x, 0.0) }
                else if h < 120.0 { (x, c, 0.0) }
                else if h < 180.0 { (0.0, c, x) }
                else if h < 240.0 { (0.0, x, c) }
                else if h < 300.0 { (x, 0.0, c) }
                else { (c, 0.0, x) };
            Color::Rgb(
                ((r + m) * 255.0) as u8,
                ((g + m) * 255.0) as u8,
                ((b + m) * 255.0) as u8,
            )
        } else {
            Color::White
        }
    } else {
        Color::White
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

            let mode_str = match state.mode {
                Mode::Selecting => {
                    match (state.select_start, state.select_end) {
                        (Some(s), Some(e)) => format!(" SELECT {}-{} ", s.min(e), s.max(e)),
                        (Some(s), None) => format!(" SELECT {}-... ", s),
                        _ => String::new(),
                    }
                }
                _ => String::new(),
            };

            let mut header_spans = vec![];
            if state.is_stale {
                header_spans.push(Span::styled(
                    " ⚠ FILE CHANGED ",
                    Style::default().fg(Color::White).bg(Color::Red),
                ));
            }
            if !mode_str.is_empty() {
                header_spans.push(Span::styled(
                    mode_str,
                    Style::default().fg(Color::White).bg(Color::Yellow),
                ));
            }
            header_spans.push(Span::styled(
                format!(" Line {}/{}", line_num, state.lines.len()),
                Style::default().fg(Color::White),
            ));
            
            // Show tag names with colors
            for tag_name in &tags {
                header_spans.push(Span::raw(" "));
                let color = state.tags.tags.get(tag_name)
                    .map(|t| parse_color(&t.color))
                    .unwrap_or(Color::White);
                header_spans.push(Span::styled(
                    format!("●{}", tag_name),
                    Style::default().fg(color),
                ));
            }
            
            header_spans.push(Span::raw("  "));
            header_spans.push(Span::styled(
                state.file_name.clone(),
                Style::default().fg(Color::DarkGray),
            ));

            let header = Paragraph::new(Line::from(header_spans))
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
                    let annotation = ownership.get_annotation(line_num);

                    // Build marker - colored dots for tags, tree chars for annotations
                    let marker_spans: Vec<Span> = if has_tags || annotation.is_some() {
                        let mut markers: Vec<Span> = Vec::new();
                        
                        // Add tag dots (max 2)
                        if has_tags {
                            for t in entry.unwrap().tags.iter().take(2) {
                                let color = state.tags.tags.get(t)
                                    .map(|tag| parse_color(&tag.color))
                                    .unwrap_or(Color::White);
                                markers.push(Span::styled("●", Style::default().fg(color)));
                            }
                        }
                        
                        // Add annotation marker (tree chars for ranges)
                        if let Some(ann) = &annotation {
                            let ann_char = if ann.start_line == ann.end_line {
                                // Single line annotation
                                "●"
                            } else if line_num == ann.start_line {
                                // Start of range
                                "┌"
                            } else if line_num == ann.end_line {
                                // End of range
                                "└"
                            } else {
                                // Middle of range
                                "│"
                            };
                            markers.push(Span::styled(ann_char, Style::default().fg(Color::Magenta)));
                        }
                        
                        // Pad to 3 chars
                        while markers.len() < 3 {
                            markers.insert(0, Span::raw(" "));
                        }
                        markers
                    } else {
                        // No markers - skip the column entirely
                        vec![]
                    };

                    let in_selection = state.is_in_selection(line_num);
                    let line_style = if is_current {
                        Style::default().add_modifier(Modifier::BOLD)
                    } else if in_selection {
                        Style::default().bg(Color::DarkGray)
                    } else {
                        Style::default()
                    };

                    let mut spans = vec![
                        Span::styled(format!("{:>4} ", line_num), line_style),
                    ];
                    if !marker_spans.is_empty() {
                        spans.extend(marker_spans);
                        spans.push(Span::raw(" │ "));
                    }
                    spans.push(Span::styled(line.clone(), line_style));

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
                        format!("Note: {}", state.input_buffer),
                        Style::default().fg(Color::White),
                    ),
                    Span::styled(" │ Enter", Style::default().fg(Color::Yellow)),
                    Span::raw(" save, "),
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
            } else if state.mode == Mode::Command {
                Paragraph::new(Line::from(vec![
                    Span::styled(":", Style::default().fg(Color::White)),
                    Span::styled(&state.input_buffer, Style::default().fg(Color::White)),
                    Span::styled("█", Style::default().fg(Color::White)),
                ]))
            } else {
                let last_tag_str = match &state.last_tag {
                    Some(tag) => format!(" [{}]", tag),
                    None => String::new(),
                };

                Paragraph::new(Line::from(vec![
                    Span::styled("j/k", Style::default().fg(Color::Yellow)),
                    Span::raw(" move  "),
                    Span::styled("v", Style::default().fg(Color::Green)),
                    Span::raw(" sel  "),
                    Span::styled("t", Style::default().fg(Color::Green)),
                    Span::raw(" tag"),
                    Span::styled(last_tag_str, Style::default().fg(Color::Cyan)),
                    Span::raw("  "),
                    Span::styled("T", Style::default().fg(Color::Green)),
                    Span::raw(" pick  "),
                    Span::styled("r", Style::default().fg(Color::Red)),
                    Span::raw(" rm  "),
                    Span::styled("n", Style::default().fg(Color::Cyan)),
                    Span::raw(" note  "),
                    Span::styled("a", Style::default().fg(Color::Magenta)),
                    Span::raw(" view  "),
                    Span::styled(":", Style::default().fg(Color::White)),
                    Span::raw(" cmd"),
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

            // Tag removal popup
            if state.mode == Mode::SelectTagRemove {
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
                            Span::styled("✗ ", Style::default().fg(Color::Red))
                        } else {
                            Span::raw("  ")
                        };

                        Line::from(vec![
                            marker,
                            Span::styled(name.clone(), style),
                        ])
                    })
                    .collect();

                let tag_list = Paragraph::new(tag_lines)
                    .block(Block::default().title("Remove Tag").borders(Borders::ALL));
                f.render_widget(tag_list, popup_area);
            }

            // Annotation popup
            if state.mode == Mode::ViewAnnotation {
                let line_num = state.line_number();
                if let Some(ann) = state.get_annotation(line_num) {
                    let popup_height = 8;
                    let popup_width = 60;

                    let area = f.area();
                    let popup_x = (area.width - popup_width) / 2;
                    let popup_y = (area.height - popup_height) / 2;
                    let popup_area = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

                    f.render_widget(Clear, popup_area);

                    let ann_lines = vec![
                        Line::from(Span::styled(
                            format!("Lines {}-{}", ann.start_line, ann.end_line),
                            Style::default().fg(Color::Yellow),
                        )),
                        Line::raw(""),
                        Line::from(Span::raw(&ann.note)),
                    ];

                    let ann_popup = Paragraph::new(ann_lines)
                        .block(Block::default().title("Annotation").borders(Borders::ALL));
                    f.render_widget(ann_popup, popup_area);
                } else {
                    // No annotation on this line
                    let popup_height = 5;
                    let popup_width = 30;

                    let area = f.area();
                    let popup_x = (area.width - popup_width) / 2;
                    let popup_y = (area.height - popup_height) / 2;
                    let popup_area = ratatui::layout::Rect::new(popup_x, popup_y, popup_width, popup_height);

                    f.render_widget(Clear, popup_area);

                    let ann_lines = vec![
                        Line::from(Span::styled(
                            "No annotation on this line",
                            Style::default().fg(Color::DarkGray),
                        )),
                        Line::raw(""),
                        Line::from(Span::styled(
                            "Press 'n' to add one",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ];

                    let ann_popup = Paragraph::new(ann_lines)
                        .block(Block::default().title("Annotation").borders(Borders::ALL));
                    f.render_widget(ann_popup, popup_area);
                }
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
                            KeyCode::Char('s') => {
                                state.store.get_file_mut(&state.file_name).update_snapshot(&state.file_path);
                                state.store.save()?;
                                state.is_stale = false;
                            }
                            KeyCode::Char('d') => {
                                let line_num = state.line_number();
                                state.store.get_file_mut(&state.file_name).entries.remove(&line_num);
                            }
                            KeyCode::Char('T') => {
                                // Always open picker to choose a tag
                                state.mode = Mode::SelectTag;
                                state.tag_filter.clear();
                                state.tag_cursor = 0;
                            }
                            KeyCode::Char('t') => {
                                if state.last_tag.is_some() {
                                    // Has a tag, toggle it
                                    if state.select_start.is_some() {
                                        state.sync_tag_on_selection(&state.last_tag.clone().unwrap());
                                    } else {
                                        state.toggle_tag(&state.last_tag.clone().unwrap());
                                    }
                                } else {
                                    // No tag selected, open picker
                                    state.mode = Mode::SelectTag;
                                    state.tag_filter.clear();
                                    state.tag_cursor = 0;
                                }
                            }
                            KeyCode::Char('r') => {
                                if let Some(tag) = state.last_tag.clone() {
                                    if state.select_start.is_some() {
                                        // Remove from selection
                                        let start = state.select_start.unwrap();
                                        let end = state.select_end.unwrap_or(start);
                                        let min = start.min(end);
                                        let max = start.max(end);
                                        for line_num in min..=max {
                                            let entry = state.store.get_file(&state.file_name).entries.get(&line_num);
                                            let mut tags = entry.map(|e| e.tags.clone()).unwrap_or_default();
                                            let content = state.lines.get(line_num - 1).map(|s| s.as_str());
                                            let author = state.author.clone();
                                            tags.retain(|t| t != &tag);
                                            state.store.get_file_mut(&state.file_name).set_line(line_num, tags, content, &author);
                                        }
                                    } else {
                                        state.remove_tag(&tag);
                                    }
                                }
                            }
                            KeyCode::Char('a') => {
                                state.mode = Mode::ViewAnnotation;
                            }
                            KeyCode::Char('v') => {
                                state.select_start = Some(state.line_number());
                                state.select_end = None;
                                state.mode = Mode::Selecting;
                            }
                            KeyCode::Char('n') => {
                                // If annotation exists, edit it; otherwise create new
                                let line_num = state.line_number();
                                let ownership = state.store.get_file(&state.file_name);
                                state.input_buffer = ownership.get_annotation(line_num)
                                    .map(|a| a.note.clone())
                                    .unwrap_or_default();
                                state.mode = Mode::InputNote;
                            }
                            KeyCode::Char(':') => {
                                state.mode = Mode::Command;
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
                                    state.last_tag = Some(tag.clone());
                                }
                                state.mode = Mode::Normal;
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
                        Mode::SelectTagRemove => match key.code {
                            KeyCode::Esc => {
                                state.mode = Mode::Normal;
                            }
                            KeyCode::Enter => {
                                let filtered = state.get_filtered_tags();
                                if let Some(tag) = filtered.get(state.tag_cursor) {
                                    state.remove_tag(tag);
                                }
                                state.mode = Mode::Normal;
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
                        Mode::Selecting => match key.code {
                            KeyCode::Char('j') | KeyCode::Down => {
                                state.move_cursor(1);
                                state.select_end = Some(state.line_number());
                            }
                            KeyCode::Char('k') | KeyCode::Up => {
                                state.move_cursor(-1);
                                state.select_end = Some(state.line_number());
                            }
                            KeyCode::Char('t') => {
                                if let Some(tag) = state.last_tag.clone() {
                                    state.sync_tag_on_selection(&tag);
                                }
                            }
                            KeyCode::Char('n') => {
                                // If annotation exists, edit it; otherwise create new
                                let line_num = state.line_number();
                                let ownership = state.store.get_file(&state.file_name);
                                state.input_buffer = ownership.get_annotation(line_num)
                                    .map(|a| a.note.clone())
                                    .unwrap_or_default();
                                state.mode = Mode::InputNote;
                            }
                            KeyCode::Esc => {
                                state.select_start = None;
                                state.select_end = None;
                                state.mode = Mode::Normal;
                            }
                            KeyCode::Enter | KeyCode::Char('v') => {
                                state.mode = Mode::Normal;
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
                                if note.is_empty() {
                                    // Delete annotation if empty
                                    let line_num = state.line_number();
                                    state.store.get_file_mut(&state.file_name).remove_annotation(line_num);
                                } else {
                                    state.set_annotation(note);
                                }
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
                        Mode::Command => match key.code {
                            KeyCode::Esc => {
                                state.mode = Mode::Normal;
                                state.input_buffer.clear();
                            }
                            KeyCode::Enter => {
                                let cmd = state.input_buffer.trim().to_string();
                                state.mode = Mode::Normal;
                                state.input_buffer.clear();
                                
                                match cmd.as_str() {
                                    "q" | "quit" => return Ok(()),
                                    "w" | "save" => {
                                        state.store.save()?;
                                    }
                                    "wq" | "wq" => {
                                        state.store.save()?;
                                        return Ok(());
                                    }
                                    "d" | "delete" => {
                                        let line_num = state.line_number();
                                        state.store.get_file_mut(&state.file_name).entries.remove(&line_num);
                                    }
                                    _ => {
                                        // Unknown command, ignore
                                    }
                                }
                            }
                            KeyCode::Char(c) => {
                                state.input_buffer.push(c);
                            }
                            KeyCode::Backspace => {
                                state.input_buffer.pop();
                            }
                            _ => {}
                        },
                        Mode::ViewAnnotation => match key.code {
                            KeyCode::Esc | KeyCode::Enter | KeyCode::Char('a') => {
                                state.mode = Mode::Normal;
                            }
                            _ => {}
                        },
                    }
                }
            }
        }
    }
}
