use std::path::PathBuf;

use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use hephos_cli::{format_json, format_table, generate_openapi, scaffold_app, scan_routes};

#[derive(Parser, Debug)]
#[command(
    name = "hephos",
    version,
    about = "The backend framework built only for coding agents"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a new Hephos application following the canonical AGENTS.md layout.
    New(NewArgs),

    /// Inspect and print the application's route table.
    Routes(RoutesArgs),

    /// Generate and print the OpenAPI 3.1.0 specification for the application.
    Openapi(OpenapiArgs),
}

#[derive(Args, Debug)]
struct NewArgs {
    /// Name of the new application (e.g. "my-app").
    name: String,

    /// Target directory for the new project (defaults to `./<name>`).
    #[arg(short, long)]
    path: Option<PathBuf>,

    /// Path to a local hephos crate checkout (for offline development/tests).
    #[arg(long)]
    local_path: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct RoutesArgs {
    /// Path to the application root directory (default: current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Output route table as formatted JSON.
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct OpenapiArgs {
    /// Path to the application root directory (default: current directory).
    #[arg(default_value = ".")]
    path: PathBuf,

    /// Title of the API.
    #[arg(long, default_value = "Hephos Application")]
    title: String,

    /// Version of the API.
    #[arg(long, default_value = "0.1.0")]
    version: String,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New(args) => {
            let target_dir = args.path.unwrap_or_else(|| PathBuf::from(&args.name));
            scaffold_app(&args.name, &target_dir, args.local_path.as_deref())?;
            println!(
                "Created Hephos application '{}' at {}",
                args.name,
                target_dir.display()
            );
            println!("\nNext steps:");
            println!("  cd {}", target_dir.display());
            println!("  cargo test");
            println!("  cargo run");
            Ok(())
        }
        Command::Routes(args) => {
            let routes = scan_routes(&args.path)?;
            if args.json {
                println!("{}", format_json(&routes)?);
            } else {
                print!("{}", format_table(&routes));
            }
            Ok(())
        }
        Command::Openapi(args) => {
            let routes = scan_routes(&args.path)?;
            let doc = generate_openapi(&routes, &args.title, &args.version);
            println!("{}", serde_json::to_string_pretty(&doc)?);
            Ok(())
        }
    }
}
