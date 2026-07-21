use anyhow::Result;
use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

const TAGS_FILE: &str = ".own/tags";

#[derive(Debug, Clone)]
pub struct Tag {
    pub name: String,
    pub color: String,
}

#[derive(Debug, Default)]
pub struct TagStore {
    pub tags: BTreeMap<String, Tag>,
}

impl TagStore {
    pub fn load() -> Self {
        let mut store = Self::default();

        let path = PathBuf::from(TAGS_FILE);
        if !path.exists() {
            return store;
        }

        if let Ok(content) = fs::read_to_string(&path) {
            for line in content.lines() {
                let line = line.trim();
                if line.is_empty() || line.starts_with('#') {
                    continue;
                }

                // Format: name:#color
                if let Some((name, color)) = line.split_once(':') {
                    store.tags.insert(
                        name.to_string(),
                        Tag {
                            name: name.to_string(),
                            color: color.to_string(),
                        },
                    );
                }
            }
        }

        store
    }

    pub fn save(&self) -> Result<()> {
        let path = PathBuf::from(TAGS_FILE);

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let mut lines = Vec::new();
        for (_, tag) in &self.tags {
            lines.push(format!("{}:{}", tag.name, tag.color));
        }

        fs::write(&path, lines.join("\n"))?;
        Ok(())
    }
}

// ─── CLI Commands ────────────────────────────────────────────────────────────

pub fn list() -> Result<()> {
    let store = TagStore::load();

    if store.tags.is_empty() {
        println!("No tags defined. Create one: own tags create <name> <color>");
        return Ok(());
    }

    println!("=== Tags ===\n");

    for (_, tag) in &store.tags {
        println!("{:<20} {}", tag.name, tag.color);
    }

    Ok(())
}

pub fn create(name: &str, color: &str) -> Result<()> {
    let mut store = TagStore::load();

    if store.tags.contains_key(name) {
        anyhow::bail!("Tag already exists: {}", name);
    }

    store.tags.insert(
        name.to_string(),
        Tag {
            name: name.to_string(),
            color: color.to_string(),
        },
    );
    store.save()?;

    println!("Created tag: {} ({})", name, color);
    Ok(())
}

pub fn delete(name: &str) -> Result<()> {
    let mut store = TagStore::load();

    if store.tags.remove(name).is_none() {
        anyhow::bail!("Tag not found: {}", name);
    }

    store.save()?;
    println!("Deleted tag: {}", name);
    Ok(())
}
