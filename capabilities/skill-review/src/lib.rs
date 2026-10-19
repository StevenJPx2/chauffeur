//! Skill review: before a third-party skill is added, judge whether agents
//! can safely follow it. System One answers one question over the whole
//! skill; an unsure or failed answer, or a skill too long for System One to
//! see whole, goes to a System Two review that must cite its concerns. No
//! verdict is ever read as safe. Run it as `Judging::new(SkillReview::default())`.

use std::path::Path;

use chauffeur_core::judge::strategy::{self, Escalation, Screened};
use chauffeur_core::system_two::MAX_REASONING_INPUT_BYTES;
use chauffeur_core::{
    Effect, Evidence, Judge, Judged, Question, QuestionKind, Reasoned, Reasoning, Schema, Signal,
    SignalKind, Situation, SkillFile, SkillSafety, Template, Threshold, load_layered,
};
use serde::Deserialize;

pub const ID: &str = "skill-review";
const QUESTION: &str = "safe";
/// The skill as System One reads it; a longer skill cannot be cleared there.
const MAX_SCREEN_CHARS: usize = 48_000;
/// Left of the review's input budget for evidence IDs and file headers.
const EVIDENCE_MARGIN: usize = 1_024;
/// Kept back so the list of files not shown always fits.
const UNREAD_RESERVE: usize = 1_024;
const SHIPPED: &str = include_str!("../../../skills/config/skill-review.json");

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillReviewConfig {
    /// A confident yes at or above this bar is safe.
    pub safe: Threshold,
    /// A confident no at or below this bar is unsafe; between the two bars,
    /// or below either confidence, System One is unsure.
    pub unsafe_at: Threshold,
    pub review: ReviewConfig,
    pub texts: SkillReviewTexts,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ReviewConfig {
    pub enabled: bool,
    pub when: Escalation,
    /// The System Two profile in `judge.json`.
    pub profile: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SkillReviewTexts {
    /// System One's question: `{skill}`.
    pub question: Template,
    /// The skill as one text: `{name}`, `{source}`, `{provenance}`,
    /// `{instructions}`, `{files}`, `{unread}`.
    pub skill: Template,
    /// One readable file: `{path}`, `{content}`.
    pub file: Template,
    /// Files not shown: `{paths}`.
    pub unread: Template,
    /// System Two's instructions.
    pub review: Template,
}

impl SkillReviewConfig {
    /// The shipped config overlaid by your `skill-review.json` at `path`.
    ///
    /// # Errors
    ///
    /// When your file is unreadable or invalid, a bar is outside `[0, 1]`, or
    /// a text names a placeholder it may not use.
    pub fn load(path: &Path) -> Result<Self, String> {
        load_layered::<Self>(SHIPPED, path)?
            .checked()
            .map_err(|error| format!("{}: {error}", path.display()))
    }

    fn checked(self) -> Result<Self, String> {
        let texts = &self.texts;

        for (field, template, allowed) in [
            ("texts.question", &texts.question, &["skill"][..]),
            (
                "texts.skill",
                &texts.skill,
                &[
                    "name",
                    "source",
                    "provenance",
                    "instructions",
                    "files",
                    "unread",
                ][..],
            ),
            ("texts.file", &texts.file, &["path", "content"][..]),
            ("texts.unread", &texts.unread, &["paths"][..]),
            ("texts.review", &texts.review, &[][..]),
        ] {
            template.check(field, allowed)?;
        }

        let profile = &self.review.profile;

        if profile.is_empty()
            || !profile
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err("review.profile must be a profile name".into());
        }

        if self.review.enabled && !self.review.when.unsure && !self.review.when.failed {
            return Err("review.when must select unsure or failed".into());
        }

        Ok(self)
    }
}

impl Default for SkillReviewConfig {
    fn default() -> Self {
        serde_json::from_str::<Self>(SHIPPED)
            .map_err(|error| error.to_string())
            .and_then(Self::checked)
            .expect("shipped skill-review defaults are valid")
    }
}

/// What a review decided.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Outcome {
    pub safety: SkillSafety,
    pub judge: &'static str,
    pub reason: String,
    pub concerns: Vec<String>,
}

impl Outcome {
    fn undetermined(reason: impl Into<String>) -> Self {
        Self {
            safety: SkillSafety::Undetermined,
            judge: "",
            reason: reason.into(),
            concerns: Vec::new(),
        }
    }
}

#[derive(Default)]
pub struct SkillReview {
    config: SkillReviewConfig,
}

