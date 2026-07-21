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
    pub author: Option<String>,
    pub timestamp: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Annotation {
    pub start_line: usize,
    pub end_line: usize,
    pub note: String,
}

/// A review entry from a specific author
#[derive(Debug, Clone)]
pub struct ReviewEntry {
    pub author: String,
    pub timestamp: String,
    pub tags: Vec<String>,
    pub note: Option<String>,
}

impl ReviewEntry {
    pub fn parse(line: &str) -> Option<Self> {
        // Format: @10:alice:2024-01-20T10:00:00Z:reviewed,approved:optional note
        if !line.starts_with('@') {
            return None;
        }
        let rest = &line[1..];
        let parts: Vec<&str> = rest.splitn(5, ':').collect();
        if parts.len() < 4 {
            return None;
        }
        Some(ReviewEntry {
            author: parts[1].to_string(),
            timestamp: parts[2].to_string(),
            tags: parts[3].split(',').map(|s| s.trim().to_string()).collect(),
            note: if parts.len() > 4 && !parts[4].is_empty() {
                Some(parts[4].to_string())
            } else {
                None
            },
        })
    }

    pub fn serialize(&self, line: usize) -> String {
        let note = self.note.as_deref().unwrap_or("");
        format!("@{}:{}:{}:{}:{}", line, self.author, self.timestamp, self.tags.join(","), note)
    }
}

#[derive(Debug, Default)]
pub struct FileOwnership {
    pub snapshot: Option<String>,  // Hash of source file when reviewed
    pub entries: BTreeMap<usize, LineEntry>,
    pub reviews: BTreeMap<usize, Vec<ReviewEntry>>,  // Multiple reviews per line
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
        static EMPTY: FileOwnership = FileOwnership { snapshot: None, entries: std::collections::BTreeMap::new(), reviews: std::collections::BTreeMap::new(), annotations: Vec::new() };
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

