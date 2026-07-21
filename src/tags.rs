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

pub fn create(name: &str, color: Option<&str>) -> Result<()> {
    let mut store = TagStore::load();

    if store.tags.contains_key(name) {
        anyhow::bail!("Tag already exists: {}", name);
    }

    let final_color = match color {
        Some(c) => c.to_string(),
        None => generate_color(name),
    };

    store.tags.insert(
        name.to_string(),
        Tag {
            name: name.to_string(),
            color: final_color.clone(),
        },
    );
    store.save()?;

    println!("Created tag: {} ({})", name, final_color);
    Ok(())
}

fn generate_color(name: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    let hash = hasher.finish();

    // Use different bits for more variety
    let hue = ((hash >> 8) % 360) as u16;
    let saturation = 70 + ((hash >> 16) % 20) as u16; // 70-89%
    let lightness = 45 + ((hash >> 24) % 15) as u16; // 45-59%

    format!("hsl({}, {}%, {}%)", hue, saturation, lightness)
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
