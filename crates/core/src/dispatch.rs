//! Protocol dispatch over the SDK runtime.

use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::protocol::{
    AcknowledgeParams, DaemonRequest, DaemonResponse, METHOD_HEALTH, METHOD_REMINDERS_ACKNOWLEDGE,
    METHOD_REMINDERS_READ, METHOD_SKILL_EVALUATE, METHOD_SKILLS_LIST, METHOD_STEER,
    SkillEvaluateParams, SkillEvaluateResult, SteerParams, SteerResult, TargetParams,
};
use crate::runtime::Runtime;

pub fn dispatch(runtime: &Runtime, request: DaemonRequest) -> DaemonResponse {
    let result = dispatch_method(runtime, &request.method, request.params);

    match result {
        Ok(value) => DaemonResponse {
            id: request.id,
            result: Some(value),
            error: None,
        },
        Err(error) => DaemonResponse {
            id: request.id,
            result: None,
            error: Some(error),
        },
    }
}

pub fn dispatch_method(runtime: &Runtime, method: &str, params: Value) -> Result<Value, String> {
    match method {
        METHOD_HEALTH => Ok(json!({ "ok": true })),
        METHOD_STEER => steer(runtime, parse(params)?),
        METHOD_REMINDERS_READ => reminders_read(runtime, parse(params)?),
        METHOD_REMINDERS_ACKNOWLEDGE => acknowledge(runtime, parse(params)?),
        METHOD_SKILL_EVALUATE => skill_evaluate(runtime, parse(params)?),
        METHOD_SKILLS_LIST => {
            serde_json::to_value(runtime.skill_ids()).map_err(|error| error.to_string())
        }
        _ => Err(format!("unknown method {method}")),
    }
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|error| format!("invalid params: {error}"))
}

fn steer(runtime: &Runtime, params: SteerParams) -> Result<Value, String> {
    let reminders = runtime.steer(&params.target, &params.context)?;

    serde_json::to_value(SteerResult { reminders }).map_err(|error| error.to_string())
}

fn reminders_read(runtime: &Runtime, params: TargetParams) -> Result<Value, String> {
    let reminders = runtime.reminders(&params.target)?;

    serde_json::to_value(reminders).map_err(|error| error.to_string())
}

fn acknowledge(runtime: &Runtime, params: AcknowledgeParams) -> Result<Value, String> {
    runtime.acknowledge(&params.target, &params.reminder_ids)?;

    Ok(json!({ "ok": true }))
}

fn skill_evaluate(runtime: &Runtime, params: SkillEvaluateParams) -> Result<Value, String> {
    let evaluation = runtime.evaluate_skill(&params.skill_id, &params.context)?;

    serde_json::to_value(SkillEvaluateResult { evaluation }).map_err(|error| error.to_string())
}
