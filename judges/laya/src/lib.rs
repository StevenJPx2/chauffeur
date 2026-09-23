//! Native client for the warm Laya Core ML daemon.
//!
//! The wire protocol is newline-delimited JSON over a Unix socket. Laya
//! answers typed questions only; no generated text crosses this boundary.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chauffeur_core::{
    Brief, DecisionAnswer, DecisionQuestion, DecisionValue, Error, Judge, QuestionType, Rule,
    Urgency, Verdict,
};
use serde::{Deserialize, Serialize};

pub const DEFAULT_SOCKET_RELATIVE: &str = "Library/Application Support/laya/laya.sock";
const SOCKET_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_MESSAGE_BYTES: usize = 65_536;
const URGENCY_CRITERIA: [&str; 3] = ["routine", "important", "urgent"];

#[derive(Serialize)]
struct PredictRequest {
    state: String,
    questions: HashMap<String, Question>,
}

#[derive(Serialize)]
struct Question {
    #[serde(rename = "type")]
    kind: &'static str,
    instructions: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    criteria: Option<Vec<String>>,
}

#[derive(Serialize)]
struct SkillPredictRequest {
    state: String,
    questions: HashMap<String, Question>,
}

#[derive(Deserialize)]
struct PredictResponse {
    answers: HashMap<String, Answer>,
}

#[derive(Deserialize)]
struct Answer {
    #[serde(rename = "type")]
    kind: String,
    confidence: Option<f32>,
    choice: Option<String>,
    noul: Option<f32>,
    score: Option<f32>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct LayaHealth {
    pub status: String,
    pub model: String,
    pub warm: bool,
}

#[derive(Deserialize)]
struct ErrorResponse {
    error: String,
}

pub struct LayaJudge {
    socket: BufReader<UnixStream>,
}

impl LayaJudge {
    /// Connect to the installed Laya daemon and verify that its model is warm.
    pub fn connect(path: &Path) -> Result<Self, String> {
        let stream = UnixStream::connect(path)
            .map_err(|error| format!("connect to Laya socket {}: {error}", path.display()))?;
        stream
            .set_read_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|error| format!("configure Laya read timeout: {error}"))?;
        stream
            .set_write_timeout(Some(SOCKET_TIMEOUT))
            .map_err(|error| format!("configure Laya write timeout: {error}"))?;

        let mut judge = Self {
            socket: BufReader::new(stream),
        };
        let health = judge.health()?;

        if health.status != "ok" || !health.warm {
            return Err(format!(
                "Laya daemon is not ready: status={}, warm={}",
                health.status, health.warm
            ));
        }

        Ok(judge)
    }

    /// Ask the daemon for its model readiness.
    pub fn health(&mut self) -> Result<LayaHealth, String> {
        self.request(&serde_json::json!({ "op": "health" }))
    }

    fn request<T, R>(&mut self, request: &T) -> Result<R, String>
    where
        T: Serialize,
        R: serde::de::DeserializeOwned,
    {
        let mut encoded =
            serde_json::to_vec(request).map_err(|error| format!("encode Laya request: {error}"))?;

        if encoded.len() > MAX_MESSAGE_BYTES {
            return Err(format!("Laya request exceeds {MAX_MESSAGE_BYTES} bytes"));
        }

        encoded.push(b'\n');

        self.socket
            .get_mut()
            .write_all(&encoded)
            .and_then(|()| self.socket.get_mut().flush())
            .map_err(|error| format!("write Laya request: {error}"))?;

        let limit = u64::try_from(MAX_MESSAGE_BYTES.saturating_add(1))
            .map_err(|error| error.to_string())?;
        let mut response = String::new();
        let bytes = (&mut self.socket)
            .take(limit)
            .read_line(&mut response)
            .map_err(|error| format!("read Laya response: {error}"))?;

        if bytes == 0 {
            return Err("Laya daemon closed the socket".to_string());
        }
        if response.len() > MAX_MESSAGE_BYTES {
            return Err(format!("Laya response exceeds {MAX_MESSAGE_BYTES} bytes"));
        }

        decode_response(&response)
    }
}

