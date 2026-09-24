use std::sync::Arc;

use chauffeur_capability_model_router::{
    MAX_CANDIDATES, ModelRouter, ModelRouterConfig, STAY, SWITCH_BACK_AFTER_SECS, is_limit_error,
    is_unusable_error,
};
use chauffeur_core::{
    Answer, AnswerValue, AvailableModel, Capability, Effect, ModelRef, Plan, Provider,
    QuestionKind, Signal, SignalKind, Situation, Tier, TierEntry,
};

struct Table(&'static str, Vec<TierEntry>);

impl Provider for Table {
    fn id(&self) -> &str {
        self.0
    }

    fn tiers(&self) -> &[TierEntry] {
        &self.1
    }
}

/// Rows as `(model, tier)` for every variant.
fn rows(entries: &[(&'static str, Tier)]) -> Vec<TierEntry> {
    entries
        .iter()
        .map(|(model, tier)| TierEntry {
            model,
            variant: None,
            tier: *tier,
        })
        .collect()
}

fn model_ref(key: &str) -> ModelRef {
    model(key)
}

/// `provider/model` or `provider/model#variant`.
fn model(key: &str) -> ModelRef {
    let (provider, rest) = key.split_once('/').expect("provider/model");
    let (id, variant) = match rest.split_once('#') {
        Some((id, variant)) => (id, Some(variant.to_string())),
        None => (rest, None),
    };

    ModelRef {
        provider: provider.into(),
        model: id.into(),
        variant,
    }
}

fn router(pins: &[&str]) -> ModelRouter {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(Table(
            "anthropic",
            rows(&[("opus", Tier::Frontier), ("haiku", Tier::Fast)]),
        )),
        Arc::new(Table(
            "openai",
            rows(&[
                ("sol", Tier::Frontier),
                ("luna", Tier::Frontier),
                ("spark", Tier::Fast),
            ]),
        )),
    ];
    let config = ModelRouterConfig {
        pins: pins.iter().map(|pin| (*pin).to_string()).collect(),
    };

    ModelRouter::new(providers, config)
}

fn limit(tool_executed: bool) -> Signal {
    let available = [
        "anthropic/opus",
        "anthropic/haiku",
        "openai/sol",
        "openai/luna",
        "openai/spark",
    ]
    .into_iter()
    .map(|key| AvailableModel {
        model: model(key),
        usable: true,
    })
    .collect();

    Signal {
        agent_id: "session".into(),
        at: 1,
        kind: SignalKind::ModelError {
            model: model("anthropic/opus"),
            error_type: "rate_limit_error".into(),
            status: Some(429),
            message: "usage limit reached".into(),
            tool_executed,
            available,
        },
    }
}

fn options(plan: &Plan) -> Vec<String> {
    let Plan::Ask(questions) = plan else {
        panic!("expected a question, got {plan:?}")
    };
    let [question] = questions.as_slice() else {
        panic!("expected one question")
    };
    let QuestionKind::Choice { options } = &question.kind else {
        panic!("expected a choice")
    };

    options.iter().map(|option| option.value.clone()).collect()
}

fn choice(value: &str, confidence: f32) -> Answer {
    Answer {
        id: "choice".into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(confidence),
    }
}

fn switch(key: &str) -> Vec<Effect> {
    vec![Effect::SwitchModel {
        agent_id: "session".into(),
        model: model(key),
    }]
}

#[test]
fn asks_among_same_tier_candidates_plus_stay() {
    let plan = router(&[]).plan(&Situation::default(), &limit(false));

    assert_eq!(options(&plan), vec!["openai/sol", "openai/luna", STAY]);
}

#[test]
fn pins_order_the_candidates() {
    let plan = router(&["openai/luna"]).plan(&Situation::default(), &limit(false));

    assert_eq!(options(&plan), vec!["openai/luna", "openai/sol", STAY]);
}

#[test]
fn applies_the_judged_choice_and_never_retries_it() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice("openai/luna", 0.8)][..])),
        switch("openai/luna")
    );
    assert_eq!(
        options(&router.plan(&Situation::default(), &signal)),
        vec!["openai/sol", STAY]
    );
}

#[test]
fn stay_keeps_the_model() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice(STAY, 0.9)][..])),
        vec![Effect::KeepModel {
            agent_id: "session".into()
        }]
    );
}

#[test]
fn classifier_failure_or_low_confidence_switches_to_the_preferred_candidate() {
    let mut router = router(&["openai/luna"]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);

    assert_eq!(
        router.decide(&signal, Some(&[choice(STAY, 0.05)][..])),
        switch("openai/luna")
    );
    assert_eq!(router.decide(&signal, None), switch("openai/sol"));
}

#[test]
fn a_tool_that_already_ran_blocks_failover_without_asking() {
    let plan = router(&[]).plan(&Situation::default(), &limit(true));

    assert_eq!(
        plan,
        Plan::Settled(vec![Effect::KeepModel {
            agent_id: "session".into()
        }])
    );
}

