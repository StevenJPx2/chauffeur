//! `chauffeur`: runs the daemon, and talks to it over its socket.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::ExitCode;

use chauffeur_daemon::{DaemonError, Paths};
use clap::{Parser, Subcommand};
use color_eyre::eyre::{Result, WrapErr, eyre};
use serde_json::{Value, json};

#[derive(Parser)]
#[command(version, about = "A judgment engine for coding agents")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the daemon in the foreground. Exits at once if one already runs.
    Daemon {
        /// Shipped models, rules and safety patterns.
        #[arg(long)]
        distribution: Option<PathBuf>,
    },
    /// Print the version.
    Version,
    /// Send one event, read as JSON on stdin, and print what the rules returned.
    Emit {
        /// How long the rules may take, in milliseconds.
        #[arg(long, default_value_t = 3_000)]
        deadline_ms: u64,
    },
    /// Print the daemon's recent runs.
    Events,
    /// Print the daemon's version, loaded rules and refused rule files.
    Status,
}

fn main() -> Result<ExitCode> {
    color_eyre::install()?;
    let name = std::env::current_exe()
        .ok()
        .and_then(|exe| {
            exe.file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "chauffeur".to_owned());
    match Cli::parse().command {
        Command::Daemon { distribution } => {
            match chauffeur_daemon::run(&Paths::from_env(&name, distribution)) {
                Ok(()) => Ok(ExitCode::SUCCESS),
                Err(DaemonError::Running(socket)) => {
                    eprintln!(
                        "{name}: a daemon is already running on {}",
                        socket.display()
                    );
                    Ok(ExitCode::SUCCESS)
                }
                Err(error) => Err(error.into()),
            }
        }
        Command::Version => {
            println!("{name} {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Emit { deadline_ms } => {
            let mut text = String::new();
            std::io::stdin().read_to_string(&mut text)?;
            let event: Value =
                serde_json::from_str(&text).wrap_err("the event on stdin is not JSON")?;
            print(&call(
                &name,
                "chauffeur.emit",
                &json!({ "event": event, "deadline_ms": deadline_ms }),
            )?)
        }
        Command::Events => print(&call(&name, "chauffeur.events", &Value::Null)?),
        Command::Status => print(&call(&name, "chauffeur.status", &Value::Null)?),
    }
}

fn print(result: &Value) -> Result<ExitCode> {
    println!("{}", serde_json::to_string_pretty(result)?);
    Ok(ExitCode::SUCCESS)
}

/// One JSON-RPC call to the daemon, returning its result.
fn call(name: &str, method: &str, params: &Value) -> Result<Value> {
    let socket = Paths::from_env(name, None).socket;
    let mut stream = UnixStream::connect(&socket)
        .wrap_err_with(|| format!("no daemon answers on {}", socket.display()))?;
    writeln!(
        stream,
        "{}",
        json!({ "jsonrpc": "2.0", "id": 1, "method": method, "params": params })
    )?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    let reply: Value = serde_json::from_str(&line).wrap_err("the daemon's reply is not JSON")?;
    if let Some(error) = reply.get("error") {
        let message = error.get("message").and_then(Value::as_str);
        return Err(eyre!(
            "{}",
            message.unwrap_or("the daemon refused the call")
        ));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}
