//! Chauffeur CLI. All behavior goes through the daemon protocol.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::process::ExitCode;
use std::{fs::File, io::Read};

use chauffeur_core::{
    AgentContext, DEFAULT_DAEMON_URL, DaemonClient, SkillContext, Target, load_skill,
    skill::MAX_CONTEXT_BYTES,
};
use chauffeur_daemon::{DEFAULT_PORT, DaemonOptions};

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
        "steer" => steer(&args[1..]).await,
        "reminders" => reminders(&args[1..]).await,
        "skill" => skill(&args[1..]).await,
        "skills" => skills().await,
        "health" => client()?.health().await,
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

async fn steer(args: &[String]) -> Result<(), String> {
    let target = target(args)?;
    let context_path = option(args, "--context").ok_or("steer needs --context PATH")?;
    let raw =
        std::fs::read_to_string(context_path).map_err(|error| format!("read context: {error}"))?;
    let context: AgentContext =
        serde_json::from_str(&raw).map_err(|error| format!("decode context: {error}"))?;
    let result = client()?.steer(target, context).await?;

    println!(
        "{}",
        serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?
    );
    Ok(())
}

async fn reminders(args: &[String]) -> Result<(), String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err("reminders needs read or ack".into());
    };
    let target = target(&args[1..])?;

    match action {
        "read" => {
            let reminders = client()?.reminders(target).await?;
            println!(
                "{}",
                serde_json::to_string_pretty(&reminders).map_err(|error| error.to_string())?
            );
            Ok(())
        }
        "ack" => {
            let ids = values(&args[1..], "--id");
            client()?.acknowledge(target, ids).await
        }
        _ => Err("reminders needs read or ack".into()),
    }
}

async fn skill(args: &[String]) -> Result<(), String> {
    let Some(action) = args.first().map(String::as_str) else {
        return Err("skill needs validate or evaluate".into());
    };

    match action {
        "validate" => validate_skill(args.get(1).ok_or("skill validate needs PATH")?),
        "evaluate" => evaluate_skill(&args[1..]).await,
        _ => Err("skill needs validate or evaluate".into()),
    }
}

fn validate_skill(path: &str) -> Result<(), String> {
    let skill = load_skill(Path::new(path))?;
    let encoded = serde_json::to_string_pretty(&skill).map_err(|error| error.to_string())?;

    println!("{encoded}");

    Ok(())
}

async fn evaluate_skill(args: &[String]) -> Result<(), String> {
    let skill_id = option(args, "--skill").ok_or("skill evaluate needs --skill ID")?;
    let context_path = option(args, "--context").ok_or("skill evaluate needs --context PATH")?;
    let bytes = read_bounded(context_path, MAX_CONTEXT_BYTES)?;
    let context: SkillContext =
        serde_json::from_slice(&bytes).map_err(|error| format!("decode skill context: {error}"))?;
    let result = client()?.evaluate_skill(skill_id, context).await?;
    let encoded = serde_json::to_string_pretty(&result).map_err(|error| error.to_string())?;

    println!("{encoded}");

    Ok(())
}

async fn skills() -> Result<(), String> {
    let ids = client()?.skills().await?;
    let encoded = serde_json::to_string_pretty(&ids).map_err(|error| error.to_string())?;

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

fn target(args: &[String]) -> Result<Target, String> {
    let kind = option(args, "--target-kind").ok_or("missing --target-kind")?;
    let id = option(args, "--target-id").ok_or("missing --target-id")?;

    Ok(Target::new(kind, id))
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

fn values(args: &[String], name: &str) -> Vec<String> {
    args.windows(2)
        .filter(|pair| pair.first().is_some_and(|argument| argument == name))
        .filter_map(|pair| pair.get(1).cloned())
        .collect()
}

fn usage() -> String {
    "usage: chauffeur daemon [--port PORT] | mcp | health | skills | skill validate PATH | skill evaluate --skill ID --context PATH | steer --target-kind KIND --target-id ID --context PATH | reminders read|ack --target-kind KIND --target-id ID [--id ID]".into()
}
