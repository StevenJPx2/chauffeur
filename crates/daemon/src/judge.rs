//! Provider selection and bounded, opt-in judge fallback.

use std::sync::Arc;
use std::time::{Duration, Instant};

use chauffeur_core::{Answer, Question, SystemOne, SystemOneError, validate_answers};
use chauffeur_judge_jev::{JevClient, JevConfig};
use chauffeur_judge_subq::{SubqClient, SubqConfig};

const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Builds one System One client for each judge worker.
pub type SystemOneFactory = Arc<dyn Fn() -> Result<Box<dyn SystemOne>, String> + Send + Sync>;

#[derive(Clone)]
enum Provider {
    Jev(JevConfig),
    Subq(SubqConfig),
    Custom(SystemOneFactory),
}

impl Provider {
    fn build(&self) -> Result<Box<dyn SystemOne>, String> {
        match self {
            Self::Jev(config) => Ok(Box::new(JevClient::new(config.clone())?)),
            Self::Subq(config) => Ok(Box::new(SubqClient::new(config.clone())?)),
            Self::Custom(factory) => factory(),
        }
    }
}

#[derive(Clone)]
pub struct JudgeConfig {
    primary: Provider,
    fallback: Option<Provider>,
}

impl JudgeConfig {
    /// Jev-only configuration, also used by local daemon fixtures.
    #[must_use]
    pub fn jev(config: JevConfig) -> Self {
        Self {
            primary: Provider::Jev(config),
            fallback: None,
        }
    }

    /// A judge built by `factory`, once for each worker, such as a local
    /// model or a test double.
    #[must_use]
    pub fn custom(factory: SystemOneFactory) -> Self {
        Self {
            primary: Provider::Custom(factory),
            fallback: None,
        }
    }

    /// Only selected providers require credentials. A key alone never enables
    /// fallback, because it sends the same redacted context to another service.
    pub fn from_env() -> Result<Self, String> {
        Self::select(
            &std::env::var("CHAUFFEUR_JUDGE").unwrap_or_else(|_| "jev".into()),
            std::env::var("CHAUFFEUR_JUDGE_FALLBACK").ok().as_deref(),
            JevConfig::from_env(),
            SubqConfig::from_env(),
        )
    }

    fn select(
        primary: &str,
        fallback: Option<&str>,
        jev: Option<JevConfig>,
        subq: Option<SubqConfig>,
    ) -> Result<Self, String> {
        let fallback = fallback.filter(|name| !name.is_empty() && *name != "none");

        if fallback == Some(primary) {
            return Err("CHAUFFEUR_JUDGE_FALLBACK must differ from CHAUFFEUR_JUDGE".into());
        }

        let provider = |name: &str| match name {
            "jev" => jev
                .clone()
                .map(Provider::Jev)
                .ok_or_else(|| "TYPESAFE_API_KEY is required for the selected Jev judge".into()),
            "subq" => subq
                .clone()
                .map(Provider::Subq)
                .ok_or_else(|| "SUBQ_API_KEY is required for the selected SubQ judge".into()),
            _ => Err(format!("unknown judge {name:?}; expected jev or subq")),
        };

        Ok(Self {
            primary: provider(primary)?,
            fallback: fallback.map(provider).transpose()?,
        })
    }

    /// Build on a worker thread, never inside an async runtime. Each worker
    /// builds its own, so a fallback's cooldown is per worker.
    pub(crate) fn build(&self) -> Result<Box<dyn SystemOne>, String> {
        let primary = self.primary.build()?;

        match &self.fallback {
            Some(fallback) => Ok(Box::new(Fallback::new(primary, fallback.build()?))),
            None => Ok(primary),
        }
    }
}

struct Fallback {
    primary: Box<dyn SystemOne>,
    fallback: Box<dyn SystemOne>,
    retry_at: Option<Instant>,
    last_used: String,
}

impl Fallback {
    fn new(primary: Box<dyn SystemOne>, fallback: Box<dyn SystemOne>) -> Self {
        let last_used = primary.name().to_string();

        Self {
            primary,
            fallback,
            retry_at: None,
            last_used,
        }
    }

    fn ask_validated(
        judge: &mut dyn SystemOne,
        state: &str,
        questions: &[Question],
    ) -> Result<Vec<Answer>, SystemOneError> {
        let answers = judge.ask(state, questions)?;
        validate_answers(questions, &answers)?;

        Ok(answers)
    }
}

impl SystemOne for Fallback {
    fn name(&self) -> &str {
        &self.last_used
    }

