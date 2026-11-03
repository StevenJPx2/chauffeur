//! Orders a prompt's skills by the words they share with it: BM25 over each
//! skill's name and description, with a light stemmer so "deployed" meets
//! "deployment". On the `bench/ask` ranking sets it ranked the needed skill
//! as well as the embedding service, in well under a millisecond and with no
//! service to run.

use std::collections::{HashMap, HashSet};

use chauffeur_core::CatalogEntry;

/// How fast a word's weight saturates as it repeats in one skill.
const K1: f32 = 1.2;
/// How much a long description's matches are discounted.
const B: f32 = 0.75;
/// Words too common in requests and descriptions to tell skills apart.
const STOP: &[&str] = &[
    "a", "all", "an", "and", "any", "are", "as", "at", "be", "by", "can", "do", "does", "for",
    "from", "how", "i", "in", "is", "it", "its", "me", "my", "of", "on", "or", "so", "that", "the",
    "this", "to", "use", "what", "when", "which", "why", "will", "with", "you", "your",
];
/// Suffixes removed, longest first, when at least three letters remain.
const SUFFIXES: &[&str] = &[
    "ations", "ation", "ments", "ment", "ings", "ing", "ers", "er", "ies", "ied", "ed", "es", "s",
];

/// Skill IDs, the best word match for `query` first; skills sharing no word
/// with it follow in catalog order.
pub(super) fn order(query: &str, catalog: &[CatalogEntry]) -> Vec<String> {
    let documents: Vec<Vec<String>> = catalog
        .iter()
        .map(|skill| terms(&format!("{} {}", skill.id, skill.description)))
        .collect();
    let query: HashSet<String> = terms(query).into_iter().collect();
    let scores = scores(&query, &documents);
    let mut ranked: Vec<(usize, f32)> = scores.into_iter().enumerate().collect();

    // A stable sort keeps catalog order among equal scores.
    ranked.sort_by(|a, b| b.1.total_cmp(&a.1));
    ranked
        .into_iter()
        .map(|(index, _)| catalog[index].id.clone())
        .collect()
}

/// Each document's BM25 score for the query's terms.
fn scores(query: &HashSet<String>, documents: &[Vec<String>]) -> Vec<f32> {
    #[allow(
        clippy::cast_precision_loss,
        reason = "a catalog holds at most a few hundred skills"
    )]
    let count = documents.len() as f32;
    #[allow(
        clippy::cast_precision_loss,
        reason = "descriptions are a few hundred bytes"
    )]
    let average = documents.iter().map(Vec::len).sum::<usize>() as f32 / count.max(1.0);
    let mut containing: HashMap<&str, f32> = HashMap::new();

    for document in documents {
        for term in document.iter().collect::<HashSet<_>>() {
            *containing.entry(term.as_str()).or_default() += 1.0;
        }
    }

    documents
        .iter()
        .map(|document| {
            #[allow(
                clippy::cast_precision_loss,
                reason = "descriptions are a few hundred bytes"
            )]
            let length = document.len() as f32;

            query
                .iter()
                .filter_map(|term| {
                    let n = *containing.get(term.as_str())?;
                    #[allow(
                        clippy::cast_precision_loss,
                        reason = "a description repeats a word a few times"
                    )]
                    let frequency = document.iter().filter(|word| *word == term).count() as f32;
                    let idf = (1.0 + (count - n + 0.5) / (n + 0.5)).ln();

                    Some(
                        idf * frequency * (K1 + 1.0)
                            / (frequency + K1 * (1.0 - B + B * length / average)),
                    )
                })
                .sum()
        })
        .collect()
}

/// Lowercase alphanumeric words, stop words dropped, each stemmed.
fn terms(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|word| !word.is_empty() && !STOP.contains(word))
        .map(stem)
        .collect()
}

/// One root for a word's forms: "deployed", "deployment" and "deploys"
/// become "deploy"; "release" and "releases" become "releas"; "running" and
/// "runs" become "run"; "libraries" becomes "library".
fn stem(word: &str) -> String {
    let root = SUFFIXES
        .iter()
        .find_map(|suffix| {
            let root = word.strip_suffix(suffix)?;

            (root.len() >= 3).then(|| match *suffix {
                "ies" | "ied" => format!("{root}y"),
                _ => root.to_owned(),
            })
        })
        .unwrap_or_else(|| word.to_owned());

    trimmed(root)
}

/// Without a final "e" or a doubled final consonant, while three letters remain.
fn trimmed(mut root: String) -> String {
    let bytes = root.as_bytes();
    let last = bytes.len().checked_sub(1).map(|at| bytes[at]);
    let before = bytes.len().checked_sub(2).map(|at| bytes[at]);
    let doubled = last == before
        && last.is_some_and(|letter| !b"aeiouls".contains(&letter) && letter.is_ascii_alphabetic());

    if root.len() > 3 && (last == Some(b'e') || doubled) {
        root.pop();
    }

    root
}

#[cfg(test)]
mod tests {
    use super::*;

    fn skill(id: &str, description: &str) -> CatalogEntry {
        CatalogEntry {
            id: id.into(),
            description: description.into(),
            bytes: 0,
        }
    }

    fn catalog() -> Vec<CatalogEntry> {
        vec![
            skill("vitest", "Write and run unit test suites"),
            skill(
                "slack-cli",
                "Read channels and threads and send replies in Slack",
            ),
            skill(
                "wrangler",
                "Run Wrangler CLI commands, including local development and deployment",
            ),
        ]
    }

    #[test]
    fn a_word_in_another_form_still_matches() {
        assert_eq!(
            order("the worker we deployed yesterday", &catalog())[0],
            "wrangler"
        );
        assert_eq!(
            order("tell the #releases channel it shipped", &catalog())[0],
            "slack-cli"
        );
        assert_eq!(order("add tests for the parser", &catalog())[0], "vitest");
    }

    #[test]
    fn skills_sharing_no_word_keep_catalog_order_after_the_matches() {
        assert_eq!(
            order("send a reply", &catalog()),
            ["slack-cli", "vitest", "wrangler"]
        );
        assert_eq!(order("", &catalog()), ["vitest", "slack-cli", "wrangler"]);
    }

    #[test]
    fn a_words_forms_share_one_stem_and_short_words_stay_whole() {
        let same = |a: &str, b: &str| assert_eq!(stem(a), stem(b), "{a} and {b}");

        same("deployed", "deployment");
        same("release", "releases");
        same("service", "services");
        same("configure", "configuration");
        same("running", "runs");
        same("library", "libraries");
        same("cached", "caching");
        same("retry", "retries");
        assert_ne!(stem("general"), stem("generate"));
        assert_ne!(stem("universe"), stem("university"));
        assert_eq!(stem("bus"), "bus");
        assert_eq!(stem("all"), "all");
    }
}
