//! The agent asks for a tool: System One judges every hidden group and Code
//! Mode namespace, and each one that serves the request is granted.

use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig};
use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, CodeModeNamespace, Delivery, Effect, Judging,
    Plan, QuestionKind, Signal, SignalKind, Situation,
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

fn noul(id: &str, probability: f32, confidence: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(probability),
        confidence: Some(confidence),
    }
}

fn exposure() -> Judging<ToolExposure> {
    Judging::new(ToolExposure::new(ToolExposureConfig::default()))
}

/// The yes/no questions asked, by ID; each is worded after the agent's need.
fn asked(plan: &Plan) -> Vec<String> {
    let Plan::Ask(questions) = plan else {
        panic!("expected questions, got {plan:?}")
    };

    questions
        .iter()
        .map(|question| {
            assert!(matches!(question.kind, QuestionKind::Noul));
            assert!(question.instructions.contains("open and click"));
            assert!(question.instructions.contains("landing page"));
            question.id.clone()
        })
        .collect()
}

fn granted(signal: &Signal, answers: Option<&[Answer]>) -> Vec<Effect> {
    let mut exposure = exposure();

    exposure.plan(&Situation::default(), signal);
    exposure.decide(signal, answers)
}

#[test]
fn each_hidden_group_and_namespace_is_asked_but_base_tools_never() {
    let signal = request(
        &["read", "browser_open", "browser_click", "github_merge"],
        &["safari"],
    );

    assert_eq!(
        asked(&exposure().plan(&Situation::default(), &signal)),
        ["tools:browser", "tools:github", "code-mode:safari"]
    );
    assert_eq!(
        exposure().plan(&Situation::default(), &request(&["read"], &[])),
        Plan::Skip
    );
}

#[test]
fn two_groups_and_a_namespace_are_all_granted_together() {
    let signal = request(
        &["browser_open", "browser_click", "github_merge", "jira_view"],
        &["safari", "jina"],
    );
    let answers = [
        noul("tools:browser", 0.9, 0.8),
        noul("tools:github", 0.8, 0.8),
        noul("tools:jira", 0.2, 0.8),
        noul("code-mode:safari", 0.9, 0.8),
        noul("code-mode:jina", 0.1, 0.8),
    ];
    let effects = granted(&signal, Some(&answers));
    let [
        Effect::Tools { hide, reveal, .. },
        Effect::Context {
            delivery: Delivery::Steer,
            text: Some(text),
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected a reveal and a steered note, got {effects:?}")
    };

    assert!(hide.is_empty());
    assert_eq!(reveal, &["browser_open", "browser_click", "github_merge"]);
    assert!(text.contains("safari_open") && !text.contains("jina"));
}

#[test]
fn no_confident_yes_grants_nothing() {
    let signal = request(&["browser_open"], &["safari"]);
    let answers = [
        noul("tools:browser", 0.5, 0.9),
        noul("code-mode:safari", 0.9, 0.2),
    ];

    assert!(granted(&signal, Some(&answers)).is_empty());
}

#[test]
fn a_failed_call_grants_nothing() {
    let signal = request(&["browser_open"], &["safari"]);

    assert!(granted(&signal, None).is_empty());
}