            // Parse review entry (multi-author) or annotation
            if line.starts_with('@') {
                // Try review entry first: @line:author:timestamp:tags:note
                if let Some(review) = ReviewEntry::parse(line) {
                    let rest = &line[1..]; // remove '@'
                    if let Some(colon_pos) = rest.find(':') {
                        if let Ok(line_num) = rest[..colon_pos].parse::<usize>() {
                            ownership.reviews.entry(line_num).or_default().push(review);
                        }
                    }
                } else if let Some(ann) = Annotation::parse(line) {
                    // Try annotation: @start-end:note
                    ownership.annotations.push(ann);
                }
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

        // Build map: content_hash -> line_number for new file (first occurrence)
        let mut hash_to_line: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for (i, hash) in new_hashes.iter().enumerate() {
            // Only insert first occurrence
            hash_to_line.entry(hash.clone()).or_insert(i + 1); // 1-based
        }

        // Try to re-anchor each entry
        let mut new_entries = BTreeMap::new();
        let mut deleted_lines = Vec::new();
        
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
                // Content was deleted - remove the annotation
                    deleted_lines.push(*old_line);
                }
            } else {
                // No content hash - keep at original line
                new_entries.insert(*old_line, entry.clone());
            }
        }
        
        // Remove reviews for deleted lines
        for line in &deleted_lines {
            self.reviews.remove(line);
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

        // Write line entries (legacy format)
        for (_, entry) in &self.entries {
            lines.push(entry.serialize());
        }

        // Write reviews (multi-author format)
        for (line_num, reviews) in &self.reviews {
            for review in reviews {
                lines.push(review.serialize(*line_num));
            }
        }

        // Write annotations
        for ann in &self.annotations {
            lines.push(ann.serialize());
        }

        lines.join("\n")
    }

    /// Merge another FileOwnership into this one
    pub fn merge(&mut self, other: &FileOwnership) {
        // Merge reviews - other's reviews are added
        for (line_num, reviews) in &other.reviews {
            let entry = self.reviews.entry(*line_num).or_default();
            for review in reviews {
                // Check if same author already reviewed this line
                if let Some(existing) = entry.iter_mut().find(|r| r.author == review.author) {
                    // Update if newer
                    if review.timestamp > existing.timestamp {
                        *existing = review.clone();
                    }
                } else {
                    entry.push(review.clone());
                }
            }
        }

        // Merge annotations
        for ann in &other.annotations {
            // Simple: add if not exists
            if !self.annotations.iter().any(|a| a.start_line == ann.start_line && a.end_line == ann.end_line && a.note == ann.note) {
                self.annotations.push(ann.clone());
            }
        }
    }

    /// Get combined tags for a line from all reviews
    pub fn get_line_tags(&self, line: usize) -> Vec<String> {
        let mut all_tags = std::collections::HashSet::new();
        
        // From legacy entries
        if let Some(entry) = self.entries.get(&line) {
            for tag in &entry.tags {
                all_tags.insert(tag.clone());
            }
        }
        
        // From reviews
        if let Some(reviews) = self.reviews.get(&line) {
            for review in reviews {
                for tag in &review.tags {
                    all_tags.insert(tag.clone());
                }
            }
        }
        
        let mut tags: Vec<String> = all_tags.into_iter().collect();
        tags.sort();
        tags
    }

    /// Get review count for a line
    pub fn get_review_count(&self, line: usize) -> usize {
        self.reviews.get(&line).map(|r| r.len()).unwrap_or(0)
    }

    /// Get all unique reviewers
    pub fn get_reviewers(&self) -> Vec<String> {
        let mut reviewers = std::collections::HashSet::new();
        for reviews in self.reviews.values() {
            for review in reviews {
                reviewers.insert(review.author.clone());
            }
        }
        let mut reviewers: Vec<String> = reviewers.into_iter().collect();
        reviewers.sort();
        reviewers
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

    pub fn set_line(&mut self, line: usize, tags: Vec<String>, content: Option<&str>, author: &str) {
        // Update legacy entry
        if tags.is_empty() {
            self.entries.remove(&line);
        } else {
            let entry = match content {
                Some(c) => LineEntry::with_content(line, tags.clone(), c, author),
                None => {
                    let existing = self.entries.get(&line).and_then(|e| e.content_hash.clone());
                    LineEntry {
                        line,
                        tags: tags.clone(),
                        content_hash: existing,
                        author: Some(author.to_string()),
                        timestamp: Some(chrono::Utc::now().to_rfc3339()),
                    }
                }
            };
            self.entries.insert(line, entry);
        }

        // Also add review entry
        let review = ReviewEntry {
            author: author.to_string(),
            timestamp: chrono::Utc::now().to_rfc3339(),
            tags,
            note: None,
        };
        self.reviews.entry(line).or_default().push(review);
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
        // Format: 5:r,approved:content_hash:author:timestamp
        // or: 5:r,approved:content_hash
        // or: 5:r,approved
        let parts: Vec<&str> = line.splitn(5, ':').collect();
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
        let author = if parts.len() > 3 && !parts[3].is_empty() {
            Some(parts[3].to_string())
        } else {
            None
        };
        let timestamp = if parts.len() > 4 && !parts[4].is_empty() {
            Some(parts[4].to_string())
        } else {
            None
        };

        Some(LineEntry {
            line: line_num,
            tags,
            content_hash,
            author,
            timestamp,
        })
    }

    pub fn serialize(&self) -> String {
        let hash = self.content_hash.as_deref().unwrap_or("");
        let author = self.author.as_deref().unwrap_or("");
        let ts = self.timestamp.as_deref().unwrap_or("");
        format!("{}:{}:{}:{}:{}", self.line, self.tags.join(","), hash, author, ts)
    }

    /// Create entry with content hash from line content
    pub fn with_content(line: usize, tags: Vec<String>, content: &str, author: &str) -> Self {
        let mut hasher = DefaultHasher::new();
        content.hash(&mut hasher);
        let content_hash = format!("{:x}", hasher.finish());
        LineEntry {
            line,
            tags,
            content_hash: Some(content_hash),
            author: Some(author.to_string()),
            timestamp: Some(chrono::Utc::now().to_rfc3339()),
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

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_line_entry_parse_simple() {
        let entry = LineEntry::parse("5:reviewed").unwrap();
        assert_eq!(entry.line, 5);
        assert_eq!(entry.tags, vec!["reviewed"]);
        assert!(entry.content_hash.is_none());
    }

    #[test]
    fn test_line_entry_parse_multiple_tags() {
        let entry = LineEntry::parse("10:reviewed,approved").unwrap();
        assert_eq!(entry.line, 10);
        assert_eq!(entry.tags, vec!["reviewed", "approved"]);
    }

    #[test]
    fn test_line_entry_parse_with_hash() {
        let entry = LineEntry::parse("3:r,a:abc123def").unwrap();
        assert_eq!(entry.line, 3);
        assert_eq!(entry.tags, vec!["r", "a"]);
        assert_eq!(entry.content_hash, Some("abc123def".to_string()));
    }

    #[test]
    fn test_line_entry_parse_invalid() {
        assert!(LineEntry::parse("invalid").is_none());
        assert!(LineEntry::parse("abc:def").is_none());
        assert!(LineEntry::parse("").is_none());
    }

    #[test]
    fn test_line_entry_serialize_simple() {
        let entry = LineEntry {
            line: 5,
            tags: vec!["reviewed".to_string()],
            content_hash: None,
            author: None,
            timestamp: None,
        };
        assert_eq!(entry.serialize(), "5:reviewed:::");
    }

    #[test]
    fn test_line_entry_serialize_with_hash() {
        let entry = LineEntry {
            line: 10,
            tags: vec!["r".to_string(), "a".to_string()],
            content_hash: Some("abc123".to_string()),
            author: Some("alice".to_string()),
            timestamp: Some("2024-01-20T10:00:00Z".to_string()),
        };
        assert_eq!(entry.serialize(), "10:r,a:abc123:alice:2024-01-20T10:00:00Z");
    }

    #[test]
    fn test_line_entry_roundtrip() {
        let original = "7:reviewed,approved:hash123:alice:2024-01-20T10:00:00Z";
        let entry = LineEntry::parse(original).unwrap();
        let serialized = entry.serialize();
        assert_eq!(original, serialized);
    }

    #[test]
    fn test_line_entry_with_content() {
        let entry = LineEntry::with_content(1, vec!["test".to_string()], "hello world", "alice");
        assert_eq!(entry.line, 1);
        assert_eq!(entry.tags, vec!["test"]);
        assert!(entry.content_hash.is_some());
        assert_eq!(entry.author, Some("alice".to_string()));
        // Same content should produce same hash
        let entry2 = LineEntry::with_content(2, vec!["test".to_string()], "hello world", "bob");
        assert_eq!(entry.content_hash, entry2.content_hash);
    }

    #[test]
    fn test_annotation_parse() {
        let ann = Annotation::parse("@5-10:this is a note").unwrap();
        assert_eq!(ann.start_line, 5);
        assert_eq!(ann.end_line, 10);
        assert_eq!(ann.note, "this is a note");
    }

    #[test]
    fn test_annotation_parse_single_line() {
        let ann = Annotation::parse("@3-3:single line note").unwrap();
        assert_eq!(ann.start_line, 3);
        assert_eq!(ann.end_line, 3);
    }

    #[test]
    fn test_annotation_parse_invalid() {
        assert!(Annotation::parse("invalid").is_none());
        assert!(Annotation::parse("5-10:note").is_none());
        assert!(Annotation::parse("@5:note").is_none());
        assert!(Annotation::parse("").is_none());
    }

    #[test]
    fn test_annotation_serialize() {
        let ann = Annotation {
            start_line: 5,
            end_line: 10,
            note: "test note".to_string(),
        };
        assert_eq!(ann.serialize(), "@5-10:test note");
    }

    #[test]
    fn test_annotation_roundtrip() {
        let original = "@1-5:important code";
        let ann = Annotation::parse(original).unwrap();
        assert_eq!(ann.serialize(), original);
    }

    #[test]
    fn test_file_ownership_parse_empty() {
        let ownership = FileOwnership::parse("");
        assert!(ownership.snapshot.is_none());
        assert!(ownership.entries.is_empty());
        assert!(ownership.annotations.is_empty());
    }

    #[test]
    fn test_file_ownership_parse_full() {
        let content = "snapshot: abc123\n1:reviewed\n2:approved:hash\n@5-10:note";
        let ownership = FileOwnership::parse(content);
        assert_eq!(ownership.snapshot, Some("abc123".to_string()));
        assert_eq!(ownership.entries.len(), 2);
        assert_eq!(ownership.annotations.len(), 1);
    }

    #[test]
    fn test_file_ownership_serialize() {
        let mut ownership = FileOwnership::default();
        ownership.snapshot = Some("test123".to_string());
        ownership.entries.insert(1, LineEntry {
            line: 1,
            tags: vec!["reviewed".to_string()],
            content_hash: None,
            author: None,
            timestamp: None,
        });
        let serialized = ownership.serialize();
        assert!(serialized.contains("snapshot: test123"));
        assert!(serialized.contains("1:reviewed"));
    }

    #[test]
    fn test_file_ownership_set_line() {
        let mut ownership = FileOwnership::default();
        
        // Add line with content
        ownership.set_line(5, vec!["reviewed".to_string()], Some("hello"), "alice");
        assert!(ownership.entries.contains_key(&5));
        assert!(ownership.entries[&5].content_hash.is_some());
        assert_eq!(ownership.entries[&5].author, Some("alice".to_string()));
        
        // Update line without content (should preserve hash)
        let old_hash = ownership.entries[&5].content_hash.clone();
        ownership.set_line(5, vec!["approved".to_string()], None, "bob");
        assert_eq!(ownership.entries[&5].tags, vec!["approved"]);
        assert_eq!(ownership.entries[&5].content_hash, old_hash);
        
        // Remove line
        ownership.set_line(5, vec![], None, "charlie");
        assert!(!ownership.entries.contains_key(&5));
    }

    #[test]
    fn test_file_ownership_reanchor_no_snapshot() {
        let mut ownership = FileOwnership::default();
        ownership.entries.insert(1, LineEntry {
            line: 1,
            tags: vec!["test".to_string()],
            content_hash: Some("hash".to_string()),
            author: None,
            timestamp: None,
        });
        
        let lines = vec!["new content".to_string()];
        ownership.reanchor(&lines);
        
        // Without snapshot, should not change
        assert!(ownership.entries.contains_key(&1));
    }

    #[test]
    fn test_file_ownership_reanchor_with_snapshot() {
        let mut ownership = FileOwnership::default();
        ownership.snapshot = Some("old_hash".to_string());
        
        // Add entry with content hash matching "line2"
        let entry = LineEntry::with_content(2, vec!["reviewed".to_string()], "line2", "alice");
        ownership.entries.insert(2, entry);
        
        // New file: line1, NEW_LINE, line2
        let lines = vec![
            "line1".to_string(),
            "NEW_LINE".to_string(),
            "line2".to_string(),
        ];
        
        ownership.reanchor(&lines);
        
        // Entry should move from line 2 to line 3
        assert!(!ownership.entries.contains_key(&2));
        assert!(ownership.entries.contains_key(&3));
        assert_eq!(ownership.entries[&3].tags, vec!["reviewed"]);
    }

    #[test]
    fn test_file_ownership_reanchor_deleted_line() {
        let mut ownership = FileOwnership::default();
        ownership.snapshot = Some("old_hash".to_string());
        
        // Add entry for "deleted_line"
        let entry = LineEntry::with_content(2, vec!["test".to_string()], "deleted_line", "alice");
        ownership.entries.insert(2, entry);
        
        // New file without that line
        let lines = vec![
            "line1".to_string(),
            "line3".to_string(),
        ];
        
        ownership.reanchor(&lines);
        
        // Entry should be removed (content no longer exists)
        assert!(!ownership.entries.contains_key(&2));
    }

    #[test]
    fn test_file_ownership_reanchor_duplicate_content() {
        let mut ownership = FileOwnership::default();
        ownership.snapshot = Some("old_hash".to_string());
        
        // Add entry for "same_line"
        let entry = LineEntry::with_content(1, vec!["test".to_string()], "same_line", "alice");
        ownership.entries.insert(1, entry);
        
        // New file with duplicate content
        let lines = vec![
            "same_line".to_string(),
            "other".to_string(),
            "same_line".to_string(),
        ];
        
        ownership.reanchor(&lines);
        
        // Should match first occurrence
        assert!(ownership.entries.contains_key(&1));
        assert!(!ownership.entries.contains_key(&3));
    }

    #[test]
    fn test_file_ownership_reanchor_multiple_entries() {
        let mut ownership = FileOwnership::default();
        ownership.snapshot = Some("old_hash".to_string());
        
        // Add multiple entries
        ownership.entries.insert(1, LineEntry::with_content(1, vec!["a".to_string()], "line1", "alice"));
        ownership.entries.insert(3, LineEntry::with_content(3, vec!["b".to_string()], "line3", "bob"));
        
        // New file: line0, line1, line2, line3
        let lines = vec![
            "line0".to_string(),
            "line1".to_string(),
            "line2".to_string(),
            "line3".to_string(),
        ];
        
        ownership.reanchor(&lines);
        
        // line1 should move from 1->2, line3 should move from 3->4
        assert!(!ownership.entries.contains_key(&1));
        assert!(ownership.entries.contains_key(&2));
        assert!(!ownership.entries.contains_key(&3));
        assert!(ownership.entries.contains_key(&4));
        assert_eq!(ownership.entries[&2].tags, vec!["a"]);
        assert_eq!(ownership.entries[&4].tags, vec!["b"]);
    }

    #[test]
    fn test_store_get_file_default() {
        let store = Store::default();
        let file = store.get_file("nonexistent");
        assert!(file.entries.is_empty());
        assert!(file.annotations.is_empty());
    }

    #[test]
    fn test_hash_consistency() {
        // Same content should produce same hash
        let hash1 = {
            let mut hasher = DefaultHasher::new();
            "test content".hash(&mut hasher);
            format!("{:x}", hasher.finish())
        };
        let hash2 = {
            let mut hasher = DefaultHasher::new();
            "test content".hash(&mut hasher);
            format!("{:x}", hasher.finish())
        };
        assert_eq!(hash1, hash2);
        
        // Different content should produce different hash
        let hash3 = {
            let mut hasher = DefaultHasher::new();
            "different content".hash(&mut hasher);
            format!("{:x}", hasher.finish())
        };
        assert_ne!(hash1, hash3);
    }
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