    fn ask(&mut self, state: &str, questions: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
        let mut primary_error = None;

        if self.retry_at.is_none_or(|at| Instant::now() >= at) {
            self.last_used = self.primary.name().to_string();

            match Self::ask_validated(self.primary.as_mut(), state, questions) {
                Ok(answers) => {
                    self.retry_at = None;
                    return Ok(answers);
                }
                Err(error) => {
                    primary_error = Some(error);
                    self.retry_at = Some(Instant::now() + RETRY_AFTER);
                    eprintln!(
                        "chauffeur: {} judge failed; using {} fallback (primary retry in 60s)",
                        self.primary.name(),
                        self.fallback.name()
                    );
                }
            }
        }

        self.last_used = self.fallback.name().to_string();
        Self::ask_validated(self.fallback.as_mut(), state, questions).map_err(|error| {
            SystemOneError(format!(
                "{}; {} fallback failed: {}",
                primary_error.map_or_else(
                    || format!("{} judge cooling down", self.primary.name()),
                    |error| error.0
                ),
                self.fallback.name(),
                error.0
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chauffeur_core::{AnswerValue, QuestionKind};
    use std::collections::VecDeque;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    struct Fake {
        name: &'static str,
        replies: VecDeque<Result<Vec<Answer>, SystemOneError>>,
        calls: Arc<AtomicUsize>,
    }

    impl SystemOne for Fake {
        fn name(&self) -> &str {
            self.name
        }
        fn ask(&mut self, state: &str, _: &[Question]) -> Result<Vec<Answer>, SystemOneError> {
            assert_eq!(state, "redacted [REDACTED]");
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.replies.pop_front().expect("unexpected judge call")
        }
    }

    fn fake(
        name: &'static str,
        replies: Vec<Result<Vec<Answer>, SystemOneError>>,
    ) -> (Box<dyn SystemOne>, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        (
            Box::new(Fake {
                name,
                replies: replies.into(),
                calls: calls.clone(),
            }),
            calls,
        )
    }

    fn answer(p: f32) -> Result<Vec<Answer>, SystemOneError> {
        Ok(vec![Answer {
            id: "q".into(),
            value: AnswerValue::Noul(p),
            confidence: None,
        }])
    }

    fn ask(judge: &mut Fallback) -> Result<Vec<Answer>, SystemOneError> {
        judge.ask(
            "redacted [REDACTED]",
            &[Question {
                id: "q".into(),
                instructions: "Needed?".into(),
                kind: QuestionKind::Noul,
            }],
        )
    }

    #[test]
    fn valid_low_confidence_answers_never_trigger_fallback() {
        let (primary, pc) = fake("jev", vec![answer(0.5)]);
        let (fallback, fc) = fake("subq", vec![]);
        let mut judge = Fallback::new(primary, fallback);
        assert_eq!(ask(&mut judge).unwrap()[0].value, AnswerValue::Noul(0.5));
        assert_eq!(pc.load(Ordering::SeqCst), 1);
        assert_eq!(fc.load(Ordering::SeqCst), 0);
        assert_eq!(judge.name(), "jev");
    }

    #[test]
    fn failure_uses_fallback_and_skips_primary_until_recovery_probe() {
        let (primary, pc) = fake(
            "jev",
            vec![Err(SystemOneError("HTTP 503".into())), answer(0.9)],
        );
        let (fallback, fc) = fake("subq", vec![answer(0.8), answer(0.8)]);
        let mut judge = Fallback::new(primary, fallback);
        ask(&mut judge).unwrap();
        assert_eq!(judge.name(), "subq");
        ask(&mut judge).unwrap();
        assert_eq!(pc.load(Ordering::SeqCst), 1);
        assert_eq!(fc.load(Ordering::SeqCst), 2);
        judge.retry_at = Some(Instant::now());
        ask(&mut judge).unwrap();
        assert_eq!(pc.load(Ordering::SeqCst), 2);
        assert_eq!(fc.load(Ordering::SeqCst), 2);
        assert_eq!(judge.name(), "jev");
        assert!(judge.retry_at.is_none());
    }

    #[test]
    fn invalid_primary_answers_fall_back_and_invalid_fallback_fails_closed() {
        let (primary, _) = fake("jev", vec![answer(2.0)]);
        let (fallback, _) = fake("subq", vec![answer(0.9)]);
        assert!(ask(&mut Fallback::new(primary, fallback)).is_ok());
        let (primary, _) = fake("subq", vec![Err(SystemOneError("HTTP 402".into()))]);
        let (fallback, _) = fake("jev", vec![Ok(vec![])]);
        let error = ask(&mut Fallback::new(primary, fallback)).unwrap_err();
        assert!(error.0.contains("HTTP 402"));
        assert!(error.0.contains("jev fallback failed"));
    }

    #[test]
    fn selection_requires_only_selected_credentials_and_rejects_typos() {
        let jev = JevConfig {
            base_url: "http://localhost".into(),
            api_key: "test".into(),
            model: "jev".into(),
            timeout: Duration::from_secs(3),
            record: None,
        };
        let subq = SubqConfig {
            base_url: "http://localhost".into(),
            api_key: "test".into(),
            model: "subq".into(),
            timeout: Duration::from_secs(3),
            record: None,
        };
        assert!(JudgeConfig::select("jev", None, Some(jev.clone()), None).is_ok());
        assert!(JudgeConfig::select("subq", None, None, Some(subq.clone())).is_ok());
        assert!(JudgeConfig::select("jev", Some("subq"), Some(jev.clone()), None).is_err());
        assert!(JudgeConfig::select("subq", Some("jev"), None, Some(subq.clone())).is_err());
        assert!(
            JudgeConfig::select("jev", Some("subq"), Some(jev.clone()), Some(subq.clone())).is_ok()
        );
        assert!(
            JudgeConfig::select("subq", Some("jev"), Some(jev.clone()), Some(subq.clone())).is_ok()
        );
        assert!(JudgeConfig::select("jev", Some("jev"), Some(jev.clone()), None).is_err());
        assert!(JudgeConfig::select("typo", None, Some(jev.clone()), Some(subq.clone())).is_err());
        assert!(JudgeConfig::select("jev", Some("typo"), Some(jev.clone()), Some(subq)).is_err());
        assert!(JudgeConfig::select("jev", Some("none"), Some(jev), None).is_ok());
    }
}