#[test]
fn success_resets_attempts_and_non_limit_errors_are_ignored() {
    let mut router = router(&[]);
    let signal = limit(false);
    router.plan(&Situation::default(), &signal);
    router.decide(&signal, Some(&[choice("openai/sol", 0.9)][..]));

    let success = Signal {
        agent_id: "session".into(),
        at: 2,
        kind: SignalKind::ModelSucceeded {
            model: model("openai/sol"),
        },
    };

    assert_eq!(router.plan(&Situation::default(), &success), Plan::Skip);
    assert_eq!(
        options(&router.plan(&Situation::default(), &signal)),
        vec!["openai/sol", "openai/luna", STAY]
    );
    assert!(!is_limit_error("invalid_request", Some(400), "bad schema"));
    assert!(is_limit_error("Rate Limit", None, ""));
    // OpenCode's own classification of an exhausted provider balance.
    assert!(is_limit_error("provider.quota", None, ""));
    assert!(is_limit_error("provider.invalid-request", Some(402), ""));
    // A provider out of capacity.
    assert!(is_limit_error(
        "provider.internal",
        Some(503),
        "This model is currently experiencing high demand."
    ));
    assert!(!is_limit_error(
        "provider.invalid-request",
        Some(400),
        "unknown field"
    ));
    // Anthropic reports an empty balance as an invalid request.
    assert!(is_limit_error(
        "provider.invalid-request",
        Some(400),
        "Your credit balance is too low to access the Anthropic API."
    ));
}

#[test]
fn an_unknown_tier_offers_a_bounded_pinned_first_choice() {
    let mut router = router(&["google/pinned"]);
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model, available, ..
    } = &mut signal.kind
    {
        *model = model_ref("unknown/current");
        available.extend((0..40).map(|n| AvailableModel {
            model: model_ref(&format!("many/m{n}")),
            usable: true,
        }));
        available.push(AvailableModel {
            model: model_ref("google/pinned"),
            usable: true,
        });
    }

    let offered = options(&router.plan(&Situation::default(), &signal));

    assert_eq!(offered.len(), MAX_CANDIDATES + 1);
    assert_eq!(offered[0], "google/pinned");
    assert_eq!(offered.last().map(String::as_str), Some(STAY));
}

#[test]
fn a_pinned_model_is_a_candidate_across_tiers() {
    let mut router = router(&["anthropic/haiku"]);

    assert_eq!(
        options(&router.plan(&Situation::default(), &limit(false))),
        vec!["anthropic/haiku", "openai/sol", "openai/luna", STAY]
    );
}

fn user_message(at: u64, current: &str) -> Signal {
    Signal {
        agent_id: "session".into(),
        at,
        kind: SignalKind::UserMessage {
            text: "keep going".into(),
            first_in_context: false,
            skills: Vec::new(),
            tools: Vec::new(),
            model: Some(model(current)),
            code_mode: Vec::new(),
        },
    }
}

fn switch_back(p: f32) -> Answer {
    Answer {
        id: "switch_back".into(),
        value: AnswerValue::Noul(p),
        confidence: None,
    }
}

/// Fail over from anthropic/opus to openai/sol at t=1.
fn switched_router() -> ModelRouter {
    let mut router = router(&[]);
    let signal = limit(false);
    let chosen = Answer {
        id: "choice".into(),
        value: AnswerValue::Choice("openai/sol".into()),
        confidence: Some(0.9),
    };

    router.plan(&Situation::default(), &signal);
    assert!(matches!(
        router.decide(&signal, Some(&[chosen])).as_slice(),
        [Effect::SwitchModel { model, .. }] if model.key() == "openai/sol"
    ));

    router
}

#[test]
fn switching_back_is_judged_only_after_the_wait_and_on_the_switched_model() {
    let mut router = switched_router();
    let after = 1 + SWITCH_BACK_AFTER_SECS;

    assert_eq!(
        router.plan(&Situation::default(), &user_message(60, "openai/sol")),
        Plan::Skip
    );

    let Plan::Ask(questions) =
        router.plan(&Situation::default(), &user_message(after, "openai/sol"))
    else {
        panic!("expected a switch-back question")
    };
    assert!(
        questions[0]
            .instructions
            .contains("model anthropic/opus failed")
    );

    assert_eq!(
        router.decide(
            &user_message(after, "openai/sol"),
            Some(&[switch_back(0.9)])
        ),
        vec![Effect::SwitchModel {
            agent_id: "session".into(),
            model: model("anthropic/opus")
        }]
    );
    // Back on the original model: nothing more to judge.
    assert_eq!(
        router.plan(
            &Situation::default(),
            &user_message(after + 1, "anthropic/opus")
        ),
        Plan::Skip
    );
}

