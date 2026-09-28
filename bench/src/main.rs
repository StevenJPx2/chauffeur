//! Entry point: parse the command line and dispatch.

use std::path::Path;
use std::process::ExitCode;

use chauffeur_bench::cli::{self, Command};
use chauffeur_bench::inputs::{self, Setup, Task};
use chauffeur_bench::{report, runner, verify};

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match cli::parse(&args).and_then(|cli| dispatch(&cli.root, cli.command)) {
        Ok(code) => code,
        Err(error) => {
            eprintln!("chauffeur-bench: {error}");

            ExitCode::FAILURE
        }
    }
}

fn dispatch(root: &Path, command: Command) -> Result<ExitCode, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("resolve {}: {error}", root.display()))?;

    match command {
        Command::Help => {
            println!("{}", cli::USAGE);

            Ok(ExitCode::SUCCESS)
        }
        Command::List => list(&root).map(|()| ExitCode::SUCCESS),
        Command::Verify { tasks } => run_verify(&root, tasks.as_deref()),
        Command::Run(options) => runner::run(&root, &options).map(|()| ExitCode::SUCCESS),
        Command::Report { dir } => {
            let path = report::write(&dir)?;
            println!("{}", path.display());

            Ok(ExitCode::SUCCESS)
        }
    }
}

fn list(root: &Path) -> Result<(), String> {
    let tasks = inputs::load_tasks(root)?;
    let variants = inputs::load_variants(root)?;

    println!("tasks:");
    for task in &tasks {
        println!(
            "  {:<28} {}s  {}",
            task.id,
            task.spec.timeout_seconds,
            task.spec.tags.join(", ")
        );
    }
    println!("variants:");
    for variant in &variants {
        let setup = match &variant.setup {
            Setup::Base => "plain OpenCode".to_string(),
            Setup::Chauffeur { disable } if disable.is_empty() => "Chauffeur".to_string(),
            Setup::Chauffeur { disable } => format!("Chauffeur without {}", disable.join(", ")),
        };
        println!("  {:<28} {setup}", variant.id);
    }

    Ok(())
}

fn run_verify(root: &Path, wanted: Option<&[String]>) -> Result<ExitCode, String> {
    let tasks = inputs::select(
        &inputs::load_tasks(root)?,
        wanted,
        |task: &Task| &task.id,
        "task",
    )?;
    let scratch =
        std::env::temp_dir().join(format!("chauffeur-bench-verify-{}", std::process::id()));
    let verdicts = verify::verify(&tasks, &scratch);

    print!("{}", verify::render(&verdicts));
    println!("\nworking copies and logs: {}", scratch.display());

    if verdicts.iter().all(verify::Verdict::ok) {
        Ok(ExitCode::SUCCESS)
    } else {
        Ok(ExitCode::FAILURE)
    }
}
