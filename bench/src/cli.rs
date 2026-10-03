//! Command-line parsing.

use std::collections::BTreeMap;
use std::path::PathBuf;

use crate::judges::JudgeOptions;
use crate::runner::RunOptions;

/// The default model: the "base" setting, without a thinking variant.
pub const DEFAULT_MODEL: &str = "openai/gpt-6-luna";
const MAX_PARALLEL: usize = 16;

/// Usage text.
pub const USAGE: &str = "usage: chauffeur-bench [--root DIR] <command>

commands:
  list                                   tasks (id, tags) and variants
  verify [--tasks a,b]                   check fails unsolved, passes solved
  run [--tasks a,b] [--variants a,b] [--repeats 3] [--parallel 2]
      [--model openai/gpt-6-luna] [--out DIR] [--chauffeur BIN] [--opencode BIN]
  report DIR                             rebuild DIR/report.md from DIR/results.jsonl
  judges CORPUS [--judges FILE] [--only a,b] [--out DIR] [--limit N]
                                         replay judgments recorded with
                                         CHAUFFEUR_RECORD_JUDGMENTS against each judge
  judges-report DIR [--labels FILE]      rebuild a judges report, scoring disputes

--root defaults to the bench/ folder of this checkout; --judges to ROOT/judges.json.";

/// A parsed invocation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cli {
    /// The folder holding `variants.json` and `tasks/`.
    pub root: PathBuf,
    pub command: Command,
}

/// What to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    List,
    Verify {
        tasks: Option<Vec<String>>,
    },
    Run(RunOptions),
    Report {
        dir: PathBuf,
    },
    Judges(JudgeOptions),
    JudgesReport {
        dir: PathBuf,
        labels: Option<PathBuf>,
    },
    Help,
}

/// Parse arguments (without the program name).
///
/// # Errors
/// An unknown command or flag, a missing value, or an invalid number.
pub fn parse(args: &[String]) -> Result<Cli, String> {
    let (mut flags, positional) = split(args)?;
    let root = flags
        .remove("root")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")), PathBuf::from);
    let (name, rest) = positional
        .split_first()
        .map_or(("help", &[][..]), |(name, rest)| (name.as_str(), rest));

    let (command, allowed): (Command, &[&str]) = match (name, rest) {
        ("help" | "-h" | "--help", _) => (Command::Help, &[]),
        ("list", []) => (Command::List, &[]),
        ("verify", []) => (
            Command::Verify {
                tasks: flags.get("tasks").map(|value| csv(value)),
            },
            &["tasks"],
        ),
        ("run", []) => (Command::Run(run_options(&flags)?), RUN_FLAGS),
        ("report", [dir]) => (
            Command::Report {
                dir: PathBuf::from(dir),
            },
            &[],
        ),
        ("judges", [corpus]) => (
            Command::Judges(judge_options(&root, corpus, &flags)?),
            &["judges", "only", "out", "limit"],
        ),
        ("judges-report", [dir]) => (
            Command::JudgesReport {
                dir: PathBuf::from(dir),
                labels: flags.get("labels").map(PathBuf::from),
            },
            &["labels"],
        ),
        (name, _) => return Err(format!("unexpected arguments for {name}\n{USAGE}")),
    };

    if let Some(unknown) = flags.keys().find(|flag| !allowed.contains(&flag.as_str())) {
        return Err(format!("{name} does not take --{unknown}\n{USAGE}"));
    }

    Ok(Cli { root, command })
}

const RUN_FLAGS: &[&str] = &[
    "tasks",
    "variants",
    "repeats",
    "parallel",
    "model",
    "out",
    "chauffeur",
    "opencode",
];

fn run_options(flags: &BTreeMap<String, String>) -> Result<RunOptions, String> {
    let repeats = number(flags, "repeats", 3)?;
    let parallel = number(flags, "parallel", 2)?;

    if repeats == 0 {
        return Err("--repeats must be at least 1".into());
    }
    if parallel == 0 || parallel > MAX_PARALLEL {
        return Err(format!("--parallel must be between 1 and {MAX_PARALLEL}"));
    }

    Ok(RunOptions {
        tasks: flags.get("tasks").map(|value| csv(value)),
        variants: flags.get("variants").map(|value| csv(value)),
        repeats: u32::try_from(repeats).map_err(|_| "--repeats is too large".to_string())?,
        parallel,
        model: flags
            .get("model")
            .cloned()
            .unwrap_or_else(|| DEFAULT_MODEL.into()),
        out: flags.get("out").map(PathBuf::from),
        chauffeur: flags
            .get("chauffeur")
            .cloned()
            .unwrap_or_else(|| "chauffeur".into()),
        opencode: flags
            .get("opencode")
            .cloned()
            .unwrap_or_else(|| "opencode".into()),
    })
}

fn judge_options(
    root: &std::path::Path,
    corpus: &str,
    flags: &BTreeMap<String, String>,
) -> Result<JudgeOptions, String> {
    let limit = flags
        .get("limit")
        .map(|_| number(flags, "limit", 0))
        .transpose()?;

    Ok(JudgeOptions {
        corpus: PathBuf::from(corpus),
        judges: flags
            .get("judges")
            .map_or_else(|| root.join("judges.json"), PathBuf::from),
        only: flags.get("only").map(|value| csv(value)),
        out: flags
            .get("out")
            .map_or_else(|| PathBuf::from("judges-results"), PathBuf::from),
        limit,
    })
}

/// Split `--flag value` / `--flag=value` pairs from positional arguments.
fn split(args: &[String]) -> Result<(BTreeMap<String, String>, Vec<String>), String> {
    let mut flags = BTreeMap::new();
    let mut positional = Vec::new();
    let mut iter = args.iter();

    while let Some(arg) = iter.next() {
        let Some(flag) = arg.strip_prefix("--").filter(|flag| *flag != "help") else {
            positional.push(arg.clone());
            continue;
        };

        let (name, value) = match flag.split_once('=') {
            Some((name, value)) => (name.to_string(), value.to_string()),
            None => (
                flag.to_string(),
                iter.next()
                    .ok_or_else(|| format!("--{flag} needs a value"))?
                    .clone(),
            ),
        };
        if flags.insert(name.clone(), value).is_some() {
            return Err(format!("--{name} is given twice"));
        }
    }

    Ok((flags, positional))
}

fn number(flags: &BTreeMap<String, String>, name: &str, default: usize) -> Result<usize, String> {
    flags.get(name).map_or(Ok(default), |value| {
        value
            .parse()
            .map_err(|_| format!("--{name} must be a whole number, not {value}"))
    })
}

fn csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(str::to_string)
        .collect()
}
