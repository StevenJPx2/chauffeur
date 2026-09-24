//! Chauffeur CLI. All behavior goes through the daemon protocol.

mod audit;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::{fs::File, io::Read};

use chauffeur_capability_permission::load_skill;
use chauffeur_core::{DEFAULT_DAEMON_URL, DaemonClient, Signal};
use chauffeur_daemon::{DEFAULT_PORT, DaemonOptions};

/// Matches the daemon's RPC body cap.
const MAX_SIGNAL_BYTES: usize = 128 * 1024;

#[tokio::main]
async fn main() -> ExitCode {
    match run(std::env::args().skip(1).collect()).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("chauffeur: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(usage());
    };

    match command {
        "daemon" => run_daemon(&args[1..]).await,
        "mcp" => chauffeur_mcp::run_stdio(client()?).await,
        "skill" => skill(&args[1..]),
        "signal" => signal(&args[1..]).await,
        "health" => client()?.health().await,
        "audit" => audit(&args[1..]),
        _ => Err(usage()),
    }
}

async fn run_daemon(args: &[String]) -> Result<(), String> {
    let port = option(args, "--port").map_or(Ok(DEFAULT_PORT), |value| {
        value.parse::<u16>().map_err(|error| error.to_string())
    })?;
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let options = DaemonOptions::from_env(address)?;

    chauffeur_daemon::serve(options).await
}

fn audit(args: &[String]) -> Result<(), String> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT);
    let path = DaemonOptions::from_env(address)?
        .audit_file
        .ok_or("the daemon keeps no audit log")?;

    audit::run(args, &path)
}

fn skill(args: &[String]) -> Result<(), String> {
    match (args.first().map(String::as_str), args.get(1)) {
        (Some("validate"), Some(path)) => validate_skill(path),
        (Some("validate-project"), Some(path)) => {
            let skills = chauffeur_capability_project_skills::load(path)?;

            for skill in skills {
                println!("{}: {} judgment step(s)", skill.id, skill.steps.len());
            }

            Ok(())
        }
        _ => Err("usage: chauffeur skill validate PATH | validate-project WORKSPACE".into()),
    }
}

/// Validate a permission contract, or a misuse contract (one with
/// `match.tools`), and print what was accepted.
fn validate_skill(path: &str) -> Result<(), String> {
    let bytes = read_bounded(path, MAX_SIGNAL_BYTES)?;
    let value: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|error| format!("{path}: {error}"))?;

    if value["match"]["tools"].is_array() {
        let contract = chauffeur_capability_tool_misuse::MisuseContract::from_json(&bytes)
            .map_err(|error| format!("{path}: {error}"))?;

        println!(
            "valid misuse contract {} watching {:?}",
            contract.identity.id, contract.matches.tools
        );

        return Ok(());
    }

    let skill = load_skill(Path::new(path))?;
    let encoded = serde_json::to_string_pretty(&skill).map_err(|error| error.to_string())?;

    println!("{encoded}");

    Ok(())
}

/// Send one signal from a JSON file and print the effects it produced.
async fn signal(args: &[String]) -> Result<(), String> {
    let path = option(args, "--file").ok_or("signal needs --file PATH")?;
    let bytes = read_bounded(path, MAX_SIGNAL_BYTES)?;
    let signal: Signal =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode signal: {error}"))?;
    let effects = client()?.signal(signal).await?;
    let encoded = serde_json::to_string_pretty(&effects).map_err(|error| error.to_string())?;

    println!("{encoded}");

    Ok(())
}

fn read_bounded(path: &str, limit: usize) -> Result<Vec<u8>, String> {
    let file = File::open(path).map_err(|error| format!("read {path}: {error}"))?;
    let cap = u64::try_from(limit.saturating_add(1)).map_err(|error| error.to_string())?;
    let mut bytes = Vec::with_capacity(limit.min(8_192));

    file.take(cap)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("read {path}: {error}"))?;

    if bytes.len() > limit {
        return Err(format!("{path} exceeds {limit} bytes"));
    }

    Ok(bytes)
}

fn client() -> Result<DaemonClient, String> {
    let url = std::env::var("CHAUFFEUR_DAEMON_URL").unwrap_or_else(|_| DEFAULT_DAEMON_URL.into());
    let token = std::env::var("CHAUFFEUR_DAEMON_TOKEN").ok();

    DaemonClient::new(url, token)
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.windows(2)
        .find(|pair| pair.first().is_some_and(|argument| argument == name))
        .and_then(|pair| pair.get(1))
        .map(String::as_str)
}

fn usage() -> String {
    "usage: chauffeur daemon [--port PORT] | mcp | health | audit [N] | skill validate PATH | signal --file PATH".into()
}