impl Judge for LayaJudge {
    fn judge(&mut self, brief: &Brief, rules: &[&Rule]) -> Result<Vec<Verdict>, Error> {
        let request = predict_request(brief, rules);
        let response: PredictResponse = self.request(&request).map_err(Error::Judge)?;

        verdicts_from(response, rules).map_err(Error::Judge)
    }

    fn decide_skill(
        &mut self,
        state: &str,
        question: &DecisionQuestion,
    ) -> Result<DecisionAnswer, Error> {
        let request = skill_request(state, question);
        let response: PredictResponse = self.request(&request).map_err(Error::Judge)?;

        skill_answer(response, question).map_err(Error::Judge)
    }
}

#[must_use]
pub fn default_socket_path(home: &Path) -> PathBuf {
    home.join(DEFAULT_SOCKET_RELATIVE)
}

fn predict_request(brief: &Brief, rules: &[&Rule]) -> PredictRequest {
    let mut questions = HashMap::with_capacity(rules.len().saturating_mul(2));

    for rule in rules {
        questions.insert(
            applies_id(&rule.id),
            Question {
                kind: "noul",
                instructions: rule.situation.clone(),
                criteria: None,
            },
        );
        questions.insert(
            urgency_id(&rule.id),
            Question {
                kind: "score",
                instructions: format!(
                    "How urgently does this agent need a supervisor's nudge about this: {}",
                    rule.situation
                ),
                criteria: Some(
                    URGENCY_CRITERIA
                        .iter()
                        .map(|item| (*item).to_string())
                        .collect(),
                ),
            },
        );
    }

    PredictRequest {
        state: brief.to_prose(),
        questions,
    }
}

fn skill_request(state: &str, question: &DecisionQuestion) -> SkillPredictRequest {
    let kind = match question.kind {
        QuestionType::Choice => "choice",
        QuestionType::Score => "score",
        QuestionType::Noul => "noul",
    };
    let criteria = (!question.criteria.is_empty()).then(|| {
        question
            .criteria
            .iter()
            .map(|criterion| criterion.value.clone())
            .collect()
    });
    let mut questions = HashMap::with_capacity(1);

    questions.insert(
        question.id.clone(),
        Question {
            kind,
            instructions: question.instructions.clone(),
            criteria,
        },
    );

    SkillPredictRequest {
        state: state.to_string(),
        questions,
    }
}

fn skill_answer(
    response: PredictResponse,
    question: &DecisionQuestion,
) -> Result<DecisionAnswer, String> {
    if response.answers.len() != 1 {
        return Err(format!(
            "expected one Laya answer, got {}",
            response.answers.len()
        ));
    }

    let answer = response
        .answers
        .get(&question.id)
        .ok_or_else(|| format!("missing Laya answer {}", question.id))?;
    let expected_type = match question.kind {
        QuestionType::Choice => "choice",
        QuestionType::Score => "score",
        QuestionType::Noul => "noul",
    };

    if answer.kind != expected_type {
        return Err(format!(
            "Laya answer {} has type {}, expected {expected_type}",
            question.id, answer.kind
        ));
    }

    let confidence = answer
        .confidence
        .ok_or_else(|| format!("Laya answer {} has no confidence", question.id))?;
    let value = skill_value(answer, question)?;

    Ok(DecisionAnswer {
        question_id: question.id.clone(),
        confidence,
        value,
    })
}

