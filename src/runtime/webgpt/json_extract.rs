pub(super) fn extract_json_object_text(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if serde_json::from_str::<serde_json::Value>(trimmed).is_ok_and(|value| value.is_object()) {
        return Some(trimmed.to_owned());
    }
    if let Some(fenced) = extract_fenced_json_text(trimmed) {
        return Some(fenced);
    }
    extract_first_balanced_json_object(trimmed)
}

fn extract_fenced_json_text(raw: &str) -> Option<String> {
    let fence_start = raw.find("```")?;
    let after_start = &raw[fence_start + 3..];
    let after_header = after_start
        .find('\n')
        .map_or(after_start, |index| &after_start[index + 1..]);
    let fence_end = after_header.find("```")?;
    let candidate = after_header[..fence_end].trim();
    if serde_json::from_str::<serde_json::Value>(candidate).is_ok_and(|value| value.is_object()) {
        Some(candidate.to_owned())
    } else {
        None
    }
}

fn extract_first_balanced_json_object(raw: &str) -> Option<String> {
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
                        .is_ok_and(|value| value.is_object())
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
