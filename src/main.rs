mod own;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "own", about = "Track your code ownership - human review, line by line")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Review a file line by line
    Review {
        /// File to review
        file: PathBuf,
    },
    /// Show ownership status
    Status {
        /// Specific file to check (optional)
        file: Option<PathBuf>,
    },
    /// Initialize .own directory
    Init,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Review { file } => {
            own::review(&file)?;
        }
        Commands::Status { file } => {
            own::status(file.as_deref())?;
        }
        Commands::Init => {
            own::init()?;
        }
    }

    Ok(())
}
