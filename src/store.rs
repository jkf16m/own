use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

// ─── Data Model ──────────────────────────────────────────────────────────────

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

#[derive(Debug, Clone)]
pub struct LineEntry {
    pub line: usize,
    pub tags: Vec<String>,
    pub content_hash: Option<String>,  // Hash of line content for tracking
}

#[derive(Debug, Clone)]
pub struct Annotation {
    pub start_line: usize,
    pub end_line: usize,
    pub note: String,
}

#[derive(Debug, Default)]
pub struct FileOwnership {
    pub snapshot: Option<String>,  // Hash of source file when reviewed
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
        static EMPTY: FileOwnership = FileOwnership { snapshot: None, entries: std::collections::BTreeMap::new(), annotations: Vec::new() };
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

            // Parse snapshot line
            if let Some(hash) = line.strip_prefix("snapshot: ") {
                ownership.snapshot = Some(hash.to_string());
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

    /// Re-anchor annotations to new file content
    pub fn reanchor(&mut self, new_lines: &[String]) {
        let Some(old_snapshot) = &self.snapshot else {
            return; // No snapshot to compare
        };

        // Compute hash of each new line
        let new_hashes: Vec<String> = new_lines.iter().map(|l| {
            let mut hasher = DefaultHasher::new();
            l.hash(&mut hasher);
            format!("{:x}", hasher.finish())
        }).collect();

        // Build map: content_hash -> line_number for new file
        let mut hash_to_line: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, hash) in new_hashes.iter().enumerate() {
            hash_to_line.insert(hash.clone(), i + 1); // 1-based
        }

        // Try to re-anchor each entry
        let mut new_entries = BTreeMap::new();
        for (old_line, entry) in &self.entries {
            if let Some(ref content_hash) = entry.content_hash {
                // Find where this content is now
                if let Some(&new_line) = hash_to_line.get(content_hash) {
                    // Content found at new line
                    let mut new_entry = entry.clone();
                    new_entry.line = new_line;
                    new_entries.insert(new_line, new_entry);
                    // Remove from map to prevent double-matching
                    hash_to_line.remove(content_hash);
                } else {
                    // Content deleted - keep at original line (may be wrong)
                    new_entries.insert(*old_line, entry.clone());
                }
            } else {
                // No content hash - keep at original line
                new_entries.insert(*old_line, entry.clone());
            }
        }

        self.entries = new_entries;

        // Re-anchor annotations similarly
        let mut new_annotations = Vec::new();
        for ann in &self.annotations {
            // For now, keep annotations at same relative position
            // TODO: use selected_text for better re-anchoring
            new_annotations.push(ann.clone());
        }
        self.annotations = new_annotations;
    }

    pub fn serialize(&self) -> String {
        let mut lines = Vec::new();

        // Write snapshot first
        if let Some(snapshot) = &self.snapshot {
            lines.push(format!("snapshot: {}", snapshot));
        }

        // Write line entries
        for (_, entry) in &self.entries {
            lines.push(entry.serialize());
        }

        // Write annotations
        for ann in &self.annotations {
            lines.push(ann.serialize());
        }

        lines.join("\n")
    }

    /// Compute hash of source file content
    pub fn compute_snapshot(file_path: &std::path::Path) -> Option<String> {
        let content = std::fs::read_to_string(file_path).ok()?;
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        Some(format!("{:x}", hasher.finish()))
    }

    /// Check if file has changed since last review
    pub fn is_stale(&self, file_path: &std::path::Path) -> bool {
        match (&self.snapshot, Self::compute_snapshot(file_path)) {
            (Some(stored), Some(current)) => stored != &current,
            (None, _) => false,  // No snapshot = not stale
            (_, None) => true,   // Can't read file = stale
        }
    }

    /// Update snapshot to current file content
    pub fn update_snapshot(&mut self, file_path: &std::path::Path) {
        self.snapshot = Self::compute_snapshot(file_path);
    }

