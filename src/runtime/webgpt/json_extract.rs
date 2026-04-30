pub(super) fn extract_json_object_text(raw: &str) -> Option<String> {
    extract_json_object_text_matching(raw, is_agent_turn_response_object)
}

pub(super) fn extract_json_object_text_for_schema(
    raw: &str,
    schema_version: &str,
    required_keys: &[&str],
) -> Option<String> {
    extract_json_object_text_matching(raw, |value| {
        is_object_with_schema_and_keys(value, schema_version, required_keys)
    })
}

fn extract_json_object_text_matching(
    raw: &str,
    accepts: impl Fn(&serde_json::Value) -> bool,
) -> Option<String> {
    let trimmed = raw.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok_and(|value| accepts(&value)) {
        return Some(trimmed.to_owned());
    }
    if let Some(fenced) = extract_fenced_json_text(trimmed, &accepts) {
        return Some(fenced);
    }
    extract_first_balanced_json_object(trimmed, accepts)
}

fn extract_fenced_json_text(
    raw: &str,
    accepts: &impl Fn(&serde_json::Value) -> bool,
) -> Option<String> {
    let fence_start = raw.find("```")?;
    let after_start = &raw[fence_start + 3..];
    let after_header = after_start
        .find('\n')
        .map_or(after_start, |index| &after_start[index + 1..]);
    let fence_end = after_header.find("```")?;
    let candidate = after_header[..fence_end].trim();
    if serde_json::from_str::<serde_json::Value>(candidate).is_ok_and(|value| accepts(&value)) {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn extract_first_balanced_json_object(
    raw: &str,
    accepts: impl Fn(&serde_json::Value) -> bool,
) -> Option<String> {
    let mut start = None;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    for (index, ch) in raw.char_indices() {
        if start.is_none() {
            if ch == '{' {
                start = Some(index);
                depth = 1;
            }
            continue;
        }
        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }
        match ch {
            '"' => in_string = true,
            '{' => depth += 1,
            '}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let candidate = raw[start?..=index].trim();
                    if serde_json::from_str::<serde_json::Value>(candidate)
                        .is_ok_and(|value| accepts(&value))
                    {
                        return Some(candidate.to_owned());
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    None
}

fn is_agent_turn_response_object(value: &serde_json::Value) -> bool {
    is_object_with_schema_and_keys(
        value,
        "singulari.agent_turn_response.v1",
        &["world_id", "turn_id", "visible_scene", "next_choices"],
    )
}

fn is_object_with_schema_and_keys(
    value: &serde_json::Value,
    schema_version: &str,
    required_keys: &[&str],
) -> bool {
    let Some(object) = value.as_object() else {
        return false;
    };
    object
        .get("schema_version")
        .and_then(serde_json::Value::as_str)
        == Some(schema_version)
        && required_keys.iter().all(|key| object.contains_key(*key))
}

#[cfg(test)]
mod tests {
    use super::extract_json_object_text;

    #[test]
    fn extracts_complete_agent_turn_response_from_prose() {
        let raw = r#"좋아.
{"schema_version":"singulari.agent_turn_response.v1","world_id":"stw","turn_id":"turn_0001","resolution_proposal":null,"visible_scene":{"schema_version":"singulari.narrative_scene.v1","text_blocks":["문장"],"tone_notes":[]},"next_choices":[]}
끝."#;

        let Some(extracted) = extract_json_object_text(raw) else {
            panic!("complete response should extract");
        };
        assert!(extracted.contains("\"visible_scene\""));
        assert!(extracted.contains("\"next_choices\""));
    }

    #[test]
    fn rejects_prematurely_closed_agent_response_prefix() {
        let raw = r#"{"schema_version":"singulari.agent_turn_response.v1","world_id":"stw","turn_id":"turn_0001","resolution_proposal":{"next_choice_plan":[]}},"visible_scene":{"schema_version":"singulari.narrative_scene.v1","text_blocks":["문장"],"tone_notes":[]},"next_choices":[]}"#;

        assert!(extract_json_object_text(raw).is_none());
    }
}
