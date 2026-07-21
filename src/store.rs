use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ─── Data Model ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LineEntry {
    pub line: usize,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Annotation {
    pub start_line: usize,
    pub end_line: usize,
    pub note: String,
}

#[derive(Debug, Default)]
pub struct FileOwnership {
    pub entries: BTreeMap<usize, LineEntry>,
    pub annotations: Vec<Annotation>,
}

#[derive(Debug, Default)]
pub struct Store {
    pub files: BTreeMap<String, FileOwnership>,
}

// ─── Store Implementation ────────────────────────────────────────────────────

impl Store {
    pub fn load() -> Result<Self> {
        let mut store = Self::default();
        let own_dir = PathBuf::from(".own");

        if !own_dir.exists() {
            return Ok(store);
        }

        // Recursively find all .own files
        Self::load_recursive(&own_dir, &own_dir, &mut store)?;
        Ok(store)
    }

    fn load_recursive(base: &Path, dir: &Path, store: &mut Store) -> Result<()> {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::load_recursive(base, &path, store)?;
                } else if path.extension().and_then(|e| e.to_str()) == Some("own") {
                    let rel = path.strip_prefix(base).unwrap_or(&path);
                    let source = rel.to_string_lossy().strip_suffix(".own").unwrap_or("").to_string();
                    if !source.is_empty() {
                        let content = fs::read_to_string(&path)?;
                        store.files.insert(source, FileOwnership::parse(&content));
                    }
                }
            }
        }
        Ok(())
    }

    pub fn save(&self) -> Result<()> {
        let own_dir = PathBuf::from(".own");
        fs::create_dir_all(&own_dir)?;

        for (file_name, ownership) in &self.files {
            let own_path = own_dir.join(format!("{}.own", file_name));

            // Create parent directories
            if let Some(parent) = own_path.parent() {
                fs::create_dir_all(parent)?;
            }

            let content = ownership.serialize();
            fs::write(&own_path, content)?;
        }

        Ok(())
    }

    pub fn get_file(&self, path: &str) -> &FileOwnership {
        static EMPTY: FileOwnership = FileOwnership { entries: std::collections::BTreeMap::new(), annotations: Vec::new() };
        self.files.get(path).unwrap_or(&EMPTY)
    }

    pub fn get_file_mut(&mut self, path: &str) -> &mut FileOwnership {
        self.files.entry(path.to_string()).or_default()
    }
}

// ─── FileOwnership Implementation ────────────────────────────────────────────

impl FileOwnership {
    pub fn parse(content: &str) -> Self {
        let mut ownership = Self::default();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(ann) = Annotation::parse(line) {
                ownership.annotations.push(ann);
            } else if let Some(entry) = LineEntry::parse(line) {
                ownership.entries.insert(entry.line, entry);
            }
        }

        ownership
    }

    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();

        for (_, entry) in &self.entries {
            lines.push(entry.serialize());
        }

        for ann in &self.annotations {
            lines.push(ann.serialize());
        }

        lines.join("\n")
    }

    pub fn set_line(&mut self, line: usize, tags: Vec<String>) {
        if tags.is_empty() {
            self.entries.remove(&line);
        } else {
            self.entries.insert(line, LineEntry { line, tags });
        }
    }

    pub fn get_annotation(&self, line: usize) -> Option<&Annotation> {
        self.annotations.iter().find(|a| line >= a.start_line && line <= a.end_line)
    }

    pub fn set_annotation(&mut self, start: usize, end: usize, note: String) {
        self.annotations.retain(|a| !(a.start_line <= end && a.end_line >= start));
        self.annotations.push(Annotation { start_line: start, end_line: end, note });
    }

    pub fn remove_annotation(&mut self, line: usize) {
        self.annotations.retain(|a| !(line >= a.start_line && line <= a.end_line));
    }
}

// ─── LineEntry Implementation ────────────────────────────────────────────────

impl LineEntry {
    pub fn parse(line: &str) -> Option<Self> {
        // Format: 5:r,approved
        let parts: Vec<&str> = line.splitn(2, ':').collect();
        if parts.len() < 2 {
            return None;
        }

        let line_num: usize = parts[0].parse().ok()?;
        let tags: Vec<String> = parts[1].split(',').map(|s| s.trim().to_string()).collect();

        Some(LineEntry { line: line_num, tags })
    }

    pub fn serialize(&self) -> String {
        format!("{}:{}", self.line, self.tags.join(","))
    }
}

// ─── Annotation Implementation ───────────────────────────────────────────────

impl Annotation {
    pub fn parse(line: &str) -> Option<Self> {
        // Format: @5-10:note text here
        if !line.starts_with('@') {
            return None;
        }

        let rest = &line[1..];
        let parts: Vec<&str> = rest.splitn(2, ':').collect();
        if parts.len() < 2 {
            return None;
        }

        let range_parts: Vec<&str> = parts[0].split('-').collect();
        if range_parts.len() != 2 {
            return None;
        }

        let start: usize = range_parts[0].parse().ok()?;
        let end: usize = range_parts[1].parse().ok()?;
        let note = parts[1].to_string();

        Some(Annotation {
            start_line: start,
            end_line: end,
            note,
        })
    }

    pub fn serialize(&self) -> String {
        format!("@{}-{}:{}", self.start_line, self.end_line, self.note)
    }
}

// ─── CLI Commands ────────────────────────────────────────────────────────────

pub fn add(file: &Path) -> Result<()> {
    let file_str = file.to_string_lossy().to_string();

    // Check file exists
    if !file.exists() {
        anyhow::bail!("File not found: {}", file.display());
    }

    let mut store = Store::load()?;
    store.get_file_mut(&file_str);
    store.save()?;

    println!("Added: {}", file_str);
    Ok(())
}

pub fn status() -> Result<()> {
    let store = Store::load()?;

    if store.files.is_empty() {
        println!("No files tracked. Run: own add <file>");
        return Ok(());
    }

    println!("=== Ownership Status ===\n");

    for (file, ownership) in &store.files {
        // Read source file to check for empty lines
        let source = std::fs::read_to_string(file).unwrap_or_default();
        let lines: Vec<&str> = source.lines().collect();

        let total = ownership.entries.len();
        let reviewed = ownership.entries.values()
            .filter(|e| {
                // Skip empty lines
                if let Some(line_content) = lines.get(e.line - 1) {
                    if line_content.trim().is_empty() {
                        return false;
                    }
                }
                e.tags.contains(&"r".to_string()) || e.tags.contains(&"a".to_string())
            })
            .count();

        // Count non-empty lines
        let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();

        let pct = if non_empty > 0 {
            reviewed as f64 / non_empty as f64 * 100.0
        } else {
            0.0
        };

        println!("{:<40} {:>5.1}%", file, pct);
    }

    Ok(())
}
