mod store;
mod tags;
mod review;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "own", about = "Track code ownership, line by line")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Add a file to track
    Add {
        /// File path
        file: PathBuf,
    },
    /// Review a file
    Review {
        /// File path
        file: PathBuf,
    },
    /// Show ownership status
    Status,
    /// Extract ownership data
    Extract {
        /// Output format: md (markdown) or json
        #[arg(short, long, default_value = "md")]
        format: String,
    },
    /// Manage tags
    Tags {
        #[command(subcommand)]
        command: TagCommands,
    },
}

#[derive(Subcommand)]
enum TagCommands {
    /// List all tags
    List,
    /// Create a new tag
    Create {
        /// Tag name
        name: String,
        /// Color (hex, e.g. #ff0000 or hsl). Auto-generated if not provided.
        color: Option<String>,
    },
    /// Delete a tag
    Delete {
        /// Tag name
        name: String,
    },
}

fn validate_file(path: &PathBuf, command: &str) -> Result<()> {
    if !path.exists() {
        anyhow::bail!("{}: {} does not exist", command, path.display());
    }
    if path.is_dir() {
        anyhow::bail!("{}: {} is a directory, not a file", command, path.display());
    }
    Ok(())
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { file } => {
            validate_file(&file, "add")?;
            store::add(&file)
        }
        Commands::Review { file } => {
            validate_file(&file, "review")?;
            review::run(&file)
        }
        Commands::Status => store::status(),
        Commands::Extract { format } => store::extract(&format),
        Commands::Tags { command } => match command {
            TagCommands::List => tags::list(),
            TagCommands::Create { name, color } => tags::create(&name, color.as_deref()),
            TagCommands::Delete { name } => tags::delete(&name),
        },
    }
}