#[test]
fn an_unconvinced_judgment_stays_and_a_manual_model_change_forgets_the_origin() {
    let mut router = switched_router();
    let after = 1 + SWITCH_BACK_AFTER_SECS;

    assert!(
        router
            .decide(
                &user_message(after, "openai/sol"),
                Some(&[switch_back(0.6)])
            )
            .is_empty()
    );
    assert!(
        router
            .decide(&user_message(after, "openai/sol"), None)
            .is_empty()
    );

    // The user picked another model themselves.
    assert_eq!(
        router.plan(&Situation::default(), &user_message(after, "openai/luna")),
        Plan::Skip
    );
    assert_eq!(
        router.plan(
            &Situation::default(),
            &user_message(after + 1, "openai/sol")
        ),
        Plan::Skip
    );
}

fn auth_error(on: &str) -> Signal {
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model,
        error_type,
        status,
        message,
        ..
    } = &mut signal.kind
    {
        *model = model_ref(on);
        *error_type = "provider.auth".into();
        *status = Some(401);
        *message = "Invalid API key".into();
    }

    signal.at = 2;
    signal
}

#[test]
fn a_switch_to_an_unusable_model_moves_on_to_the_next_candidate() {
    let mut router = switched_router();

    // openai/sol, which the router chose, rejects the credentials.
    let offered = options(&router.plan(&Situation::default(), &auth_error("openai/sol")));
    assert_eq!(offered, vec!["openai/luna", STAY]);

    // The same error on a model the user chose is not the router's to fix.
    assert_eq!(
        router.plan(&Situation::default(), &auth_error("anthropic/opus")),
        Plan::Skip
    );
    assert!(is_unusable_error("provider.auth", None, ""));
    assert!(!is_unusable_error(
        "provider.quota",
        Some(402),
        "insufficient funds"
    ));
}

#[test]
fn the_model_left_behind_survives_a_restart() {
    let before = switched_router();
    let mut after = router(&[]);

    after.load(before.save().expect("the router keeps memory"));

    let Plan::Ask(questions) = after.plan(
        &Situation::default(),
        &user_message(1 + SWITCH_BACK_AFTER_SECS, "openai/sol"),
    ) else {
        panic!("expected a switch-back question after the restart")
    };
    assert!(
        questions[0]
            .instructions
            .contains("model anthropic/opus failed")
    );
}

/// The shipped tables, with the host's models listed without variants.
fn shipped_router() -> ModelRouter {
    let providers: Vec<Arc<dyn Provider>> = vec![
        Arc::new(chauffeur_plugin_anthropic::AnthropicProvider),
        Arc::new(chauffeur_plugin_openai::OpenAiProvider),
    ];

    ModelRouter::new(providers, ModelRouterConfig::default())
}

fn limit_on(current: &str) -> Signal {
    let mut signal = limit(false);

    if let SignalKind::ModelError {
        model: failed,
        available,
        ..
    } = &mut signal.kind
    {
        *failed = model(current);
        *available = [
            "anthropic/claude-opus-5-5",
            "anthropic/claude-sonnet-4-6",
            "openai/gpt-6-sol",
            "openai/gpt-6-luna",
        ]
        .into_iter()
        .map(|key| AvailableModel {
            model: model(key),
            usable: true,
        })
        .collect();
    }

    signal
}

#[test]
fn tiers_follow_thinking_variants_and_never_offer_the_same_model() {
    let mut router = shipped_router();

    // Opus at high thinking is frontier: only gpt-6-sol matches.
    assert_eq!(
        options(&router.plan(
            &Situation::default(),
            &limit_on("anthropic/claude-opus-5-5#high")
        )),
        vec!["openai/gpt-6-sol", STAY]
    );
    // Opus at low thinking is balanced: gpt-6-luna at max thinking.
    assert_eq!(
        options(&shipped_router().plan(
            &Situation::default(),
            &limit_on("anthropic/claude-opus-5-5#low")
        )),
        vec!["openai/gpt-6-luna#max", STAY]
    );
    // Fast: sonnet 4.6 and gpt-6-luna at its default thinking.
    assert_eq!(
        options(&shipped_router().plan(
            &Situation::default(),
            &limit_on("anthropic/claude-sonnet-4-6")
        )),
        vec!["openai/gpt-6-luna", STAY]
    );

    // A switch carries the variant.
    let signal = limit_on("anthropic/claude-opus-5-5#low");
    router.plan(&Situation::default(), &signal);
    let pick = Answer {
        id: "choice".into(),
        value: AnswerValue::Choice("openai/gpt-6-luna#max".into()),
        confidence: Some(0.9),
    };
    assert!(matches!(
        router.decide(&signal, Some(&[pick])).as_slice(),
        [Effect::SwitchModel { model, .. }] if model.variant.as_deref() == Some("max") && model.model == "gpt-6-luna"
    ));
}
