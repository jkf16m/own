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
    widgets::{Block, Borders, Paragraph, Wrap},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

// ─── Ownership States ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum LineState {
    Reviewed,
    Questioned,
    Approved,
    Rejected,
}

impl LineState {
    fn from_char(c: char) -> Option<Self> {
        match c {
            'r' => Some(LineState::Reviewed),
            'q' => Some(LineState::Questioned),
            'a' => Some(LineState::Approved),
            'x' => Some(LineState::Rejected),
            _ => None,
        }
    }

    fn to_char(&self) -> char {
        match self {
            LineState::Reviewed => 'r',
            LineState::Questioned => 'q',
            LineState::Approved => 'a',
            LineState::Rejected => 'x',
        }
    }

    fn color(&self) -> Color {
        match self {
            LineState::Reviewed => Color::Green,
            LineState::Questioned => Color::Yellow,
            LineState::Approved => Color::Cyan,
            LineState::Rejected => Color::Red,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            LineState::Reviewed => "REVIEWED",
            LineState::Questioned => "QUESTIONED",
            LineState::Approved => "APPROVED",
            LineState::Rejected => "REJECTED",
        }
    }
}

// ─── Annotation ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Annotation {
    pub start_line: usize,
    pub end_line: usize,
    pub state: LineState,
    pub note: String,
}

// ─── Extract Command ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ExtractState {
    Rejected,
    Approved,
}

pub fn extract(state: ExtractState, file: Option<&Path>) -> Result<()> {
    let store = OwnershipStore::load()?;
    let repo_root = std::env::current_dir()?;
    
    let target_state = match state {
        ExtractState::Rejected => LineState::Rejected,
        ExtractState::Approved => LineState::Approved,
    };
    
    let label = match state {
        ExtractState::Rejected => "Rejected",
        ExtractState::Approved => "Approved",
    };
    
    let mut found = false;
    
    // Collect files to process
    let files: Vec<String> = if let Some(file_path) = file {
        let file_str = file_path
            .strip_prefix(&repo_root)
            .unwrap_or(file_path)
            .to_string_lossy()
            .to_string();
        vec![file_str]
    } else {
        store.files.keys().cloned().collect()
    };
    
    for file_name in &files {
        let ownership = store.get_file(file_name);
        
        // Collect ranges for this state
        let mut ranges: Vec<(usize, usize, String)> = Vec::new();
        
        // Check annotations first
        for ann in &ownership.annotations {
            if ann.state == target_state && !ann.note.is_empty() {
                ranges.push((ann.start_line, ann.end_line, ann.note.clone()));
            }
        }
        
        // Also check line states for ranges without annotations
        let mut current_range: Option<(usize, usize)> = None;
        for (&line_num, line_state) in &ownership.line_states {
            if *line_state == target_state {
                match &mut current_range {
                    Some((_, end)) if *end + 1 == line_num => {
                        *end = line_num;
                    }
                    _ => {
                        if let Some(range) = current_range.take() {
                            // Check if we already have this range from annotations
                            if !ranges.iter().any(|(s, e, _)| *s == range.0 && *e == range.1) {
                                ranges.push((range.0, range.1, String::new()));
                            }
                        }
                        current_range = Some((line_num, line_num));
                    }
                }
            }
        }
        if let Some(range) = current_range {
            if !ranges.iter().any(|(s, e, _)| *s == range.0 && *e == range.1) {
                ranges.push((range.0, range.1, String::new()));
            }
        }
        
        if ranges.is_empty() {
            continue;
        }
        
        found = true;
        
        // Read the source file
        let source_path = repo_root.join(file_name);
        let source_content = fs::read_to_string(&source_path)
            .with_context(|| format!("Failed to read {}", source_path.display()))?;
        let source_lines: Vec<&str> = source_content.lines().collect();
        
        // Print header
        println!("## {}", file_name);
        println!();
        
        // Print each range
        for (start, end, note) in &ranges {
            println!("### {} Lines {}-{}", label, start, end);
            
            // Print code block
            println!("```rust");
            for line_num in *start..=*end {
                if let Some(line) = source_lines.get(line_num - 1) {
                    println!("{}", line);
                }
            }
            println!("```");
            
            // Print annotation if present
            if !note.is_empty() {
                println!("> **Annotation:** {}", note);
            }
            println!();
        }
    }
    
    if !found {
        println!("No {} lines found.", label.to_lowercase());
        if let Some(file_path) = file {
            println!("File: {}", file_path.display());
        }
    }
    
    Ok(())
}

