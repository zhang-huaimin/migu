use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "migu", version, about = "Cross-shell command history manager")]
pub struct Cli {
    /// Max results to show (default: 10, max: 50)
    #[arg(short = 'n', long = "limit", default_value_t = 50, value_parser = clap::value_parser!(u64).range(1..=100))]
    pub limit: u64,

    /// Override database path
    #[arg(short = 'd', long = "database")]
    pub database: Option<String>,

    /// Do not deduplicate consecutive duplicates in recent mode
    #[arg(long = "no-dedup")]
    pub no_dedup: bool,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Record a command to the database (called by shell hooks)
    Add {
        /// The command to record (after --)
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, num_args = 1..)]
        command: Vec<String>,

        /// Working directory
        #[arg(long, default_value_t = String::new())]
        cwd: String,

        /// Exit code of the command
        #[arg(long)]
        exit_code: Option<i32>,

        /// Hostname
        #[arg(long)]
        hostname: Option<String>,

        /// Shell name (bash/zsh/fish)
        #[arg(long)]
        shell: Option<String>,

        /// Session identifier
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Output shell configuration snippet
    Init {
        /// Target shell: bash, zsh, or fish
        #[arg(value_parser = ["bash", "zsh", "fish"])]
        shell: String,
    },
    /// Import existing shell history into the database
    Import {
        /// Source shell: bash, zsh, or fish
        #[arg(value_parser = ["bash", "zsh", "fish"])]
        shell: String,
    },
    /// List command history to stdout
    List {
        /// Sort by frequency instead of time
        #[arg(short = 'f')]
        frequency: bool,

        /// Expand: show full command
        #[arg(short = 'z')]
        expand: bool,

        /// Show full ISO timestamp instead of relative time
        #[arg(short = 't')]
        timestamp: bool,

        /// Number of entries (overrides global --limit)
        #[arg(short = 'l')]
        limit: Option<usize>,
    },
}
