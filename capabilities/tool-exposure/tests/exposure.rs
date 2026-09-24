use chauffeur_capability_tool_exposure::{ToolExposure, ToolExposureConfig, group};
use chauffeur_core::{
    Answer, AnswerValue, Capability, CatalogEntry, CodeModeNamespace, Delivery, Effect, Plan,
    Signal, SignalKind, Situation,
};

fn tool(id: &str) -> CatalogEntry {
    CatalogEntry {
        id: id.into(),
        description: format!("{id} tool"),
        bytes: 0,
    }
}

fn message(first_in_context: bool, tools: &[&str]) -> Signal {
    Signal {
        agent_id: "ses".into(),
        at: 1,
        kind: SignalKind::UserMessage {
            text: "open a PR for this fix".into(),
            first_in_context,
            skills: Vec::new(),
            tools: tools.iter().copied().map(tool).collect(),
            model: None,
            code_mode: Vec::new(),
        },
    }
}

const CATALOG: &[&str] = &[
    "read",
    "edit",
    "skill",
    "github_create_pr",
    "browser_open",
    "github_merge",
    "jira_view",
];

fn noul(id: &str, probability: f32) -> Answer {
    Answer {
        id: id.into(),
        value: AnswerValue::Noul(probability),
        confidence: None,
    }
}

fn names(tools: &[&str]) -> Vec<String> {
    tools.iter().map(|tool| (*tool).to_string()).collect()
}

fn hidden(tools: &[&str]) -> Vec<Effect> {
    vec![Effect::Tools {
        agent_id: "ses".into(),
        hide: names(tools),
        reveal: Vec::new(),
    }]
}

fn asked(plan: &Plan) -> Vec<String> {
    let Plan::Ask(questions) = plan else {
        panic!("expected questions, got {plan:?}")
    };

    questions
        .iter()
        .map(|question| question.id.clone())
        .collect()
}

fn exposure() -> ToolExposure {
    ToolExposure::new(ToolExposureConfig::default())
}

#[test]
fn a_group_is_the_prefix_before_the_first_underscore() {
    assert_eq!(group("browser_tabs_list"), "browser");
    assert_eq!(group("webfetch"), "webfetch");
}

#[test]
fn the_first_message_asks_once_per_group_of_non_base_tools() {
    let plan = exposure().plan(&Situation::default(), &message(true, CATALOG));

    assert_eq!(asked(&plan), vec!["github", "browser", "jira"]);
}

#[test]
fn hides_every_tool_of_a_confidently_unneeded_group_and_the_skill_tool() {
    let answers = [
        noul("github", 0.9),
        noul("browser", 0.1), // margin 0.8: hidden
        noul("jira", 0.4),    // uncertain: kept
    ];

    assert_eq!(
        exposure().decide(&message(true, CATALOG), Some(&answers)),
        hidden(&["skill", "browser_open"])
    );
}

#[test]
fn classifier_failure_changes_nothing() {
    assert!(exposure().decide(&message(true, CATALOG), None).is_empty());
    assert!(
        exposure()
            .decide(&message(false, &["browser_open"]), None)
            .is_empty()
    );
}

#[test]
fn a_catalog_inside_the_base_set_settles_without_asking() {
    assert_eq!(
        exposure().plan(&Situation::default(), &message(true, &["read", "skill"])),
        Plan::Settled(hidden(&["skill"]))
    );
}

#[test]
fn only_judged_tools_can_be_hidden_so_a_truncated_catalog_keeps_the_rest() {
    // The host truncated its catalog: no base tools were sent at all.
    let signal = message(true, &["browser_open", "jira_view"]);
    let answers = [noul("browser", 0.05), noul("jira", 0.05)];

    assert_eq!(
        exposure().decide(&signal, Some(&answers)),
        hidden(&["browser_open", "jira_view"])
    );
}

#[test]
fn a_later_message_may_bring_a_hidden_group_back() {
    // After the first message the host sends only the currently hidden tools.
    let signal = message(
        false,
        &["skill", "browser_open", "browser_click", "jira_view"],
    );

    assert_eq!(
        asked(&exposure().plan(&Situation::default(), &signal)),
        vec!["browser", "jira"]
    );
    assert_eq!(
        exposure().decide(&signal, Some(&[noul("browser", 0.9), noul("jira", 0.5)])),
        vec![Effect::Tools {
            agent_id: "ses".into(),
            hide: Vec::new(),
            reveal: names(&["browser_open", "browser_click"])
        }]
    );
    assert_eq!(
        exposure().plan(&Situation::default(), &message(false, &["skill"])),
        Plan::Skip
    );
}

fn with_code_mode(first_in_context: bool) -> Signal {
    let mut signal = message(first_in_context, &[]);

    if let SignalKind::UserMessage { code_mode, .. } = &mut signal.kind {
        *code_mode = vec![
            CodeModeNamespace {
                name: "browser".into(),
                size: 45,
                tools: vec![tool("browser_tabs_open")],
            },
            CodeModeNamespace {
                name: "cloudflare".into(),
                size: 3389,
                tools: vec![tool("cloudflare_get_live")],
            },
        ];
    }

    signal
}

#[test]
fn a_code_mode_namespace_the_request_needs_gets_a_note_once_per_context() {
    let mut exposure = exposure();
    let signal = with_code_mode(false);
    let answers = [
        noul("code-mode:browser", 0.9),
        noul("code-mode:cloudflare", 0.1),
    ];

    assert_eq!(
        asked(&exposure.plan(&Situation::default(), &signal)),
        vec!["code-mode:browser", "code-mode:cloudflare"]
    );

    let effects = exposure.decide(&signal, Some(&answers));
    let [
        Effect::Context {
            delivery: Delivery::Prompt,
            text: Some(note),
            ..
        },
    ] = effects.as_slice()
    else {
        panic!("expected a note on the prompt, got {effects:?}")
    };
    assert!(note.contains("## browser (45 tools)\n- browser_tabs_open: browser_tabs_open tool"));
    assert!(note.contains("search({ namespace: \"browser\""));
    assert!(!note.contains("cloudflare"));

    // Surfaced once: the next message asks only about the rest.
    assert_eq!(
        asked(&exposure.plan(&Situation::default(), &signal)),
        vec!["code-mode:cloudflare"]
    );
    // A new context starts over.
    assert_eq!(
        asked(&exposure.plan(&Situation::default(), &with_code_mode(true))),
        vec!["code-mode:browser", "code-mode:cloudflare"]
    );
}
