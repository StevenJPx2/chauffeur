//! `chauffeur skill discover` and `chauffeur skill approve`: find skills in
//! the Skilld registry and add the ones the user approves as sources. The
//! shared crate decides; this only parses arguments and prints.

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use chauffeur_capability_skill_exposure::sources::{
    Approval, Candidate, Change, Selector, SkilldClient, Tier, approve, read_config, write_config,
};
use chauffeur_daemon::{DEFAULT_PORT, DaemonOptions};

use crate::option;

const DEFAULT_LIMIT: u8 = 10;

pub async fn discover(args: &[String]) -> Result<(), String> {
    let json = args.iter().any(|arg| arg == "--json");
    let limit = option(args, "--limit").map_or(Ok(DEFAULT_LIMIT), |value| {
        value
            .parse::<u8>()
            .map_err(|_| format!("--limit takes 1 to 50, not {value}"))
    })?;
    let query = words(args, &["--limit"]).join(" ");

    if query.is_empty() {
        return Err("usage: chauffeur skill discover WORDS... [--limit N] [--json]".into());
    }

    let found = SkilldClient::from_env()
        .map_err(|error| error.to_string())?
        .search(&query, limit)
        .await
        .map_err(|error| error.to_string())?;

    if json {
        return print_json(&found);
    }

    if found.is_empty() {
        println!("Skilld found no skills for \"{query}\".");

        return Ok(());
    }

    for skill in &found {
        println!("{}  ★{}", skill.selector, skill.stars);

        if let Some(description) = &skill.description {
            println!("  {description}");
        }
    }

    println!(
        "\nNothing is added until you approve it: chauffeur skill approve OWNER/REPOSITORY/SKILL"
    );

    Ok(())
}

pub async fn approve_skill(args: &[String]) -> Result<(), String> {
    let json = args.iter().any(|arg| arg == "--json");
    let dry_run = args.iter().any(|arg| arg == "--dry-run");
    let allow: Vec<String> = args
        .windows(2)
        .filter(|pair| pair[0] == "--allow")
        .flat_map(|pair| pair[1].split(','))
        .map(|id| id.trim().to_owned())
        .filter(|id| !id.is_empty())
        .collect();
    let [selector] = words(args, &["--allow"])[..] else {
        return Err(
            "usage: chauffeur skill approve OWNER/REPOSITORY[/SKILL] [--allow ID]... [--dry-run] [--json]"
                .into(),
        );
    };
    let selector: Selector = selector.parse().map_err(|error| format!("{error}"))?;
    let path = config_path()?;
    let config = read_config(&path).map_err(|error| error.to_string())?;
    let candidates = SkilldClient::from_env()
        .map_err(|error| error.to_string())?
        .candidates(&selector)
        .await
        .map_err(|error| error.to_string())?;
    let approval =
        approve(&selector, candidates, &config, &allow).map_err(|error| error.to_string())?;
    let write = approval.writes() && !dry_run;

    if write {
        write_config(&path, &approval.config).map_err(|error| error.to_string())?;
    }

    if json {
        print_json(&serde_json::json!({
            "approval": approval,
            "written": write,
            "config": path,
        }))?;
    } else {
        print!("{}", report(&approval, &path, write, dry_run));
    }

    if approval.blocked.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "nothing was written: {} asks to {}. If you accept that, run again with --allow {}",
            approval.selector,
            approval.blocked.join(", "),
            approval.blocked.join(",")
        ))
    }
}

/// Positional arguments: everything but flags and the values of `valued` options.
fn words<'a>(args: &'a [String], valued: &[&str]) -> Vec<&'a str> {
    let mut words = Vec::new();
    let mut skip = false;

    for arg in args {
        if skip {
            skip = false;
        } else if valued.contains(&arg.as_str()) {
            skip = true;
        } else if !arg.starts_with("--") {
            words.push(arg.as_str());
        }
    }

    words
}

fn config_path() -> Result<PathBuf, String> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT);

    Ok(DaemonOptions::from_env(address)?
        .config_dir
        .join("skill-sources.json"))
}

fn print_json(value: &impl serde::Serialize) -> Result<(), String> {
    println!(
        "{}",
        serde_json::to_string_pretty(value).map_err(|error| error.to_string())?
    );

    Ok(())
}

fn describe(text: &mut String, skill: &Candidate) {
    let _ = writeln!(text, "{}  ({})", skill.selector, skill.dir);

    if let Some(description) = &skill.description {
        let _ = writeln!(text, "  {description}");
    }

    let _ = writeln!(
        text,
        "  by {} · license {} · commit {} · {}",
        skill.author.as_deref().unwrap_or("unknown author"),
        skill.license.as_deref().unwrap_or("unknown"),
        skill
            .commit
            .as_deref()
            .map_or("unknown", |commit| &commit[..12]),
        skill.page
    );

    for behavior in &skill.behaviors {
        let tier = match behavior.tier {
            Tier::Ask => "needs approval",
            Tier::Show => "noted",
        };

        let _ = writeln!(
            text,
            "  {} ({tier}): {} ×{} {}",
            behavior.id,
            behavior.label,
            behavior.total,
            behavior.locations.join(", ")
        );
    }
}

fn report(approval: &Approval, path: &std::path::Path, written: bool, dry_run: bool) -> String {
    let mut text = String::new();

    for skill in &approval.skills {
        describe(&mut text, skill);
    }

    let scope = if approval.selector.matches('/').count() == 1 {
        format!("{} and any skill published under it later", approval.dir)
    } else {
        approval.dir.clone()
    };
    let change = match &approval.change {
        Change::Added { source } => {
            format!(
                "add source {source} for {}, following {scope}",
                approval.url
            )
        }
        Change::Extended { source } => format!("source {source} also follows {scope}"),
        Change::Covered { source } => format!("source {source} already follows it; nothing to do"),
    };
    let outcome = if written {
        format!(
            "Wrote {}. The daemon loads it within a second and fetches the skill from GitHub.",
            path.display()
        )
    } else if dry_run && approval.writes() {
        "Dry run: nothing was written.".into()
    } else {
        String::new()
    };

    let _ = writeln!(text, "\nChange: {change}");

    if !outcome.is_empty() {
        let _ = writeln!(text, "{outcome}");
    }

    text
}
