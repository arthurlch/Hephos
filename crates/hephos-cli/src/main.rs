use std::path::PathBuf;

use anyhow::{Result, bail};
use clap::{Args, Parser, Subcommand};
use hephos_cli::{
    DiagnosticStatus, check_project, format_json, format_table, generate_migration,
    generate_openapi, generate_resource, run_doctor, scaffold_app, scan_routes,
};

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

    /// Validate AGENTS.md architectural layering boundaries.
    Check(CheckArgs),

    /// Diagnose development environment and verify toolchain dependencies.
    Doctor(DoctorArgs),

    /// Generate canonical domain, repo, service, and API scaffold files.
    Generate(GenerateArgs),
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

    /// Scaffold database-ready application with Db in AppState and sqlx/uuid dependencies.
    #[arg(long)]
    db: bool,
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
    #[arg(long, default_value = "1.0.0")]
    version: String,
}

#[derive(Args, Debug)]
struct CheckArgs {
    /// Path to the application root directory (default: current directory).
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Args, Debug)]
struct DoctorArgs {
    /// Path to the project root directory (default: current directory).
    #[arg(default_value = ".")]
    path: PathBuf,
}

#[derive(Args, Debug)]
struct GenerateArgs {
    /// Type of artifact to generate (e.g. "resource").
    artifact_type: String,

    /// Name of the resource (e.g. "item", "user", "order").
    name: String,

    /// Path to the project root directory (default: current directory).
    #[arg(short, long, default_value = ".")]
    path: PathBuf,
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::New(args) => {
            let (pkg_name, target_dir) = match args.path {
                Some(p) => (args.name.clone(), p),
                None => {
                    let p = PathBuf::from(&args.name);
                    if let Some(file_name) = p.file_name().and_then(|n| n.to_str()) {
                        (file_name.to_string(), p)
                    } else {
                        (args.name.clone(), p)
                    }
                }
            };
            scaffold_app(&pkg_name, &target_dir, args.local_path.as_deref(), args.db)?;
            println!(
                "Created Hephos application '{}' at {}",
                pkg_name,
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
        Command::Check(args) => {
            println!(
                "Checking architectural boundaries at '{}'...",
                args.path.display()
            );
            let violations = check_project(&args.path)?;
            if violations.is_empty() {
                println!("✓ No boundary violations found.");
                Ok(())
            } else {
                eprintln!(
                    "✗ Found {} architectural boundary violation(s):",
                    violations.len()
                );
                for v in &violations {
                    eprintln!(
                        "  {}:{}: {} ({})",
                        v.file.display(),
                        v.line_number,
                        v.rule,
                        v.line_content.trim()
                    );
                }
                bail!("Architecture check failed.");
            }
        }
        Command::Doctor(args) => {
            println!("Hephos Environment Doctor:\n");
            let items = run_doctor(&args.path)?;
            let mut has_errors = false;
            for item in &items {
                let symbol = match item.status {
                    DiagnosticStatus::Ok => "✓",
                    DiagnosticStatus::Warning => "!",
                    DiagnosticStatus::Error => {
                        has_errors = true;
                        "✗"
                    }
                };
                println!("  [{symbol}] {}: {}", item.name, item.detail);
            }
            println!();
            if has_errors {
                bail!("One or more critical environment checks failed.");
            } else {
                println!("✓ Environment ready for Hephos development.");
                Ok(())
            }
        }
        Command::Generate(args) => match args.artifact_type.as_str() {
            "resource" => {
                let files = generate_resource(&args.path, &args.name)?;
                println!("Generated resource '{}':", args.name);
                for f in files {
                    println!("  ✓ {f}");
                }
                Ok(())
            }
            "migration" => {
                let file = generate_migration(&args.path, &args.name)?;
                println!("Generated migration '{}':", args.name);
                println!("  ✓ {file}");
                Ok(())
            }
            other => {
                bail!("Unknown artifact type '{other}'. Supported: 'resource', 'migration'");
            }
        },
    }
}
