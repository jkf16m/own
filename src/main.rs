mod own;
mod ignore;

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
    /// Add files to track ownership
    Add {
        /// Directory or file to add (default: current directory)
        path: Option<PathBuf>,
    },
    /// Extract rejected or approved lines with annotations for AI review
    Extract {
        /// Extract rejected lines (default)
        #[arg(short = 'r', long = "rejected", conflicts_with = "approved")]
        rejected: bool,
        /// Extract approved lines
        #[arg(short = 'a', long = "approved", conflicts_with = "rejected")]
        approved: bool,
        /// Specific file to extract from (optional, otherwise all files)
        file: Option<PathBuf>,
    },
    /// Remove files from tracking
    Remove {
        /// File or directory to remove from tracking
        path: PathBuf,
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
        Commands::Add { path } => {
            let add_path = path.unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
            own::add(&add_path)?;
        }
        Commands::Extract { rejected, approved, file } => {
            let state = if rejected {
                own::ExtractState::Rejected
            } else if approved {
                own::ExtractState::Approved
            } else {
                own::ExtractState::Rejected // default
            };
            own::extract(state, file.as_deref())?;
        }
        Commands::Remove { path } => {
            own::remove(&path)?;
        }
        Commands::Init => {
            own::init()?;
        }
    }

    Ok(())
}
