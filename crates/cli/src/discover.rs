//! `chauffeur skill discover` and `chauffeur skill approve`: find skills in
//! the Skilld registry and add the ones the user approves as sources. The
//! shared crate decides; this only parses arguments and prints.

use std::fmt::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use chauffeur_capability_skill_exposure::sources::{
    Approval, Candidate, Change, Consent, Review, Selector, SkilldClient, Tier, approve, gather,
    read_config, read_verdict, review_signal, write_config,
};
use chauffeur_core::SkillSafety;
use chauffeur_daemon::{DEFAULT_PORT, DaemonOptions};

use crate::option;

const DEFAULT_LIMIT: u8 = 10;
pub const APPROVE_USAGE: &str =
    "OWNER/REPOSITORY[/SKILL] [--allow ID]... [--accept-risk] [--unvetted] [--dry-run] [--json]";

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
    let flag = |name: &str| args.iter().any(|arg| arg == name);
    let json = flag("--json");
    let dry_run = flag("--dry-run");
    let consent = Consent {
        allow: args
            .windows(2)
            .filter(|pair| pair[0] == "--allow")
            .flat_map(|pair| pair[1].split(','))
            .map(|id| id.trim().to_owned())
            .filter(|id| !id.is_empty())
            .collect(),
        accept_risk: flag("--accept-risk"),
        unvetted: flag("--unvetted"),
    };
    let [selector] = words(args, &["--allow"])[..] else {
        return Err(format!("usage: chauffeur skill approve {APPROVE_USAGE}"));
    };
    let selector: Selector = selector.parse().map_err(|error| format!("{error}"))?;
    let path = config_path()?;
    let config = read_config(&path).map_err(|error| error.to_string())?;
    let skilld = SkilldClient::from_env().map_err(|error| error.to_string())?;
    let candidates = skilld
        .candidates(&selector)
        .await
        .map_err(|error| error.to_string())?;
    let plan = approve(&selector, candidates.clone(), &config, &consent, &[])
        .map_err(|error| error.to_string())?;
    // A skill already followed needs no review.
    let approval = if matches!(plan.change, Change::Covered { .. }) {
        plan
    } else {
        if !json {
            eprintln!(
                "Reviewing {} skill(s) with Chauffeur's judges…",
                candidates.len()
            );
        }

        let reviews = review_all(&candidates).await;

        approve(&selector, candidates, &config, &consent, &reviews)
            .map_err(|error| error.to_string())?
    };
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

    let mut stops = approval.refused.clone();

    if !approval.blocked.is_empty() {
        stops.push(format!(
            "{} asks to {}; --allow {} accepts that",
            approval.selector,
            approval.blocked.join(", "),
            approval.blocked.join(",")
        ));
    }

    if stops.is_empty() {
        Ok(())
    } else {
        Err(format!("nothing was written: {}", stops.join("; ")))
    }
}

/// Reviews at once; the daemon judges up to eight signals together.
const CONCURRENT_REVIEWS: usize = 4;
/// A review's budget: the daemon's whole run limit, plus its reply reserve.
const REVIEW_DEADLINE_MS: u64 = 14_150;

/// Each candidate judged by the daemon, in order. Any failure along the way
/// is a review without a verdict, never a pass.
async fn review_all(candidates: &[Candidate]) -> Vec<Review> {
    let daemon = match crate::client() {
        Ok(daemon) => daemon,
        Err(error) => {
            return vec![Review::undetermined(format!("no daemon: {error}")); candidates.len()];
        }
    };
    let permits = std::sync::Arc::new(tokio::sync::Semaphore::new(CONCURRENT_REVIEWS));
    let at = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    let mut tasks = tokio::task::JoinSet::new();

    for (index, candidate) in candidates.iter().cloned().enumerate() {
        let (daemon, permits) = (daemon.clone(), permits.clone());

        tasks.spawn(async move {
            let _permit = permits.acquire().await;
            let agent = format!("skill-review:{}:{index}", std::process::id());
            let scratch =
                std::env::temp_dir().join(format!("chauffeur-{agent}.git").replace(':', "-"));
            let gathered = gather(&candidate, &scratch).await;
            let _ = std::fs::remove_dir_all(&scratch);
            let review = match gathered {
                Ok(gathered) => {
                    match daemon
                        .signal_with_deadline(
                            review_signal(&candidate, gathered, agent, at),
                            Some(REVIEW_DEADLINE_MS),
                        )
                        .await
                    {
                        Ok(effects) => read_verdict(&effects),
                        Err(error) => Review::undetermined(format!("the daemon failed: {error}")),
                    }
                }
                Err(error) => Review::undetermined(error.to_string()),
            };

            (index, review)
        });
    }

    let mut reviews = vec![Review::undetermined("not reviewed"); candidates.len()];

    while let Some(done) = tasks.join_next().await {
        if let Ok((index, review)) = done {
            reviews[index] = review;
        }
    }

    reviews
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

fn describe_review(text: &mut String, review: &Review) {
    let verdict = match review.safety {
        SkillSafety::Safe => "safe",
        SkillSafety::Unsafe => "UNSAFE",
        SkillSafety::Undetermined => "no verdict",
    };
    let judge = match review.judge.as_str() {
        "system_one" => " (System One)",
        "system_two" => " (System Two)",
        _ => "",
    };

    let _ = writeln!(text, "  review: {verdict}{judge}: {}", review.reason);

    for concern in &review.concerns {
        let _ = writeln!(text, "    - {concern}");
    }
}

fn report(approval: &Approval, path: &std::path::Path, written: bool, dry_run: bool) -> String {
    let mut text = String::new();
    let reviewed = !matches!(approval.change, Change::Covered { .. });

    for (skill, review) in approval.skills.iter().zip(&approval.reviews) {
        describe(&mut text, skill);

        if reviewed {
            describe_review(&mut text, review);
        }
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
