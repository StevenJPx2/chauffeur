//! The agent asks for a tool: System One picks a hidden group or Code Mode
//! namespace, or none.

use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, ChoiceOption, CodeModeNamespace, Delivery,
    Effect, PipeStep, Plan, QuestionKind, Signal, SignalKind, Situation,
};

fn tool(id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} tool"),
        bytes: 0,
    }
}

fn request(tools: &[&str], namespaces: &[&str]) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::AgentRequest {
            need: "a way to open and click through a web page".into(),
            user_request: "check the landing page renders".into(),
            tools: tools.iter().copied().map(tool).collect(),
            code_mode: namespaces
                .iter()
                .map(|name| CodeModeNamespace {
                    name: (*name).into(),
                    size: 12,
                    tools: vec![tool(&format!("{name}_open"))],
                })
                .collect(),
        },
    }
}

fn choice(value: &str, confidence: f32) -> Answer {
    Answer {
        id: "request".into(),
        value: AnswerValue::Choice(value.into()),
        confidence: Some(confidence),
    }
}

fn exposure() -> ToolExposure {
    ToolExposure::new(ToolExposureConfig::default())
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

    assert!(question.instructions.contains("open and click"));
    options
        .iter()
        .map(|ChoiceOption { value, .. }| value.clone())
        .collect()
}

fn granted(exposure: &mut ToolExposure, signal: &Signal, answer: Answer) -> Vec<Effect> {
    match exposure.advance(signal, Some(&[answer]), 1) {
        PipeStep::Done(effects) => effects,
        PipeStep::Next(_) => panic!("a request is one round"),
    }
}

#[test]
fn hidden_groups_and_namespaces_are_offered_but_base_tools_never() {
    let signal = request(
        &["read", "browser_open", "browser_click", "github_merge"],
        &["safari"],
    );

    assert_eq!(
        options(&exposure().plan(&Situation::default(), &signal)),
        ["tools:browser", "tools:github", "code-mode:safari", "none"]
    );
    assert_eq!(
        exposure().plan(&Situation::default(), &request(&["read"], &[])),
        Plan::Skip
    );
}

#[test]
fn a_confident_group_pick_reveals_the_whole_group() {
    let signal = request(&["browser_open", "browser_click", "github_merge"], &[]);
    let mut exposure = exposure();

    exposure.plan(&Situation::default(), &signal);

    assert_eq!(
        granted(&mut exposure, &signal, choice("tools:browser", 0.9)),
        vec![Effect::Tools {
            agent_id: "ses".into(),
            hide: Vec::new(),
            reveal: vec!["browser_open".into(), "browser_click".into()],
        }]
    );
}

#[test]
fn a_namespace_pick_steers_its_matches_into_the_turn() {
    let signal = request(&[], &["safari", "jina"]);
    let mut exposure = exposure();

    exposure.plan(&Situation::default(), &signal);

    let effects = granted(&mut exposure, &signal, choice("code-mode:safari", 0.9));

    assert!(matches!(
        effects.as_slice(),
        [Effect::Context { delivery: Delivery::Steer, text: Some(text), .. }]
            if text.contains("safari_open") && !text.contains("jina")
    ));
}

#[test]
fn none_an_unsure_pick_an_unknown_option_or_a_failure_grants_nothing() {
    let signal = request(&["browser_open"], &["safari"]);

    for answers in [
        Some(vec![choice("none", 0.9)]),
        Some(vec![choice("tools:browser", 0.2)]),
        Some(vec![choice("tools:shell", 0.9)]),
        None,
    ] {
        let mut exposure = exposure();

        exposure.plan(&Situation::default(), &signal);

        assert!(matches!(
            exposure.advance(&signal, answers.as_deref(), 1),
            PipeStep::Done(effects) if effects.is_empty()
        ));
    }
}