// ─── .own File Format ────────────────────────────────────────────────────────
//
// The .own file is a text format that stores:
// 1. A snapshot hash of the file when ownership was recorded
// 2. Line-by-line ownership states
// 3. Annotations (ranges with notes)
//
// Format:
// ```
// # snapshot: sha256:abc123def456...
// # file: src/main.rs
// 3-6:r
// 10-15:q:Why use String instead of &str?
// 20-25:x:This approach is wrong, needs refactoring
// 50:a:Approved after discussion
// ```
//
// When the file changes, we use git diff to migrate ownership:
// - Unchanged lines keep their state
// - New lines are unreviewed
// - Deleted lines are removed
// - We recompute the snapshot hash

#[derive(Debug, Clone, Default)]
pub struct FileOwnership {
    /// Snapshot hash of the file when ownership was recorded
    pub snapshot_hash: Option<String>,
    /// Line states (line number -> state)
    pub line_states: BTreeMap<usize, LineState>,
    /// Annotations (ranges with notes)
    pub annotations: Vec<Annotation>,
}

impl FileOwnership {
    /// Parse from .own file format
    fn parse(content: &str) -> Self {
        let mut result = Self::default();
        let mut annotations = Vec::new();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                // Parse header comments
                if let Some(hash) = line.strip_prefix("# snapshot: ") {
                    result.snapshot_hash = Some(hash.to_string());
                }
                continue;
            }

            // Parse ownership entries: "start-end:state" or "start-end:state:note"
            if let Some((range_part, rest)) = line.split_once(':') {
                let state_char = rest.chars().next().unwrap_or(' ');
                let note = if rest.len() > 2 {
                    rest[2..].trim().to_string()
                } else {
                    String::new()
                };

                if let Some(state) = LineState::from_char(state_char) {
                    // Parse line range (e.g., "3-6" or "42")
                    if let Some((start, end)) = range_part.split_once('-') {
                        if let (Ok(start), Ok(end)) = (start.parse(), end.parse()) {
                            if !note.is_empty() {
                                annotations.push(Annotation {
                                    start_line: start,
                                    end_line: end,
                                    state: state.clone(),
                                    note,
                                });
                            }
                            for line_num in start..=end {
                                result.line_states.insert(line_num, state.clone());
                            }
                        }
                    } else if let Ok(line_num) = range_part.parse::<usize>() {
                        if !note.is_empty() {
                            annotations.push(Annotation {
                                start_line: line_num,
                                end_line: line_num,
                                state: state.clone(),
                                note,
                            });
                        }
                        result.line_states.insert(line_num, state);
                    }
                }
            }
        }

        result.annotations = annotations;
        result
    }

    /// Serialize to .own file format
    fn serialize(&self, file_path: &str) -> String {
        let mut lines = Vec::new();

        // Header
        lines.push(format!("# file: {}", file_path));
        if let Some(hash) = &self.snapshot_hash {
            lines.push(format!("# snapshot: {}", hash));
        }
        lines.push(String::new());

        // Group consecutive lines with same state into ranges
        let ranges = self.group_into_ranges();

        // Write ranges
        for (start, end, state) in &ranges {
            // Check if there's an annotation for this range
            let annotation = self.annotations.iter().find(|a| {
                a.start_line == *start && a.end_line == *end && a.state == *state
            });

            if let Some(ann) = annotation {
                lines.push(format!("{}-{}:{:?}", start, end, state.to_char()));
                // Find and write the note
                lines.push(format!("  note: {}", ann.note));
            } else {
                lines.push(format!("{}-{}:{:?}", start, end, state.to_char()));
            }
        }

        lines.join("\n")
    }

    fn group_into_ranges(&self) -> Vec<(usize, usize, LineState)> {
        let mut ranges = Vec::new();
        let mut current: Option<(usize, usize, LineState)> = None;

        for (&line_num, state) in &self.line_states {
            match &mut current {
                Some((start, end, s)) if *s == *state && line_num == *end + 1 => {
                    *end = line_num;
                }
                Some((start, end, s)) => {
                    ranges.push((*start, *end, s.clone()));
                    current = Some((line_num, line_num, state.clone()));
                }
                None => {
                    current = Some((line_num, line_num, state.clone()));
                }
            }
        }

        if let Some((start, end, state)) = current {
            ranges.push((start, end, state));
        }

        ranges
    }

    fn get_line_state(&self, line: usize) -> Option<&LineState> {
        self.line_states.get(&line)
    }

    fn set_line_state(&mut self, line: usize, state: LineState) {
        self.line_states.insert(line, state);
    }

    fn clear_line_state(&mut self, line: usize) {
        self.line_states.remove(&line);
    }
}