    pub fn set_line(&mut self, line: usize, tags: Vec<String>, content: Option<&str>) {
        if tags.is_empty() {
            self.entries.remove(&line);
        } else {
            let entry = match content {
                Some(c) => LineEntry::with_content(line, tags, c),
                None => {
                    // Try to preserve existing content hash
                    let existing = self.entries.get(&line).and_then(|e| e.content_hash.clone());
                    LineEntry {
                        line,
                        tags,
                        content_hash: existing,
                    }
                }
            };
            self.entries.insert(line, entry);
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
        // Format: 5:r,approved:content_hash
        // or: 5:r,approved
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 2 {
            return None;
        }

        let line_num: usize = parts[0].parse().ok()?;
        let tags: Vec<String> = parts[1].split(',').map(|s| s.trim().to_string()).collect();
        let content_hash = if parts.len() > 2 && !parts[2].is_empty() {
            Some(parts[2].to_string())
        } else {
            None
        };

        Some(LineEntry {
            line: line_num,
            tags,
            content_hash,
        })
    }

    pub fn serialize(&self) -> String {
        match &self.content_hash {
            Some(hash) => format!("{}:{}:{}", self.line, self.tags.join(","), hash),
            None => format!("{}:{}", self.line, self.tags.join(",")),
        }
    }

    /// Create entry with content hash from line content
    pub fn with_content(line: usize, tags: Vec<String>, content: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());
        LineEntry {
            line,
            tags,
            content_hash: Some(content_hash),
        }
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

pub fn extract(format: &str) -> Result<()> {
    let store = Store::load()?;

    if store.files.is_empty() {
        println!("No files tracked.");
        return Ok(());
    }

    match format {
        "json" => extract_json(&store),
        _ => extract_markdown(&store),
    }
}

fn extract_json(store: &Store) -> Result<()> {
    let mut total_lines = 0;
    let mut total_reviewed = 0;
    let mut files = Vec::new();

    for (file, ownership) in &store.files {
        let source = std::fs::read_to_string(file).unwrap_or_default();
        let lines: Vec<&str> = source.lines().collect();
        let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();

        let reviewed = ownership.entries.values()
            .filter(|e| {
                if let Some(line_content) = lines.get(e.line - 1) {
                    if line_content.trim().is_empty() {
                        return false;
                    }
                }
                e.tags.contains(&"r".to_string()) || e.tags.contains(&"a".to_string())
            })
            .count();

        total_lines += non_empty;
        total_reviewed += reviewed;

        let pct = if non_empty > 0 {
            reviewed as f64 / non_empty as f64 * 100.0
        } else {
            0.0
        };

        let annotations: Vec<serde_json::Value> = ownership.annotations.iter().map(|a| {
            serde_json::json!({
                "start": a.start_line,
                "end": a.end_line,
                "note": a.note,
            })
        }).collect();

        files.push(serde_json::json!({
            "file": file,
            "total": non_empty,
            "reviewed": reviewed,
            "percentage": pct,
            "annotations": annotations,
        }));
    }

    let total_pct = if total_lines > 0 {
        total_reviewed as f64 / total_lines as f64 * 100.0
    } else {
        0.0
    };

    let output = serde_json::json!({
        "total_lines": total_lines,
        "total_reviewed": total_reviewed,
        "percentage": total_pct,
        "files": files,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn extract_markdown(store: &Store) -> Result<()> {
    println!("# Code Ownership Report\n");
    println!("## Summary\n");

    let mut total_lines = 0;
    let mut total_reviewed = 0;

    for (file, ownership) in &store.files {
        let source = std::fs::read_to_string(file).unwrap_or_default();
        let lines: Vec<&str> = source.lines().collect();
        let non_empty = lines.iter().filter(|l| !l.trim().is_empty()).count();

        let reviewed = ownership.entries.values()
            .filter(|e| {
                if let Some(line_content) = lines.get(e.line - 1) {
                    if line_content.trim().is_empty() {
                        return false;
                    }
                }
                e.tags.contains(&"r".to_string()) || e.tags.contains(&"a".to_string())
            })
            .count();

        total_lines += non_empty;
        total_reviewed += reviewed;

        let pct = if non_empty > 0 {
            reviewed as f64 / non_empty as f64 * 100.0
        } else {
            0.0
        };

        println!("| {} | {} | {} | {:.1}% |", file, reviewed, non_empty, pct);
    }

    let total_pct = if total_lines > 0 {
        total_reviewed as f64 / total_lines as f64 * 100.0
    } else {
        0.0
    };

    println!("\n**Total: {:.1}%** ({}/{})\n", total_pct, total_reviewed, total_lines);

    // Annotations
    let has_annotations = store.files.values().any(|f| !f.annotations.is_empty());
    if has_annotations {
        println!("## Annotations\n");

        for (file, ownership) in &store.files {
            if ownership.annotations.is_empty() {
                continue;
            }

            println!("### {}\n", file);

            let source = std::fs::read_to_string(file).unwrap_or_default();
            let source_lines: Vec<&str> = source.lines().collect();

            for ann in &ownership.annotations {
                println!("**Lines {}-{}:**\n", ann.start_line, ann.end_line);
                println!("```");
                for line_num in ann.start_line..=ann.end_line {
                    if let Some(line) = source_lines.get(line_num - 1) {
                        println!("{}", line);
                    }
                }
                println!("```\n");
                println!("> {}\n", ann.note);
            }
        }
    }

    Ok(())
}
