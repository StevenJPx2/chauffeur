use std::path::PathBuf;

use chauffeur_bench::cli::{self, Command, DEFAULT_MODEL};

fn parse(args: &[&str]) -> Result<cli::Cli, String> {
    let args: Vec<String> = args.iter().map(ToString::to_string).collect();

    cli::parse(&args)
}

#[test]
fn run_defaults() {
    let Command::Run(options) = parse(&["run"]).unwrap().command else {
        panic!("not a run");
    };

    assert_eq!(options.repeats, 3);
    assert_eq!(options.parallel, 2);
    assert_eq!(options.model, DEFAULT_MODEL);
    assert_eq!(options.tasks, None);
    assert_eq!(options.out, None);
    assert_eq!(options.chauffeur, "chauffeur");
    assert_eq!(options.opencode, "opencode");
}

#[test]
fn run_flags() {
    let cli = parse(&[
        "--root",
        "/tmp/b",
        "run",
        "--tasks",
        "a,b",
        "--variants=base,full",
        "--repeats",
        "1",
        "--parallel",
        "4",
        "--out",
        "/tmp/out",
    ])
    .unwrap();
    let Command::Run(options) = cli.command else {
        panic!("not a run");
    };

    assert_eq!(cli.root, PathBuf::from("/tmp/b"));
    assert_eq!(options.tasks, Some(vec!["a".into(), "b".into()]));
    assert_eq!(options.variants, Some(vec!["base".into(), "full".into()]));
    assert_eq!((options.repeats, options.parallel), (1, 4));
    assert_eq!(options.out, Some(PathBuf::from("/tmp/out")));
}

#[test]
fn other_commands() {
    assert_eq!(parse(&["list"]).unwrap().command, Command::List);
    assert_eq!(
        parse(&["verify", "--tasks", "a"]).unwrap().command,
        Command::Verify {
            tasks: Some(vec!["a".into()])
        }
    );
    assert_eq!(
        parse(&["report", "out"]).unwrap().command,
        Command::Report {
            dir: PathBuf::from("out")
        }
    );
    assert_eq!(parse(&[]).unwrap().command, Command::Help);
    assert_eq!(parse(&["--help"]).unwrap().command, Command::Help);
}

#[test]
fn rejects_bad_arguments() {
    let cases: [(&[&str], &str); 6] = [
        (&["list", "--tasks", "a"], "does not take --tasks"),
        (&["run", "--repeats", "0"], "at least 1"),
        (&["run", "--parallel", "99"], "between 1 and 16"),
        (&["run", "--repeats", "x"], "whole number"),
        (&["run", "--model"], "needs a value"),
        (&["report"], "unexpected arguments"),
    ];

    for (args, expected) in cases {
        let error = parse(args).unwrap_err();
        assert!(error.contains(expected), "{args:?}: {error}");
    }
}

#[test]
fn output_inside_a_git_checkout_is_refused() {
    let inside = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("results/x");
    let error = chauffeur_bench::runner::outside_repos(&inside).unwrap_err();

    assert!(error.contains("inside the git checkout"), "{error}");
    assert!(chauffeur_bench::runner::outside_repos(&std::env::temp_dir()).is_ok());
}