impl SkillReview {
    #[must_use]
    pub fn with_config(mut self, config: SkillReviewConfig) -> Self {
        self.config = config;
        self
    }
}

/// The skill's parts, borrowed from its signal.
struct Skill<'a> {
    name: &'a str,
    source: &'a str,
    provenance: &'a str,
    instructions: &'a str,
    files: &'a [SkillFile],
}

impl Skill<'_> {
    fn unread(&self) -> Vec<&str> {
        self.files
            .iter()
            .filter(|file| file.content.is_none())
            .map(|file| file.path.as_str())
            .collect()
    }

    /// The whole skill as one text, and whether nothing was left out.
    fn render(&self, texts: &SkillReviewTexts) -> (String, bool) {
        let files: String = self
            .files
            .iter()
            .filter_map(|file| {
                file.content.as_ref().map(|content| {
                    texts
                        .file
                        .render(&[("path", &file.path), ("content", content)])
                })
            })
            .collect();
        let unread = self.unread();
        let unread_text = if unread.is_empty() {
            String::new()
        } else {
            texts.unread.render(&[("paths", &unread.join(", "))])
        };
        let text = texts.skill.render(&[
            ("name", self.name),
            ("source", self.source),
            ("provenance", self.provenance),
            ("instructions", self.instructions),
            ("files", &files),
            ("unread", &unread_text),
        ]);

        match text.char_indices().nth(MAX_SCREEN_CHARS) {
            Some((end, _)) => (text[..end].to_owned(), false),
            None => (text, unread.is_empty()),
        }
    }

    /// The review's evidence within its input budget; what does not fit is
    /// named in `unread`, never silently dropped.
    fn evidence(&self, instructions: &str) -> Vec<Evidence> {
        let mut pack = Pack {
            budget: MAX_REASONING_INPUT_BYTES
                .saturating_sub(instructions.len())
                .saturating_sub(EVIDENCE_MARGIN + UNREAD_RESERVE),
            evidence: Vec::new(),
        };
        let mut unread: Vec<String> = self.unread().into_iter().map(str::to_owned).collect();

        pack.take(
            "provenance",
            format!(
                "Skill {} published at {}.\n{}",
                self.name, self.source, self.provenance
            ),
        );

        if !pack.take("skill_md", self.instructions.into()) {
            let cut = clip(self.instructions, pack.budget.saturating_sub(64));

            pack.take("skill_md", format!("{cut}\n[cut: SKILL.md continues]"));
        }

        for (index, file) in self.files.iter().enumerate() {
            let Some(content) = &file.content else {
                continue;
            };

            if !pack.take(
                &format!("file-{}", index + 1),
                format!("path: {}\n\n{content}", file.path),
            ) {
                unread.push(file.path.clone());
            }
        }

        if !unread.is_empty() {
            let text = format!(
                "Files the skill carries that are not shown: {}",
                unread.join(", ")
            );
            let room = pack.budget + UNREAD_RESERVE - "unread".len();

            pack.evidence.push(Evidence {
                id: "unread".into(),
                text: clip(&text, room).to_owned(),
            });
        }

        pack.evidence
    }
}

/// Evidence gathered within a byte budget.
struct Pack {
    budget: usize,
    evidence: Vec<Evidence>,
}

impl Pack {
    fn take(&mut self, id: &str, text: String) -> bool {
        let cost = id.len() + text.len();

        if cost > self.budget {
            return false;
        }

        self.budget -= cost;
        self.evidence.push(Evidence {
            id: id.into(),
            text,
        });

        true
    }
}

/// At most `bytes` of `text`, on a character boundary.
fn clip(text: &str, bytes: usize) -> &str {
    let mut end = bytes.min(text.len());

    while !text.is_char_boundary(end) {
        end -= 1;
    }

    &text[..end]
}