fn skill_value(answer: &Answer, question: &DecisionQuestion) -> Result<DecisionValue, String> {
    match question.kind {
        QuestionType::Choice => {
            if answer.noul.is_some() || answer.score.is_some() {
                return Err(format!(
                    "Laya answer {} has malformed choice value",
                    question.id
                ));
            }

            answer
                .choice
                .as_ref()
                .map(|choice| DecisionValue::Choice(choice.clone()))
                .ok_or_else(|| format!("Laya answer {} has no choice value", question.id))
        }
        QuestionType::Score => {
            if answer.choice.is_some() || answer.noul.is_some() {
                return Err(format!(
                    "Laya answer {} has malformed score value",
                    question.id
                ));
            }

            answer
                .score
                .map(DecisionValue::Score)
                .ok_or_else(|| format!("Laya answer {} has no score value", question.id))
        }
        QuestionType::Noul => {
            if answer.choice.is_some() || answer.score.is_some() {
                return Err(format!(
                    "Laya answer {} has malformed noul value",
                    question.id
                ));
            }

            answer
                .noul
                .map(DecisionValue::Noul)
                .ok_or_else(|| format!("Laya answer {} has no noul value", question.id))
        }
    }
}

fn verdicts_from(response: PredictResponse, rules: &[&Rule]) -> Result<Vec<Verdict>, String> {
    let expected = rules.len().saturating_mul(2);

    if response.answers.len() != expected {
        return Err(format!(
            "expected {expected} Laya answers, got {}",
            response.answers.len()
        ));
    }

    let mut seen = HashSet::with_capacity(rules.len());
    let mut verdicts = Vec::with_capacity(rules.len());

    for rule in rules {
        let applies = answer_noul(&response.answers, &rule.id)?;
        let urgency = answer_score(&response.answers, &rule.id)?;
        seen.insert(applies_id(&rule.id));
        seen.insert(urgency_id(&rule.id));
        verdicts.push(Verdict {
            rule_id: rule.id.clone(),
            applies,
            urgency: Urgency::from_score(urgency),
        });
    }

    if response.answers.keys().any(|id| !seen.contains(id)) {
        return Err("Laya returned an answer for an unknown question".to_string());
    }

    Ok(verdicts)
}

fn answer_noul(answers: &HashMap<String, Answer>, rule_id: &str) -> Result<f32, String> {
    let id = applies_id(rule_id);
    let answer = answers
        .get(&id)
        .ok_or_else(|| format!("missing Laya answer {id}"))?;

    if answer.kind != "noul" {
        return Err(format!(
            "Laya answer {id} has type {}, expected noul",
            answer.kind
        ));
    }

    let probability = answer
        .noul
        .ok_or_else(|| format!("Laya answer {id} has no noul value"))?;

    if !(0.0..=1.0).contains(&probability) {
        return Err(format!("Laya probability {probability} is outside [0, 1]"));
    }

    Ok(probability)
}

fn answer_score(answers: &HashMap<String, Answer>, rule_id: &str) -> Result<f32, String> {
    let id = urgency_id(rule_id);
    let answer = answers
        .get(&id)
        .ok_or_else(|| format!("missing Laya answer {id}"))?;

    if answer.kind != "score" {
        return Err(format!(
            "Laya answer {id} has type {}, expected score",
            answer.kind
        ));
    }

    let score = answer
        .score
        .ok_or_else(|| format!("Laya answer {id} has no score value"))?;

    if !score.is_finite() || !(0.0..=2.0).contains(&score) {
        return Err(format!("Laya score {score} is outside [0, 2]"));
    }

    Ok(score)
}

fn decode_response<R: serde::de::DeserializeOwned>(line: &str) -> Result<R, String> {
    if let Ok(error) = serde_json::from_str::<ErrorResponse>(line) {
        return Err(format!("Laya daemon: {}", error.error));
    }

    serde_json::from_str(line.trim()).map_err(|error| format!("decode Laya response: {error}"))
}

fn applies_id(rule_id: &str) -> String {
    format!("{rule_id}::applies")
}

