use serde_json::{Map, Value};

const KEYS: [&str; 3] = ["parts", "delivery", "label"];
const DELIVERIES: [&str; 3] = ["now", "next", "wake"];

/// The longest `label`, in characters, so it fits one row after
/// `Chauffeur · <rule>: `.
const MAX_LABEL: usize = 100;

/// Why a `message.append` payload is refused: it takes text `parts`, a
/// `delivery` of `now`, `next` or `wake`, and an optional one-line `label`.
pub(super) fn check_append(payload: &Value) -> Result<(), String> {
    let Some(fields) = payload.as_object() else {
        return Err("`message.append` needs an object".to_owned());
    };

    if let Some(key) = fields.keys().find(|key| !KEYS.contains(&key.as_str())) {
        return Err(format!(
            "`message.append` has `{key}`; it takes `parts`, `delivery` and `label`"
        ));
    }

    check_parts(fields)?;
    check_delivery(fields)?;
    fields.get("label").map_or(Ok(()), check_label)
}

fn check_parts(fields: &Map<String, Value>) -> Result<(), String> {
    let parts = fields
        .get("parts")
        .and_then(Value::as_array)
        .filter(|parts| !parts.is_empty())
        .ok_or_else(|| "`message.append` needs `parts`, at least one".to_owned())?;

    for (index, part) in parts.iter().enumerate() {
        let only_text = part.as_object().is_some_and(|part| part.len() == 1);
        let text = part.get("text").and_then(Value::as_str);

        if !only_text || text.is_none_or(str::is_empty) {
            return Err(format!(
                "part {index} of `message.append` needs a `text` and nothing else"
            ));
        }
    }

    Ok(())
}

fn check_delivery(fields: &Map<String, Value>) -> Result<(), String> {
    let delivery = fields.get("delivery").and_then(Value::as_str);

    if delivery.is_some_and(|delivery| DELIVERIES.contains(&delivery)) {
        return Ok(());
    }

    Err("`message.append` needs a `delivery` of `now`, `next` or `wake`".to_owned())
}

fn check_label(label: &Value) -> Result<(), String> {
    let fits = label.as_str().is_some_and(|label| {
        let length = label.chars().count();

        !label.trim().is_empty() && !label.contains(['\n', '\r']) && length <= MAX_LABEL
    });

    if fits {
        Ok(())
    } else {
        Err(format!(
            "`label` must be one line of 1 to {MAX_LABEL} characters"
        ))
    }
}
