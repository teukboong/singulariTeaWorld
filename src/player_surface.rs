const PLAYER_SURFACE_FORBIDDEN_TERMS: &[&str] = &[
    "slot",
    "선택지",
    "판정",
    "처리했다",
    "delayed",
    "partial_success",
    "costly_success",
    "visible",
    "evidence",
    "surface",
    "audit",
    "contract",
    "메타",
    "플레이어",
    "턴",
    "압력",
    "가늠",
];

const ABSTRACT_DELTA_TERMS: &[&str] = &[
    "기척",
    "압력",
    "결",
    "흐름",
    "감각",
    "정적",
    "울림",
    "흔들림",
    "어둠",
    "초점",
];

const CONCRETE_DELTA_TERMS: &[&str] = &[
    "문",
    "북문",
    "성문",
    "빗장",
    "창",
    "창끝",
    "문지기",
    "순찰",
    "사람",
    "목소리",
    "길",
    "골목",
    "방",
    "계단",
    "손",
    "발",
    "피",
    "열쇠",
    "종이",
    "기록지",
    "표식",
    "봉인",
    "끈",
    "쇠",
    "등불",
    "벽",
    "문턱",
    "바닥",
];

#[must_use]
pub fn player_surface_forbidden_terms(text: &str) -> Vec<&'static str> {
    let lowered = text.to_lowercase();
    PLAYER_SURFACE_FORBIDDEN_TERMS
        .iter()
        .copied()
        .filter(|needle| lowered.contains(&needle.to_lowercase()))
        .collect()
}

#[must_use]
pub fn is_player_surface_safe(text: &str) -> bool {
    player_surface_forbidden_terms(text).is_empty()
}

#[must_use]
pub fn concrete_delta_is_specific(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.chars().count() < 8 {
        return false;
    }
    if !is_player_surface_safe(trimmed) {
        return false;
    }
    let has_concrete = CONCRETE_DELTA_TERMS
        .iter()
        .any(|needle| trimmed.contains(needle));
    let has_only_abstract = ABSTRACT_DELTA_TERMS
        .iter()
        .any(|needle| trimmed.contains(needle))
        && !has_concrete;
    has_concrete || !has_only_abstract
}

#[must_use]
pub fn concise_player_status_from_blocks(blocks: &[String]) -> String {
    blocks
        .iter()
        .rev()
        .find_map(|block| concise_player_status(block))
        .unwrap_or_else(|| "장면이 다음 선택 앞에서 멈췄다.".to_owned())
}

#[must_use]
pub fn concise_player_status(text: &str) -> Option<String> {
    let sanitized = sanitize_player_surface_line(text);
    if sanitized.is_empty() {
        return None;
    }
    let first_sentence = sanitized
        .split_terminator(['.', '!', '?', '。', '！', '？'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(sanitized.as_str());
    let mut result = first_sentence.chars().take(72).collect::<String>();
    if result.chars().count() < first_sentence.chars().count() {
        result.push('…');
    }
    if result.ends_with(['.', '!', '?', '다', '요', '음', '함', '됨', '임', '…']) {
        Some(result)
    } else {
        Some(format!("{result}."))
    }
}

#[must_use]
pub fn sanitize_player_surface_line(text: &str) -> String {
    let mut line = text.trim().to_owned();
    for forbidden in PLAYER_SURFACE_FORBIDDEN_TERMS {
        line = line.replace(forbidden, "");
        line = line.replace(&forbidden.to_uppercase(), "");
    }
    line.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_meta_surface_terms() {
        let terms = player_surface_forbidden_terms("slot 4의 판정을 delayed로 처리했다");
        assert!(terms.contains(&"slot"));
        assert!(terms.contains(&"판정"));
        assert!(terms.contains(&"delayed"));
    }

    #[test]
    fn concrete_delta_rejects_abstract_only_motion() {
        assert!(!concrete_delta_is_specific("기척과 압력이 조금 흔들렸다"));
        assert!(concrete_delta_is_specific(
            "문지기가 창끝을 낮추고 북문 옆으로 비켜섰다"
        ));
    }
}