fn urgency_id(rule_id: &str) -> String {
    format!("{rule_id}::urgency")
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::net::UnixListener;
    use std::thread;

    use chauffeur_core::{AgentContext, Gate, Threshold};

    use super::*;

    fn rule(id: &str) -> Rule {
        Rule {
            id: id.into(),
            name: id.into(),
            situation: "The agent needs help.".into(),
            gate: Gate::default(),
            reminder: "help".into(),
            priority: 0,
            once: false,
            cooldown_secs: 0,
            threshold: Threshold::new(0.5).expect("valid threshold"),
        }
    }

    fn brief() -> Brief {
        AgentContext {
            agent_id: "agent".into(),
            status: "implementing".into(),
            source: String::new(),
            tool_history: Vec::new(),
            notifications: Vec::new(),
            hooks: Vec::new(),
            idle_at: 1,
        }
        .brief()
    }

    #[test]
    fn native_response_maps_to_existing_verdict_contract() {
        let pr = rule("pr");
        let response: PredictResponse = serde_json::from_str(
            r#"{"model":"laya","answers":{"pr::applies":{"type":"noul","confidence":0.9,"action":{"act_probability":1},"noul":0.91},"pr::urgency":{"type":"score","confidence":0.8,"action":{"act_probability":1},"score":1.4}},"usage":{"input_tokens":10,"output_tokens":0}}"#,
        )
        .expect("valid native response");

        let verdicts = verdicts_from(response, &[&pr]).expect("valid verdicts");

        assert_eq!(verdicts[0].rule_id, "pr");
        assert_eq!(verdicts[0].applies, 0.91);
        assert_eq!(verdicts[0].urgency, Urgency::Important);
    }

    #[test]
    fn malformed_native_answers_fail_closed() {
        let pr = rule("pr");
        let missing: PredictResponse =
            serde_json::from_str(r#"{"answers":{}}"#).expect("valid JSON");
        let wrong_type: PredictResponse = serde_json::from_str(
            r#"{"answers":{"pr::applies":{"type":"score","noul":0.8},"pr::urgency":{"type":"score","score":1}}}"#,
        )
        .expect("valid JSON");

        assert!(verdicts_from(missing, &[&pr]).is_err());
        assert!(verdicts_from(wrong_type, &[&pr]).is_err());
        assert!(decode_response::<PredictResponse>(r#"{"error":"bad request"}"#).is_err());
    }

    #[test]
    fn socket_client_checks_health_and_predicts_over_jsonl() {
        let path = std::env::temp_dir().join(format!("chauffeur-laya-{}.sock", std::process::id()));
        let _ = fs::remove_file(&path);
        let listener = UnixListener::bind(&path).expect("bind test socket");
        let server = thread::spawn(move || serve_fixture(listener));
        let pr = rule("pr");
        let mut judge = LayaJudge::connect(&path).expect("healthy daemon");

        let verdicts = judge.judge(&brief(), &[&pr]).expect("prediction");

        assert_eq!(verdicts[0].rule_id, "pr");
        assert_eq!(verdicts[0].urgency, Urgency::Urgent);
        server.join().expect("server exits cleanly");
        let _ = fs::remove_file(path);
    }

    fn serve_fixture(listener: UnixListener) {
        let (stream, _) = listener.accept().expect("accept client");
        let mut reader = BufReader::new(stream);
        let mut health = String::new();
        reader.read_line(&mut health).expect("read health");
        let value: serde_json::Value = serde_json::from_str(&health).expect("health JSON");
        assert_eq!(value["op"], "health");
        reader
            .get_mut()
            .write_all(b"{\"status\":\"ok\",\"model\":\"coreml\",\"warm\":true}\n")
            .expect("write health");

        let mut predict = String::new();
        reader.read_line(&mut predict).expect("read prediction");
        let value: serde_json::Value = serde_json::from_str(&predict).expect("prediction JSON");
        assert!(value["questions"]["pr::applies"].is_object());
        assert_eq!(value["questions"]["pr::urgency"]["criteria"][2], "urgent");
        reader
            .get_mut()
            .write_all(
                b"{\"model\":\"coreml\",\"answers\":{\"pr::applies\":{\"type\":\"noul\",\"noul\":0.9},\"pr::urgency\":{\"type\":\"score\",\"score\":1.8}},\"usage\":{\"input_tokens\":10,\"output_tokens\":0}}\n",
            )
            .expect("write prediction");
    }
}
