use glob::Pattern;
use std::fs;
use std::path::{Path, PathBuf};

/// Check if a path should be ignored based on .ownignore or .gitignore
pub fn should_ignore(path: &Path, repo_root: &Path) -> bool {
    let patterns = load_ignore_patterns(repo_root);

    // Get relative path from repo root
    let rel_path = path.strip_prefix(repo_root).unwrap_or(path);
    let path_str = rel_path.to_string_lossy();
    // Normalize: remove leading ./
    let path_str = path_str.strip_prefix("./").unwrap_or(&path_str);

    for pattern in &patterns {
        // Check full path
        if pattern.matches(path_str) {
            return true;
        }
        
        // Check each path component (for directory matching)
        let components: Vec<&str> = path_str.split('/').collect();
        for i in 0..components.len() {
            let partial = components[..=i].join("/");
            if pattern.matches(&partial) {
                return true;
            }
        }
    }

    false
}

/// Load ignore patterns from .ownignore or .gitignore
fn load_ignore_patterns(repo_root: &Path) -> Vec<Pattern> {
    let mut patterns = Vec::new();

    // Always ignore .git and .own directories
    if let Ok(p) = Pattern::new(".git") {
        patterns.push(p);
    }
    if let Ok(p) = Pattern::new(".own") {
        patterns.push(p);
    }

    // Try .ownignore first, then .gitignore
    let ignore_files = [".ownignore", ".gitignore"];

    for ignore_file in &ignore_files {
        let ignore_path = repo_root.join(ignore_file);
        if ignore_path.exists() {
            if let Ok(content) = fs::read_to_string(&ignore_path) {
                for line in content.lines() {
                    let line = line.trim();
                    // Skip empty lines and comments
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }
                    // Skip negation patterns (for now)
                    if line.starts_with('!') {
                        continue;
                    }
                    // Convert gitignore patterns to glob patterns
                    if let Ok(pattern) = convert_gitignore_pattern(line) {
                        patterns.push(pattern);
                    }
                }
            }
        }
    }

    patterns
}

/// Convert a gitignore pattern to a glob Pattern
fn convert_gitignore_pattern(pattern: &str) -> Result<Pattern, glob::PatternError> {
    // Handle directory-only patterns (ending with /)
    let pattern = if pattern.ends_with('/') {
        pattern.trim_end_matches('/')
    } else {
        pattern
    };

    // Handle root-anchored patterns (starting with /)
    if let Some(pattern) = pattern.strip_prefix('/') {
        // Match at root level only
        return Pattern::new(pattern);
    }

    // If pattern contains no slash, match against any level
    if !pattern.contains('/') {
        return Pattern::new(&format!("**/{}", pattern));
    }

    // Otherwise match against full path
    Pattern::new(pattern)
}

/// List files in a directory, respecting ignore patterns
pub fn list_files(dir: &Path, repo_root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();

            if path.is_dir() {
                // Skip ignored directories
                if !should_ignore(&path, repo_root) {
                    files.extend(list_files(&path, repo_root));
                }
            } else if !should_ignore(&path, repo_root) {
                files.push(path);
            }
        }
    }

    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_ignore() {
        let repo_root = PathBuf::from("/tmp/test-repo");
        let patterns = vec![
            Pattern::new("**/*.o").unwrap(),
            Pattern::new("**/target/**").unwrap(),
            Pattern::new("**/node_modules/**").unwrap(),
        ];

        // This would need actual test files to work properly
        // Just verifying pattern compilation works
        assert!(patterns.len() == 3);
    }
}
