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
        /// Color (hex, e.g. #ff0000)
        color: String,
    },
    /// Delete a tag
    Delete {
        /// Tag name
        name: String,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Add { file } => store::add(&file),
        Commands::Review { file } => review::run(&file),
        Commands::Status => store::status(),
        Commands::Tags { command } => match command {
            TagCommands::List => tags::list(),
            TagCommands::Create { name, color } => tags::create(&name, &color),
            TagCommands::Delete { name } => tags::delete(&name),
        },
    }
}