// ─── Ownership Store ─────────────────────────────────────────────────────────

pub struct OwnershipStore {
    pub own_dir: PathBuf,
    pub files: BTreeMap<String, FileOwnership>,
}

impl OwnershipStore {
    pub fn load() -> Result<Self> {
        let own_dir = PathBuf::from(".own");
        let mut files = BTreeMap::new();

        if own_dir.exists() {
            Self::load_recursive(&own_dir, &own_dir, &mut files)?;
        }

        Ok(Self { own_dir, files })
    }

    fn load_recursive(
        base: &Path,
        current: &Path,
        files: &mut BTreeMap<String, FileOwnership>,
    ) -> Result<()> {
        if current.is_dir() {
            for entry in fs::read_dir(current)? {
                let entry = entry?;
                let path = entry.path();
                if path.is_dir() {
                    Self::load_recursive(base, &path, files)?;
                } else if path.extension().and_then(|e| e.to_str()) == Some("own") {
                    // Get relative path from .own directory
                    let rel_path = path.strip_prefix(base).unwrap_or(&path);
                    // Convert: .own/src/main.rs.own -> src/main.rs
                    // Just strip the .own extension
                    let path_str = rel_path.to_string_lossy();
                    let source_str = path_str.strip_suffix(".own").unwrap_or(&path_str).to_string();
                    let content = fs::read_to_string(&path)?;
                    files.insert(source_str, FileOwnership::parse(&content));
                }
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        fs::create_dir_all(&self.own_dir)?;

        for (file_name, ownership) in &self.files {
            // Create mirrored directory structure
            // file_name: src/main.rs -> .own/src/main.rs.own
            let own_path = self.own_dir.join(format!("{}.own", file_name));
            if let Some(parent) = own_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let content = ownership.serialize(file_name);
            fs::write(&own_path, content)?;
        }

        Ok(())
    }

    pub fn get_file(&self, file_name: &str) -> FileOwnership {
        self.files
            .get(file_name)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_file_mut(&mut self, file_name: &str) -> &mut FileOwnership {
        self.files
            .entry(file_name.to_string())
            .or_default()
    }

    /// Compute SHA256 hash of file content
    pub fn compute_hash(file_path: &Path) -> Result<String> {
        let content = fs::read(file_path)?;
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Simple hash - for production we'd use a proper SHA256
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Ok(format!("{:x}", hasher.finish()))
    }

    /// Migrate ownership from old file version to new using git diff
    pub fn migrate_ownership(
        &mut self,
        file_name: &str,
        old_content: &str,
        new_content: &str,
    ) -> Result<()> {
        let ownership = self.get_file(file_name);

        // If we don't have a snapshot, we can't migrate
        let old_hash = ownership.snapshot_hash.clone().unwrap_or_default();

        // Run git diff to see what changed
        let diff_output = Command::new("git")
            .args(["diff", "--no-index", "--unified=0", "/dev/stdin", "/dev/stdin"])
            // Actually, we need to write to temp files for this to work
            // Let's use a simpler approach: compare line by line
            .output();

        // For now, use a simple approach:
        // - Parse both files into lines
        // - Use a basic diff algorithm to find matching lines
        // - Preserve ownership for unchanged lines

        let old_lines: Vec<&str> = old_content.lines().collect();
        let new_lines: Vec<&str> = new_content.lines().collect();

        // Build a map of old line content -> line number
        let mut old_line_map: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (i, line) in old_lines.iter().enumerate() {
            old_line_map
                .entry(line.to_string())
                .or_default()
                .push(i + 1); // 1-indexed
        }

        // For each new line, try to find its original position
        let mut new_ownership = FileOwnership::default();
        let mut used_old_lines: std::collections::HashSet<usize> = std::collections::HashSet::new();

        for (new_line_num, new_line) in new_lines.iter().enumerate() {
            let new_line_num = new_line_num + 1;

            if let Some(old_positions) = old_line_map.get(*new_line) {
                // Find the first unused old position
                if let Some(&old_pos) = old_positions.iter().find(|&&p| !used_old_lines.contains(&p)) {
                    // Found a match - preserve ownership
                    if let Some(state) = ownership.get_line_state(old_pos) {
                        new_ownership.set_line_state(new_line_num, state.clone());
                    }
                    used_old_lines.insert(old_pos);
                }
            }
            // If no match, line is new -> unreviewed (no entry)
        }

        // Preserve annotations that still apply
        // (For now, we'll keep annotations but update their line numbers)
        // TODO: Smart annotation migration based on content matching

        // Update snapshot hash
        new_ownership.snapshot_hash = Some(Self::compute_hash(
            &PathBuf::from(file_name),
        )?);

        self.files.insert(file_name.to_string(), new_ownership);
        Ok(())
    }

    /// Check if file has changed since last review
    pub fn has_changed(&self, file_name: &str, current_path: &Path) -> Result<bool> {
        let ownership = self.get_file(file_name);
        if let Some(ref stored_hash) = ownership.snapshot_hash {
            let current_hash = Self::compute_hash(current_path)?;
            Ok(*stored_hash != current_hash)
        } else {
            // No snapshot = never reviewed = treat as changed
            Ok(true)
        }
    }

    pub fn file_stats(&self, file_name: &str) -> (usize, usize, usize, usize) {
        let ownership = self.get_file(file_name);
        let mut reviewed = 0;
        let mut questioned = 0;
        let mut approved = 0;
        let mut rejected = 0;

        for state in ownership.line_states.values() {
            match state {
                LineState::Reviewed => reviewed += 1,
                LineState::Questioned => questioned += 1,
                LineState::Approved => approved += 1,
                LineState::Rejected => rejected += 1,
            }
        }

        (reviewed, questioned, approved, rejected)
    }
}

// ─── TUI Review ──────────────────────────────────────────────────────────────

struct ReviewState {
    #[allow(dead_code)]
    file_path: PathBuf,
    file_name: String,
    lines: Vec<String>,
    store: OwnershipStore,
    cursor: usize,
    scroll_offset: usize,
    viewport_height: usize,
    changed_warning: bool,
}

impl ReviewState {
    fn new(file_path: PathBuf) -> Result<Self> {
        let content = fs::read_to_string(&file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        let lines: Vec<String> = content.lines().map(String::from).collect();
        let file_name = file_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_string();

        let mut store = OwnershipStore::load()?;

        // Check if file has changed
        let changed = store.has_changed(&file_name, &file_path)?;

        if changed {
            // Try to migrate ownership
            let ownership = store.get_file(&file_name);
            if ownership.snapshot_hash.is_some() {
                // We have old ownership data - try to migrate
                // For now, we'll just warn the user
                // TODO: Actually migrate using diff
            }
        }

        // Update snapshot hash
        let hash = OwnershipStore::compute_hash(&file_path)?;
        store.get_file_mut(&file_name).snapshot_hash = Some(hash);

        Ok(Self {
            file_path,
            file_name,
            lines,
            store,
            cursor: 0,
            scroll_offset: 0,
            viewport_height: 20,
            changed_warning: changed,
        })
    }

    fn current_line_number(&self) -> usize {
        self.cursor + 1
    }

    fn toggle_state(&mut self, state: LineState) {
        let line_num = self.current_line_number();
        let ownership = self.store.get_file(&self.file_name);
        let current = ownership.get_line_state(line_num);

        if current == Some(&state) {
            self.store
                .get_file_mut(&self.file_name)
                .clear_line_state(line_num);
        } else {
            self.store
                .get_file_mut(&self.file_name)
                .set_line_state(line_num, state);
        }
    }

    #[allow(dead_code)]
    fn add_annotation(&mut self, state: LineState, note: String) {
        let line_num = self.current_line_number();
        let ownership = self.store.get_file_mut(&self.file_name);

        ownership.annotations.push(Annotation {
            start_line: line_num,
            end_line: line_num,
            state,
            note,
        });
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

pub fn review(file_path: &Path) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = ReviewState::new(file_path.to_path_buf())?;

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
                    Constraint::Length(3),
                    Constraint::Min(10),
                    Constraint::Length(3),
                ])
                .split(f.area());

            // Header
            let (reviewed, questioned, approved, rejected) =
                state.store.file_stats(&state.file_name);
            let total = state.lines.len();
            let owned = reviewed + approved;
            let pct = if total > 0 {
                owned as f64 / total as f64 * 100.0
            } else {
                0.0
            };

            let mut header_lines = vec![
                Line::from(vec![
                    Span::styled(
                        "own ",
                        Style::default()
                            .fg(Color::Cyan)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(&state.file_name),
                    Span::raw(format!(
                        "  [{}/{} lines reviewed]",
                        owned, total
                    )),
                ]),
                Line::from(vec![
                    Span::styled(
                        format!(" {:.0}%", pct),
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw(" ownership"),
                    Span::raw(format!(
                        "  r:{} q:{} a:{} x:{}",
                        reviewed, questioned, approved, rejected
                    )),
                ]),
            ];

            if state.changed_warning {
                header_lines.push(Line::from(Span::styled(
                    " ⚠ File has changed since last review - unreviewed lines marked with •",
                    Style::default().fg(Color::Yellow),
                )));
            }

            let header = Paragraph::new(header_lines)
                .block(Block::default().borders(Borders::ALL).title("own"));
            f.render_widget(header, chunks[0]);

            // Content
            let ownership = state.store.get_file(&state.file_name);
            let visible_lines: Vec<Line> = state
                .lines
                .iter()
                .enumerate()
                .skip(state.scroll_offset)
                .take(state.viewport_height)
                .map(|(i, line)| {
                    let line_num = i + 1;
                    let is_current = i == state.cursor;

                    let marker = ownership
                        .get_line_state(line_num)
                        .map(|s| {
                            Span::styled(
                                format!(" {} ", s.to_char().to_uppercase()),
                                Style::default()
                                    .fg(Color::Black)
                                    .bg(s.color())
                                    .add_modifier(Modifier::BOLD),
                            )
                        })
                        .unwrap_or_else(|| {
                            if state.changed_warning {
                                Span::styled(
                                    " • ",
                                    Style::default().fg(Color::DarkGray),
                                )
                            } else {
                                Span::styled("   ", Style::default())
                            }
                        });

                    let num_style = if is_current {
                        Style::default()
                            .fg(Color::White)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default().fg(Color::DarkGray)
                    };

                    let code_style = if is_current {
                        Style::default().fg(Color::White)
                    } else {
                        Style::default().fg(Color::Gray)
                    };

                    let mut spans = vec![
                        Span::styled(format!("{:>4} ", line_num), num_style),
                        marker,
                        Span::raw(" │ "),
                        Span::styled(line.clone(), code_style),
                    ];

                    if is_current {
                        spans.insert(
                            0,
                            Span::styled("▸ ", Style::default().fg(Color::Cyan)),
                        );
                    }

                    Line::from(spans)
                })
                .collect();

            let content = Paragraph::new(visible_lines)
                .block(Block::default().borders(Borders::LEFT | Borders::RIGHT))
                .wrap(Wrap { trim: false });
            f.render_widget(content, chunks[1]);

            // Footer
            let footer = Paragraph::new(vec![
                Line::from(vec![
                    Span::styled("j/k", Style::default().fg(Color::Yellow)),
                    Span::raw(" move  "),
                    Span::styled("r", Style::default().fg(Color::Green)),
                    Span::raw(" reviewed  "),
                    Span::styled("Q", Style::default().fg(Color::Yellow)),
                    Span::raw(" questioned  "),
                    Span::styled("a", Style::default().fg(Color::Cyan)),
                    Span::raw(" approved  "),
                    Span::styled("x", Style::default().fg(Color::Red)),
                    Span::raw(" rejected  "),
                    Span::styled("n", Style::default().fg(Color::Magenta)),
                    Span::raw(" annotate  "),
                    Span::styled("s", Style::default().fg(Color::DarkGray)),
                    Span::raw(" save  "),
                    Span::styled("q", Style::default().fg(Color::DarkGray)),
                    Span::raw(" quit"),
                ]),
            ])
            .block(Block::default().borders(Borders::ALL));
            f.render_widget(footer, chunks[2]);
        })?;

        if event::poll(std::time::Duration::from_millis(50))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            return Ok(());
                        }
                        KeyCode::Char('j') | KeyCode::Down => {
                            state.move_cursor(1);
                        }
                        KeyCode::Char('k') | KeyCode::Up => {
                            state.move_cursor(-1);
                        }
                        KeyCode::Char('g') => {
                            state.cursor = 0;
                            state.update_scroll();
                        }
                        KeyCode::Char('G') => {
                            state.cursor = state.lines.len().saturating_sub(1);
                            state.update_scroll();
                        }
                        KeyCode::Char('r') => {
                            state.toggle_state(LineState::Reviewed);
                            state.move_cursor(1);
                        }
                        KeyCode::Char('Q') => {
                            state.toggle_state(LineState::Questioned);
                            state.move_cursor(1);
                        }
                        KeyCode::Char('a') => {
                            state.toggle_state(LineState::Approved);
                            state.move_cursor(1);
                        }
                        KeyCode::Char('x') => {
                            state.toggle_state(LineState::Rejected);
                            state.move_cursor(1);
                        }
                        KeyCode::Char('u') => {
                            let line_num = state.current_line_number();
                            state
                                .store
                                .get_file_mut(&state.file_name)
                                .clear_line_state(line_num);
                        }
                        KeyCode::Char('s') => {
                            state.store.save()?;
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}

// ─── Status Command ──────────────────────────────────────────────────────────

pub fn status(file: Option<&Path>) -> Result<()> {
    let store = OwnershipStore::load()?;

    if let Some(file_path) = file {
        // Get full path without extension for lookup (e.g., src/main.rs -> src/main)
        let file_name = file_path
            .with_extension("")
            .to_string_lossy()
            .to_string();
        let content = fs::read_to_string(file_path)
            .with_context(|| format!("Failed to read {}", file_path.display()))?;
        let total_lines = content.lines().count();

        let (reviewed, questioned, approved, rejected) = store.file_stats(&file_name);
        let owned = reviewed + approved;

        println!("File: {}", file_name);
        println!("Lines: {} total", total_lines);
        println!("Reviewed:   {}", reviewed);
        println!("Questioned: {}", questioned);
        println!("Approved:   {}", approved);
        println!("Rejected:   {}", rejected);

        if total_lines > 0 {
            println!("Ownership: {:.1}%", owned as f64 / total_lines as f64 * 100.0);
        }

        // Show annotations
        let ownership = store.get_file(&file_name);
        if !ownership.annotations.is_empty() {
            println!("\nAnnotations:");
            for ann in &ownership.annotations {
                println!(
                    "  Lines {}-{}: {:?} - {}",
                    ann.start_line, ann.end_line, ann.state, ann.note
                );
            }
        }
    } else {
        println!("=== own Ownership Status ===\n");

        if store.files.is_empty() {
            println!("No files reviewed yet.");
            println!("Run `own review <file>` to start tracking ownership.");
            return Ok(());
        }

        println!("{:<40} {:>8} {:>8} {:>6}", "File", "Owned", "Total", "%");
        println!("{}", "-".repeat(66));

        let mut total_owned = 0;
        let mut total_lines = 0;

        for (file_name, _) in &store.files {
            let path = PathBuf::from(file_name);
            
            // Skip ignored files
            if let Ok(repo_root) = std::env::current_dir() {
                if crate::ignore::should_ignore(&path, &repo_root) {
                    continue;
                }
            }
            
            let actual_lines = if let Ok(content) = fs::read_to_string(&path) {
                content.lines().count()
            } else {
                0
            };

            let (reviewed, _questioned, approved, _rejected) = store.file_stats(file_name);
            let owned = reviewed + approved;

            total_owned += owned;
            total_lines += actual_lines;

            let pct = if actual_lines > 0 {
                owned as f64 / actual_lines as f64 * 100.0
            } else {
                0.0
            };

            println!(
                "{:<40} {:>8} {:>8} {:>5.1}%",
                file_name, owned, actual_lines, pct
            );
        }

        println!("{}", "-".repeat(66));
        let total_pct = if total_lines > 0 {
            total_owned as f64 / total_lines as f64 * 100.0
        } else {
            0.0
        };
        println!(
            "{:<40} {:>8} {:>8} {:>5.1}%",
            "TOTAL", total_owned, total_lines, total_pct
        );
    }

    Ok(())
}

// ─── Init Command ────────────────────────────────────────────────────────────

pub fn init() -> Result<()> {
    let dir = PathBuf::from(".own");
    if dir.exists() {
        println!(".own directory already exists");
    } else {
        fs::create_dir_all(&dir)?;
        println!("Created .own/ directory");
    }

    println!("\nYou can now run `own review <file>` to start tracking ownership.");
    Ok(())
}

// ─── Remove Command ─────────────────────────────────────────────────────────

pub fn remove(path: &Path) -> Result<()> {
    let mut store = OwnershipStore::load()?;
    let repo_root = std::env::current_dir()?;
    
    // Get the path to remove
    let target = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo_root.join(path)
    };
    
    let mut removed = Vec::new();
    
    // Check if it's a specific file
    if target.is_file() {
        let file_str = target
            .strip_prefix(&repo_root)
            .unwrap_or(&target)
            .to_string_lossy()
            .to_string();
        
        if store.files.remove(&file_str).is_some() {
            removed.push(file_str.clone());
            // Also remove the .own file
            let own_path = PathBuf::from(".own").join(format!("{}.own", file_str));
            if own_path.exists() {
                fs::remove_file(&own_path)?;
            }
        }
    } else if target.is_dir() {
        // Remove all files in directory
        let prefix = target
            .strip_prefix(&repo_root)
            .unwrap_or(&target)
            .to_string_lossy()
            .to_string();
        
        let keys_to_remove: Vec<String> = store.files.keys()
            .filter(|k| k.starts_with(&prefix))
            .cloned()
            .collect();
        
        for key in keys_to_remove {
            store.files.remove(&key);
            removed.push(key.clone());
            // Remove .own file
            let own_path = PathBuf::from(".own").join(format!("{}.own", key));
            if own_path.exists() {
                fs::remove_file(&own_path)?;
            }
        }
    }
    
    if removed.is_empty() {
        println!("No tracked files found for: {}", path.display());
    } else {
        println!("Removed {} files from tracking:", removed.len());
        for file in &removed {
            println!("  {}", file);
        }
    }
    
    Ok(())
}

// ─── Add Command ────────────────────────────────────────────────────────────

pub fn add(dir: &Path) -> Result<()> {
    let store = OwnershipStore::load()?;
    let repo_root = std::env::current_dir()?;
    
    println!("=== Scanning {} ===\n", dir.display());
    
    let files = crate::ignore::list_files(dir, &repo_root);
    
    // Build tree structure
    let mut tree: BTreeMap<String, Vec<(PathBuf, usize, usize)>> = BTreeMap::new(); // dir -> (file, lines, owned)
    let mut total_lines = 0;
    let mut total_owned = 0;
    
    for file in &files {
        // Get relative path for lookup
        let file_str = file.strip_prefix(&repo_root).unwrap_or(file).to_string_lossy().to_string();
        let ownership = store.get_file(&file_str);
        
        let total = fs::read_to_string(file)
            .map(|c| c.lines().count())
            .unwrap_or(0);
        
        let (reviewed, _questioned, approved, _rejected) = ownership.line_states.values().fold(
            (0, 0, 0, 0),
            |(r, q, a, x), state| match state {
                LineState::Reviewed => (r + 1, q, a, x),
                LineState::Questioned => (r, q + 1, a, x),
                LineState::Approved => (r, q, a + 1, x),
                LineState::Rejected => (r, q, a, x + 1),
            },
        );
        let owned = reviewed + approved;
        
        total_lines += total;
        total_owned += owned;
        
        // Get directory relative to scan dir
        let rel_path = file.strip_prefix(&repo_root).unwrap_or(file);
        let dir_name = rel_path.parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| ".".to_string());
        
        tree.entry(dir_name)
            .or_default()
            .push((rel_path.to_path_buf(), total, owned));
    }
    
    // Print tree
    for (dir, files) in &tree {
        // Calculate directory ownership
        let dir_lines: usize = files.iter().map(|(_, l, _)| l).sum();
        let dir_owned: usize = files.iter().map(|(_, _, o)| o).sum();
        let dir_pct = if dir_lines > 0 {
            dir_owned as f64 / dir_lines as f64 * 100.0
        } else {
            0.0
        };
        
        println!("{}/ ({:.0}%)", dir, dir_pct);
        
        for (file, lines, owned) in files {
            let file_name = file.file_name().unwrap_or_default().to_string_lossy();
            let pct = if *lines > 0 {
                *owned as f64 / *lines as f64 * 100.0
            } else {
                0.0
            };
            
            let indicator = if *owned == *lines && *lines > 0 {
                "✓"
            } else if *owned > 0 {
                "~"
            } else {
                " "
            };
            
            println!("  {} {:<40} {:>5} lines {:>5.0}%", indicator, file_name, lines, pct);
        }
        println!();
    }
    
    // Summary
    let total_pct = if total_lines > 0 {
        total_owned as f64 / total_lines as f64 * 100.0
    } else {
        0.0
    };
    
    println!("=== Summary ===");
    println!("Files: {} total", tree.values().map(|v| v.len()).sum::<usize>());
    println!("Lines: {} total", total_lines);
    println!("Ownership: {:.1}%", total_pct);
    
    Ok(())
}
