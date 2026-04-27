use crate::models::FREEFORM_CHOICE_SLOT;
use serde::{Deserialize, Serialize};

pub const CHAT_ROUTE_SCHEMA_VERSION: &str = "singulari.chat_route.v1";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRouteOptions {
    pub message: String,
    #[serde(default)]
    pub world_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRoute {
    pub schema_version: String,
    pub route: String,
    pub command: String,
    pub explanation: String,
}

/// Classify a worldsim chat simulator message into the next local CLI command.
#[must_use]
pub fn route_chat_input(options: &ChatRouteOptions) -> ChatRoute {
    let message = options.message.trim();
    let world_arg = options
        .world_id
        .as_ref()
        .map_or(String::new(), |world_id| format!(" --world-id {world_id}"));
    let (route, command, explanation) = if is_inline_freeform_choice(message) {
        (
            "turn",
            format!(
                "singulari-world turn{world_arg} --input {} --render",
                shell_quote(message)
            ),
            "slot 7 carries an inline freeform action for plausibility adjudication",
        )
    } else if message
        .parse::<u8>()
        .is_ok_and(|slot| (1..=6).contains(&slot))
    {
        (
            "turn",
            format!(
                "singulari-world turn{world_arg} --input {} --render",
                shell_quote(message)
            ),
            "bare numeric choice advances the active world turn",
        )
    } else if is_start_request(message) {
        (
            "start",
            format!(
                "singulari-world start --seed-text {}",
                shell_quote(seed_text(message))
            ),
            "explicit simulator start request creates a new active world from the seed text",
        )
    } else if is_codex_request(message) {
        (
            "codex_view",
            format!(
                "singulari-world codex-view{world_arg} --section {}",
                codex_section(message)
            ),
            "Codex request opens the player-visible DB-backed Archive View",
        )
    } else if let Some(query) = search_query(message) {
        (
            "search",
            format!(
                "singulari-world search{world_arg} --query {}",
                shell_quote(query)
            ),
            "search request queries player-visible world memory",
        )
    } else if is_resume_request(message) {
        (
            "resume_pack",
            format!("singulari-world resume-pack{world_arg}"),
            "resume request prints the compact continuation packet",
        )
    } else if is_export_request(message) {
        (
            "export_world",
            format!("singulari-world export-world{world_arg} --output <bundle_dir>"),
            "export request needs an explicit local bundle destination",
        )
    } else if is_import_request(message) {
        (
            "import_world",
            "singulari-world import-world --bundle <bundle_dir> --activate".to_owned(),
            "import request needs an explicit local bundle path",
        )
    } else {
        (
            "turn",
            format!(
                "singulari-world turn{world_arg} --input {} --render",
                shell_quote(message)
            ),
            "unrecognized gameplay text is treated as a freeform action attempt",
        )
    };
    ChatRoute {
        schema_version: CHAT_ROUTE_SCHEMA_VERSION.to_owned(),
        route: route.to_owned(),
        command,
        explanation: explanation.to_owned(),
    }
}

#[must_use]
pub fn render_chat_route(route: &ChatRoute) -> String {
    [
        format!("route: {}", route.route),
        format!("command: {}", route.command),
        format!("why: {}", route.explanation),
    ]
    .join("\n")
}

fn is_start_request(message: &str) -> bool {
    (message.contains("시작") || message.contains("start"))
        && (message.contains("싱귤")
            || message.contains("월드 시뮬")
            || message.contains("world simulator"))
}

fn seed_text(message: &str) -> &str {
    message
        .split_once(':')
        .map_or(message, |(_, seed)| seed.trim())
}

fn is_codex_request(message: &str) -> bool {
    message.contains("기록")
        || message.eq_ignore_ascii_case("codex")
        || message.contains("연대기")
        || message.contains("연감")
        || message.contains("청사진")
        || message.contains("실시간 분석")
        || message.contains("관련 항목")
}

fn codex_section(message: &str) -> &'static str {
    if message.contains("연대기") {
        return "timeline";
    }
    if message.contains("연감") {
        return "almanac";
    }
    if message.contains("청사진") || message.contains("엔티티") {
        return "blueprint";
    }
    if message.contains("분석") {
        return "analysis";
    }
    if message.contains("추천") || message.contains("관련") {
        return "related";
    }
    "all"
}

fn search_query(message: &str) -> Option<&str> {
    let trimmed = message.trim();
    for prefix in ["검색 ", "찾아줘 ", "찾아봐 ", "search "] {
        if let Some(query) = trimmed.strip_prefix(prefix) {
            let query = query.trim();
            if !query.is_empty() {
                return Some(query);
            }
        }
    }
    None
}

fn is_inline_freeform_choice(message: &str) -> bool {
    let Some(slot_digit) = char::from_digit(u32::from(FREEFORM_CHOICE_SLOT), 10) else {
        return false;
    };
    let Some(after_slot) = message.trim().strip_prefix(slot_digit) else {
        return false;
    };
    if after_slot.is_empty() {
        return true;
    }
    after_slot.starts_with("번")
        || after_slot.starts_with(char::is_whitespace)
        || after_slot
            .chars()
            .next()
            .is_some_and(|ch| matches!(ch, '.' | ')' | ':' | '-' | '—'))
}

fn is_resume_request(message: &str) -> bool {
    message.contains("재개") || message.contains("resume") || message.contains("이어")
}

fn is_export_request(message: &str) -> bool {
    message.contains("export") || message.contains("내보내")
}

fn is_import_request(message: &str) -> bool {
    message.contains("import") || message.contains("가져와") || message.contains("불러와")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::{ChatRouteOptions, route_chat_input};

    #[test]
    fn numeric_message_routes_to_turn_render() {
        let route = route_chat_input(&ChatRouteOptions {
            message: "4".to_owned(),
            world_id: Some("stw".to_owned()),
        });
        assert_eq!(route.route, "turn");
        assert!(route.command.contains("--input '4' --render"));
        assert!(route.command.contains("--world-id stw"));
    }

    #[test]
    fn inline_freeform_slot_routes_to_turn_render() {
        let route = route_chat_input(&ChatRouteOptions {
            message: "7 세아에게 낮게 묻는다".to_owned(),
            world_id: Some("stw".to_owned()),
        });
        assert_eq!(route.route, "turn");
        assert!(
            route
                .command
                .contains("--input '7 세아에게 낮게 묻는다' --render")
        );
        assert!(route.explanation.contains("freeform action"));
    }

    #[test]
    fn codex_section_message_routes_to_section_view() {
        let route = route_chat_input(&ChatRouteOptions {
            message: "연대기 열어봐".to_owned(),
            world_id: None,
        });
        assert_eq!(route.route, "codex_view");
        assert!(route.command.contains("--section timeline"));
    }
}
