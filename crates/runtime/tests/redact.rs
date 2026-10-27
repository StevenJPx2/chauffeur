//! Redaction cases carried over from v1.
#![expect(
    clippy::unwrap_used,
    reason = "the helpers fail the test on a broken fixture"
)]

use std::path::PathBuf;

use chauffeur_runtime::{Masking, Prefix, RedactError, RedactionConfig, Redactor};

fn shipped() -> Redactor {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../distribution/safety/redaction.json");
    Redactor::load(&[path]).unwrap()
}

fn redact(text: &str) -> String {
    shipped().redact(text, &mut Masking::default())
}

#[test]
fn known_tokens_are_redacted_and_the_text_around_them_kept() {
    assert_eq!(
        redact("key AKIAABCDEFGHIJKLMNOP and ghp_abcdefghijklmnopqrstuvwxyz0123 done"),
        "key [REDACTED] and [REDACTED] done"
    );
}

#[test]
fn paths_links_and_hashes_stay_readable_while_tokens_in_them_are_caught() {
    let readable = [
        "cd /Users/stevenjohn/Documents/Adeptmind/Projects/hpdp-overlay/ADEPT-45130 && ls ~/Projects/ADEPT-45130/src",
        "git show 3f786850e387550fdab836ed7e6dc881de23001b",
        "https://adeptmind.slack.com/archives/C08ABCDEF12/p1790340000123456 Can you fix this?",
        "https://github.com/StevenJPx2/chauffeur/pull/42",
        "watching StevenJPx2/chauffeur-demo-ci#3",
    ];
    for text in readable {
        assert_eq!(redact(text), text);
    }

    let sha = "3f786850e387550fdab836ed7e6dc881de23001b";
    assert!(!redact(&format!("Authorization: Bearer {sha}")).contains(sha));
    assert!(!redact(&format!("X-Api-Key: {sha}")).contains(sha));
    assert_eq!(
        redact("in acme/ghp_abcdefghijklmnopqrstuvwxyz0123"),
        "in acme/[REDACTED]"
    );
    assert_eq!(
        redact("https://x.com/api/ghp_abcdefghijklmnopqrstuvwxyz0123/repos"),
        "https://x.com/api/[REDACTED]/repos"
    );
    assert!(
        redact("https://x.com/cb?sig=Zx9kQ2mP7vL4/nR8sT1wY6uB3cD5fG0hJ+aa")
            .contains("sig=[string ")
    );
}

#[test]
fn private_key_blocks_are_redacted() {
    let text = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n-----END OPENSSH PRIVATE KEY-----\nafter";

    assert_eq!(redact(text), "before\n[REDACTED]\nafter");
    assert_eq!(redact("-----BEGIN RSA PRIVATE KEY-----\nabc"), "[REDACTED]");
}

#[test]
fn short_or_embedded_lookalikes_are_left() {
    for text in [
        "sk-short and task-ant-x",
        "mask-abcdefghijklmnopqrstuvwxyz",
        "naïve ✓",
    ] {
        assert_eq!(redact(text), text);
    }
}

#[test]
fn unknown_credentials_are_described_never_sent() {
    let secret = "Zx9kQ2mP7vL4nR8sT1wY6uB3cD5fG0hJ";
    let sha = "3f786850e387550fdab836ed7e6dc881de23001b";
    let mut masking = Masking::default();
    let text = format!("curl -H api_key={secret} && git show commit={sha} && echo {secret}");

    let sent = shipped().redact(&text, &mut masking);

    assert!(!sent.contains(secret) && !sent.contains(sha), "{sent}");
    assert_eq!(
        sent,
        "curl -H api_key=[string 1: 32 characters of mixed letters and digits, after \"api_key=\"] \
         && git show commit=[string 2: 40 characters of hex, after \"commit=\"] \
         && echo [string 1: 32 characters of mixed letters and digits]",
        "the same value keeps its number"
    );
}

#[test]
fn a_users_prefixes_add_to_or_replace_the_shipped_ones() {
    let shipped = RedactionConfig {
        replace: false,
        prefixes: vec![Prefix {
            prefix: "ghp_".into(),
            min_body: 20,
        }],
    };
    let acme = |replace| RedactionConfig {
        replace,
        prefixes: vec![Prefix {
            prefix: "acme_".into(),
            min_body: 8,
        }],
    };
    let mut masking = Masking::default();

    let added = Redactor::new([shipped.clone(), acme(false)]).unwrap();
    let replaced = Redactor::new([shipped, acme(true)]).unwrap();

    assert_eq!(
        added.redact(
            "acme_12345678 ghp_abcdefghijklmnopqrstuvwxyz0123",
            &mut masking
        ),
        "[REDACTED] [REDACTED]"
    );
    assert_eq!(
        replaced.redact("acme_12345678 ghp_abcdefghijklmnopqrstu", &mut masking),
        "[REDACTED] ghp_abcdefghijklmnopqrstu"
    );
    let empty = RedactionConfig {
        replace: false,
        prefixes: vec![Prefix {
            prefix: String::new(),
            min_body: 1,
        }],
    };
    assert!(matches!(Redactor::new([empty]), Err(RedactError::Prefixes)));
}