fn schema() -> Schema {
    Schema::Object(vec![
        (
            "verdict".into(),
            Schema::Enum(
                ["safe", "unsafe", "insufficient_evidence"]
                    .map(str::to_string)
                    .to_vec(),
            ),
        ),
        (
            "concerns".into(),
            Schema::Array {
                items: Box::new(Schema::Object(vec![
                    ("cite".into(), Schema::EvidenceId),
                    ("reason".into(), Schema::String { max_chars: 300 }),
                ])),
                max_items: 6,
            },
        ),
        ("reason".into(), Schema::String { max_chars: 512 }),
    ])
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Verdict {
    verdict: String,
    concerns: Vec<Concern>,
    reason: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Concern {
    cite: String,
    reason: String,
}

/// A review's verdict, held to what it must show: a reason for any verdict,
/// and a cited concern for `unsafe`.
fn read_review(outcome: Reasoned) -> Outcome {
    let verdict = match outcome {
        Ok(value) => match serde_json::from_value::<Verdict>(value) {
            Ok(verdict) => verdict,
            Err(error) => {
                return Outcome::undetermined(format!(
                    "System Two's verdict was unreadable: {error}"
                ));
            }
        },
        Err(failure) => {
            return Outcome::undetermined(format!("System Two gave no verdict ({failure})"));
        }
    };
    let reason = verdict.reason.trim().to_owned();
    let concerns: Vec<String> = verdict
        .concerns
        .iter()
        .filter(|concern| !concern.reason.trim().is_empty())
        .map(|concern| format!("{}: {}", concern.cite, concern.reason.trim()))
        .collect();

    match verdict.verdict.as_str() {
        "safe" if !reason.is_empty() => Outcome {
            safety: SkillSafety::Safe,
            judge: "system_two",
            reason,
            concerns,
        },
        "unsafe" if !concerns.is_empty() => Outcome {
            safety: SkillSafety::Unsafe,
            judge: "system_two",
            reason,
            concerns,
        },
        "insufficient_evidence" => Outcome {
            judge: "system_two",
            concerns,
            ..Outcome::undetermined(if reason.is_empty() {
                "System Two could not tell from what it was shown".to_owned()
            } else {
                reason
            })
        },
        _ => Outcome::undetermined("System Two's verdict gave no reason or cited concern"),
    }
}

impl Judged for SkillReview {
    type Verdict = Outcome;

    fn id(&self) -> &str {
        ID
    }

    fn judge(&mut self, _: &Situation, signal: &Signal) -> Option<Judge<Outcome>> {
        let SignalKind::SkillReview {
            name,
            source,
            provenance,
            instructions,
            files,
        } = &signal.kind
        else {
            return None;
        };
        let skill = Skill {
            name,
            source,
            provenance,
            instructions,
            files,
        };
        let config = &self.config;
        let (text, whole) = skill.render(&config.texts);
        let question = Question {
            id: QUESTION.into(),
            instructions: config.texts.question.render(&[("skill", &text)]),
            kind: QuestionKind::Noul,
        };
        let screen = strategy::screen(question, config.safe.yes(), config.unsafe_at.no())
            // System One cannot clear what it did not see.
            .map(move |screened| match screened {
                Screened::Yes if !whole => Screened::Unsure,
                other => other,
            });
        let review = config.review.enabled.then(|| {
            let instructions = config.texts.review.render(&[]);

            Reasoning {
                id: format!("{QUESTION}/review"),
                profile: config.review.profile.clone(),
                evidence: skill.evidence(&instructions),
                instructions,
                activity: false,
                schema: schema(),
            }
        });
        let when = if review.is_some() {
            config.review.when
        } else {
            Escalation::default()
        };

        Some(strategy::escalate(
            screen,
            when,
            move |screened| match screened {
                Screened::Yes => Outcome {
                    safety: SkillSafety::Safe,
                    judge: "system_one",
                    reason: "System One judged it safe to follow.".into(),
                    concerns: Vec::new(),
                },
                Screened::No => Outcome {
                    safety: SkillSafety::Unsafe,
                    judge: "system_one",
                    reason: "System One judged it unsafe to follow.".into(),
                    concerns: Vec::new(),
                },
                Screened::Unsure if !whole => Outcome::undetermined(
                    "System One could not see the whole skill, and no review is enabled",
                ),
                Screened::Unsure => {
                    Outcome::undetermined("System One was unsure, and no review is enabled")
                }
                Screened::Failed => {
                    Outcome::undetermined("System One gave no answer, and no review is enabled")
                }
            },
            move |_| match review {
                Some(request) => Judge::reason(request, read_review),
                None => Judge::done(Outcome::undetermined("no review is enabled")),
            },
        ))
    }

    fn act(&mut self, signal: &Signal, outcome: Outcome) -> Vec<Effect> {
        vec![Effect::SkillVerdict {
            agent_id: signal.agent_id.clone(),
            safety: outcome.safety,
            judge: outcome.judge.into(),
            reason: outcome.reason,
            concerns: outcome.concerns,
        }]
    }
}
