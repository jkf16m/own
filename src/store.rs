use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ─── Data Model ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LineEntry {
    pub line: usize,
    pub tags: Vec<String>,
    pub note: Option<String>,
}

#[derive(Debug, Default)]
pub struct FileOwnership {
    pub entries: BTreeMap<usize, LineEntry>,
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
        static EMPTY: FileOwnership = FileOwnership { entries: std::collections::BTreeMap::new() };
        self.files.get(path).unwrap_or(&EMPTY)
    }

    pub fn get_file_mut(&mut self, path: &str) -> &mut FileOwnership {
        self.files.entry(path.to_string()).or_default()
    }
}

// ─── FileOwnership Implementation ────────────────────────────────────────────

impl FileOwnership {
    const EMPTY: FileOwnership = FileOwnership {
        entries: BTreeMap::new(),
    };

    pub fn parse(content: &str) -> Self {
        let mut ownership = Self::default();

        for line in content.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            if let Some(entry) = LineEntry::parse(line) {
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

        lines.join("\n")
    }

    pub fn set_line(&mut self, line: usize, tags: Vec<String>, note: Option<String>) {
        if tags.is_empty() {
            self.entries.remove(&line);
        } else {
            self.entries.insert(line, LineEntry { line, tags, note });
        }
    }
}

// ─── LineEntry Implementation ────────────────────────────────────────────────

impl LineEntry {
    pub fn parse(line: &str) -> Option<Self> {
        // Format: 5:r,approved:note here
        // or: 5:r:note here
        // or: 5:r,approved
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 2 {
            return None;
        }

        let line_num: usize = parts[0].parse().ok()?;
        let tags_str = parts[1];
        let note = if parts.len() > 2 {
            Some(parts[2].to_string())
        } else {
            None
        };

        let tags: Vec<String> = tags_str.split(',').map(|s| s.trim().to_string()).collect();

        Some(LineEntry {
            line: line_num,
            tags,
            note,
        })
    }

    pub fn serialize(&self) -> String {
        let tags = self.tags.join(",");
        match &self.note {
            Some(note) => format!("{}:{}:{}", self.line, tags, note),
            None => format!("{}:{}", self.line, tags),
        }
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
