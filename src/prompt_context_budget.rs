use crate::affordance_graph::AffordanceGraphPacket;
use crate::belief_graph::BeliefGraphPacket;
use crate::world_process_clock::WorldProcessClockPacket;
use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION: &str =
    "singulari.prompt_context_budget_report.v1";

const SELECTED_MEMORY_ITEMS_PROMPT_LIMIT: usize = 36;
const BELIEF_NODES_PROMPT_LIMIT: usize = 48;
const WORLD_PROCESSES_PROMPT_LIMIT: usize = 8;
const AFFORDANCE_SLOTS_PROMPT_LIMIT: usize = 5;
const ACTIVE_CHANGES_PROMPT_LIMIT: usize = 12;
const ACTIVE_PATTERN_DEBT_PROMPT_LIMIT: usize = 8;
const SELECTED_CONTEXT_CAPSULE_PROMPT_LIMIT: usize = 8;
const OMITTED_DEBUG_SECTIONS_PROMPT_LIMIT: usize = 16;
const ACTIVE_SCENE_PRESSURE_PROMPT_LIMIT: usize = 5;
const ACTIVE_SCENE_PRESSURE_BYTE_LIMIT: usize = 12_000;
const ACTIVE_PLOT_THREADS_PROMPT_LIMIT: usize = 5;
const ACTIVE_PLOT_THREADS_BYTE_LIMIT: usize = 8_000;
const NARRATIVE_STYLE_STATE_BYTE_LIMIT: usize = 9_000;
const ACTIVE_CHARACTER_TEXT_DESIGN_BYTE_LIMIT: usize = 6_000;
const PRESSURE_OBLIGATIONS_BYTE_LIMIT: usize = 8_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextBudgetReport {
    pub schema_version: String,
    pub world_id: String,
    pub turn_id: String,
    pub budgets: BTreeMap<String, PromptContextBudgetBucket>,
    #[serde(default)]
    pub included: Vec<PromptContextBudgetInclusion>,
    #[serde(default)]
    pub excluded: Vec<PromptContextBudgetExclusion>,
    pub compiler_policy: PromptContextBudgetPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextBudgetBucket {
    pub limit: usize,
    pub used: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byte_limit: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_used: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextBudgetInclusion {
    pub section: String,
    pub source_id: String,
    pub reason: PromptContextInclusionReason,
    pub mandatory: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextBudgetExclusion {
    pub section: String,
    pub source_id: String,
    pub reason: PromptContextExclusionReason,
    pub revivable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptContextInclusionReason {
    CurrentTurn,
    OutputContract,
    VisibleState,
    SelectedMemoryRevival,
    DerivedVisibleConstraint,
    AdjudicationBoundary,
    SourceOfTruthPolicy,
    SelectedContextCapsule,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptContextExclusionReason {
    DebugOnlySection,
    BroadSourceSection,
    ContextCapsuleRejected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PromptContextBudgetPolicy {
    pub source: String,
    #[serde(default)]
    pub use_rules: Vec<String>,
}

impl Default for PromptContextBudgetPolicy {
    fn default() -> Self {
        Self {
            source: "compiled_from_prompt_context_selection_v1".to_owned(),
            use_rules: vec![
                "Every included prompt section must have a concrete reason and source id."
                    .to_owned(),
                "Every omitted broad/debug section must stay omitted unless a later compiler selects a smaller item from it.".to_owned(),
                "Budget overflow is an assembly error, never silent truncation.".to_owned(),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PromptContextBudgetReportSource<'a> {
    pub world_id: &'a str,
    pub turn_id: &'a str,
    pub selected_memory_items: &'a Value,
    pub affordance_graph: &'a AffordanceGraphPacket,
    pub belief_graph: &'a BeliefGraphPacket,
    pub world_process_clock: &'a WorldProcessClockPacket,
    pub active_change_ledger: &'a Value,
    pub active_pattern_debt: &'a Value,
    pub selected_context_capsules: &'a Value,
    pub active_scene_pressure: &'a Value,
    pub active_plot_threads: &'a Value,
    pub narrative_style_state: &'a Value,
    pub active_character_text_design: &'a Value,
    pub pressure_obligations: &'a Value,
    pub omitted_debug_sections: &'a [String],
}

/// Compile the prompt-context audit report from the already selected packet sections.
///
/// # Errors
///
/// Returns an error if a selected section exceeds its prompt budget, if selected memory
/// items are not an array, or if a selected memory item is missing its required source id.
pub fn compile_prompt_context_budget_report(
    source: PromptContextBudgetReportSource<'_>,
) -> Result<PromptContextBudgetReport> {
    Ok(PromptContextBudgetReport {
        schema_version: PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION.to_owned(),
        world_id: source.world_id.to_owned(),
        turn_id: source.turn_id.to_owned(),
        budgets: compile_budget_buckets(source)?,
        included: compile_inclusions(source)?,
        excluded: compile_exclusions(source)?,
        compiler_policy: PromptContextBudgetPolicy::default(),
    })
}

fn compile_budget_buckets(
    source: PromptContextBudgetReportSource<'_>,
) -> Result<BTreeMap<String, PromptContextBudgetBucket>> {
    let mut budgets = BTreeMap::new();
    insert_core_budget_buckets(&mut budgets, source)?;
    insert_runtime_budget_buckets(&mut budgets, source)?;
    insert_budget(
        &mut budgets,
        "omitted_debug_sections",
        OMITTED_DEBUG_SECTIONS_PROMPT_LIMIT,
        source.omitted_debug_sections.len(),
    )?;
    Ok(budgets)
}

fn insert_core_budget_buckets(
    budgets: &mut BTreeMap<String, PromptContextBudgetBucket>,
    source: PromptContextBudgetReportSource<'_>,
) -> Result<()> {
    insert_budget(
        budgets,
        "selected_memory_items",
        SELECTED_MEMORY_ITEMS_PROMPT_LIMIT,
        array_len(source.selected_memory_items, "selected_memory_items")?,
    )?;
    insert_budget(
        budgets,
        "belief_nodes",
        BELIEF_NODES_PROMPT_LIMIT,
        source.belief_graph.protagonist_visible_beliefs.len(),
    )?;
    insert_budget(
        budgets,
        "visible_world_processes",
        WORLD_PROCESSES_PROMPT_LIMIT,
        source.world_process_clock.visible_processes.len(),
    )?;
    insert_budget(
        budgets,
        "hidden_world_processes",
        WORLD_PROCESSES_PROMPT_LIMIT,
        source.world_process_clock.adjudication_only_processes.len(),
    )?;
    insert_budget(
        budgets,
        "affordance_slots",
        AFFORDANCE_SLOTS_PROMPT_LIMIT,
        source.affordance_graph.ordinary_choice_slots.len(),
    )?;
    insert_budget(
        budgets,
        "active_changes",
        ACTIVE_CHANGES_PROMPT_LIMIT,
        array_len(
            source
                .active_change_ledger
                .pointer("/active_changes")
                .unwrap_or(source.active_change_ledger),
            "active_change_ledger.active_changes",
        )?,
    )?;
    insert_budget(
        budgets,
        "active_patterns",
        ACTIVE_PATTERN_DEBT_PROMPT_LIMIT,
        array_len(
            source
                .active_pattern_debt
                .pointer("/active_patterns")
                .unwrap_or(source.active_pattern_debt),
            "active_pattern_debt.active_patterns",
        )?,
    )?;
    insert_budget(
        budgets,
        "selected_context_capsules",
        SELECTED_CONTEXT_CAPSULE_PROMPT_LIMIT,
        array_len(
            source
                .selected_context_capsules
                .pointer("/selected_capsules")
                .unwrap_or(source.selected_context_capsules),
            "selected_context_capsules.selected_capsules",
        )?,
    )?;
    Ok(())
}

fn insert_runtime_budget_buckets(
    budgets: &mut BTreeMap<String, PromptContextBudgetBucket>,
    source: PromptContextBudgetReportSource<'_>,
) -> Result<()> {
    insert_count_and_byte_budget(
        budgets,
        "active_scene_pressure",
        ACTIVE_SCENE_PRESSURE_PROMPT_LIMIT,
        array_len(source.active_scene_pressure, "active_scene_pressure")?,
        ACTIVE_SCENE_PRESSURE_BYTE_LIMIT,
        serialized_len(source.active_scene_pressure)?,
    )?;
    insert_count_and_byte_budget(
        budgets,
        "active_plot_threads",
        ACTIVE_PLOT_THREADS_PROMPT_LIMIT,
        array_len(source.active_plot_threads, "active_plot_threads")?,
        ACTIVE_PLOT_THREADS_BYTE_LIMIT,
        serialized_len(source.active_plot_threads)?,
    )?;
    insert_byte_budget(
        budgets,
        "narrative_style_state",
        NARRATIVE_STYLE_STATE_BYTE_LIMIT,
        serialized_len(source.narrative_style_state)?,
    )?;
    insert_byte_budget(
        budgets,
        "active_character_text_design",
        ACTIVE_CHARACTER_TEXT_DESIGN_BYTE_LIMIT,
        serialized_len(source.active_character_text_design)?,
    )?;
    insert_byte_budget(
        budgets,
        "pressure_obligations",
        PRESSURE_OBLIGATIONS_BYTE_LIMIT,
        serialized_len(source.pressure_obligations)?,
    )?;
    Ok(())
}

fn compile_inclusions(
    source: PromptContextBudgetReportSource<'_>,
) -> Result<Vec<PromptContextBudgetInclusion>> {
    let mut included = vec![
        inclusion(
            "current_turn",
            "source_revival.current_turn",
            PromptContextInclusionReason::CurrentTurn,
            true,
        ),
        inclusion(
            "opening_randomizer",
            "source_revival.opening_randomizer",
            PromptContextInclusionReason::CurrentTurn,
            false,
        ),
        inclusion(
            "output_contract",
            "source_revival.output_contract",
            PromptContextInclusionReason::OutputContract,
            true,
        ),
        inclusion(
            "source_of_truth_policy",
            "source_revival.source_of_truth_policy",
            PromptContextInclusionReason::SourceOfTruthPolicy,
            true,
        ),
    ];
    append_derived_inclusions(&mut included, source);
    append_selected_memory_inclusions(&mut included, source.selected_memory_items)?;
    append_selected_context_capsule_inclusions(&mut included, source.selected_context_capsules)?;
    Ok(included)
}

fn append_derived_inclusions(
    included: &mut Vec<PromptContextBudgetInclusion>,
    source: PromptContextBudgetReportSource<'_>,
) {
    included.extend([
        inclusion(
            "visible_context.affordance_graph",
            &source.affordance_graph.schema_version,
            PromptContextInclusionReason::DerivedVisibleConstraint,
            true,
        ),
        inclusion(
            "visible_context.belief_graph",
            &source.belief_graph.schema_version,
            PromptContextInclusionReason::DerivedVisibleConstraint,
            true,
        ),
        inclusion(
            "visible_context.world_process_clock",
            &source.world_process_clock.schema_version,
            PromptContextInclusionReason::DerivedVisibleConstraint,
            true,
        ),
        inclusion(
            "visible_context.active_scene_pressure",
            "active_scene_pressure.visible_active",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
        inclusion(
            "visible_context.active_plot_threads",
            "active_plot_threads.active_visible",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
        inclusion(
            "visible_context.narrative_style_state",
            "active_narrative_style_state",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
        inclusion(
            "visible_context.active_character_text_design",
            "active_character_text_design",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
        inclusion(
            "pre_turn_simulation.pressure_obligations",
            "pre_turn_simulation.pressure_obligations",
            PromptContextInclusionReason::DerivedVisibleConstraint,
            true,
        ),
        inclusion(
            "adjudication_context.hidden_world_process_clock",
            &source.world_process_clock.schema_version,
            PromptContextInclusionReason::AdjudicationBoundary,
            true,
        ),
        inclusion(
            "visible_context.active_change_ledger",
            "source_revival.memory_revival.active_change_ledger",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
        inclusion(
            "visible_context.active_pattern_debt",
            "source_revival.memory_revival.active_pattern_debt",
            PromptContextInclusionReason::VisibleState,
            true,
        ),
    ]);
}

fn compile_exclusions(
    source: PromptContextBudgetReportSource<'_>,
) -> Result<Vec<PromptContextBudgetExclusion>> {
    let mut excluded = source
        .omitted_debug_sections
        .iter()
        .map(|section| PromptContextBudgetExclusion {
            section: section.clone(),
            source_id: section.clone(),
            reason: exclusion_reason(section),
            revivable: revivable_debug_section(section),
        })
        .collect();
    append_context_capsule_exclusions(&mut excluded, source.selected_context_capsules)?;
    Ok(excluded)
}

fn append_context_capsule_exclusions(
    excluded: &mut Vec<PromptContextBudgetExclusion>,
    selected_context_capsules: &Value,
) -> Result<()> {
    let Some(rejected) = selected_context_capsules.pointer("/rejected_capsules") else {
        return Ok(());
    };
    let Value::Array(items) = rejected else {
        bail!("prompt context selected_context_capsules.rejected_capsules must be an array");
    };
    for (index, item) in items.iter().enumerate() {
        let capsule_id = item
            .pointer("/capsule_id")
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "prompt context selected_context_capsules.rejected_capsules[{index}] missing capsule_id"
                )
            })?;
        excluded.push(PromptContextBudgetExclusion {
            section: "visible_context.selected_context_capsules".to_owned(),
            source_id: capsule_id.to_owned(),
            reason: PromptContextExclusionReason::ContextCapsuleRejected,
            revivable: true,
        });
    }
    Ok(())
}

fn insert_budget(
    budgets: &mut BTreeMap<String, PromptContextBudgetBucket>,
    section: &str,
    limit: usize,
    used: usize,
) -> Result<()> {
    if used > limit {
        bail!("prompt context budget overflow: section={section}, used={used}, limit={limit}");
    }
    budgets.insert(
        section.to_owned(),
        PromptContextBudgetBucket {
            limit,
            used,
            byte_limit: None,
            bytes_used: None,
        },
    );
    Ok(())
}

fn insert_count_and_byte_budget(
    budgets: &mut BTreeMap<String, PromptContextBudgetBucket>,
    section: &str,
    limit: usize,
    used: usize,
    byte_limit: usize,
    bytes_used: usize,
) -> Result<()> {
    if used > limit {
        bail!("prompt context budget overflow: section={section}, used={used}, limit={limit}");
    }
    if bytes_used > byte_limit {
        bail!(
            "prompt context byte budget overflow: section={section}, used={bytes_used}, limit={byte_limit}"
        );
    }
    budgets.insert(
        section.to_owned(),
        PromptContextBudgetBucket {
            limit,
            used,
            byte_limit: Some(byte_limit),
            bytes_used: Some(bytes_used),
        },
    );
    Ok(())
}

fn insert_byte_budget(
    budgets: &mut BTreeMap<String, PromptContextBudgetBucket>,
    section: &str,
    byte_limit: usize,
    bytes_used: usize,
) -> Result<()> {
    if bytes_used > byte_limit {
        bail!(
            "prompt context byte budget overflow: section={section}, used={bytes_used}, limit={byte_limit}"
        );
    }
    budgets.insert(
        section.to_owned(),
        PromptContextBudgetBucket {
            limit: byte_limit,
            used: bytes_used,
            byte_limit: Some(byte_limit),
            bytes_used: Some(bytes_used),
        },
    );
    Ok(())
}

fn serialized_len(value: &Value) -> Result<usize> {
    Ok(serde_json::to_vec(value)?.len())
}

fn array_len(value: &Value, label: &str) -> Result<usize> {
    let Value::Array(items) = value else {
        bail!("prompt context budget source is not an array: {label}");
    };
    Ok(items.len())
}

fn inclusion(
    section: &str,
    source_id: &str,
    reason: PromptContextInclusionReason,
    mandatory: bool,
) -> PromptContextBudgetInclusion {
    PromptContextBudgetInclusion {
        section: section.to_owned(),
        source_id: source_id.to_owned(),
        reason,
        mandatory,
    }
}

fn append_selected_memory_inclusions(
    included: &mut Vec<PromptContextBudgetInclusion>,
    selected_memory_items: &Value,
) -> Result<()> {
    let Value::Array(items) = selected_memory_items else {
        bail!("prompt context selected_memory_items must be an array");
    };
    for (index, item) in items.iter().enumerate() {
        let source_id = item
            .pointer("/source_id")
            .and_then(Value::as_str)
            .with_context(|| {
                format!("prompt context selected_memory_items[{index}] missing required source_id")
            })?;
        included.push(inclusion(
            "visible_context.selected_memory_items",
            source_id,
            PromptContextInclusionReason::SelectedMemoryRevival,
            false,
        ));
    }
    Ok(())
}

fn append_selected_context_capsule_inclusions(
    included: &mut Vec<PromptContextBudgetInclusion>,
    selected_context_capsules: &Value,
) -> Result<()> {
    let selected = selected_context_capsules
        .pointer("/selected_capsules")
        .unwrap_or(selected_context_capsules);
    let Value::Array(items) = selected else {
        bail!("prompt context selected_context_capsules.selected_capsules must be an array");
    };
    for (index, item) in items.iter().enumerate() {
        let capsule_id = item
            .pointer("/capsule_id")
            .and_then(Value::as_str)
            .with_context(|| {
                format!(
                    "prompt context selected_context_capsules.selected_capsules[{index}] missing capsule_id"
                )
            })?;
        included.push(inclusion(
            "visible_context.selected_context_capsules",
            capsule_id,
            PromptContextInclusionReason::SelectedContextCapsule,
            false,
        ));
    }
    Ok(())
}

fn exclusion_reason(section: &str) -> PromptContextExclusionReason {
    if section.contains("active_memory_revival") {
        PromptContextExclusionReason::BroadSourceSection
    } else {
        PromptContextExclusionReason::DebugOnlySection
    }
}

fn revivable_debug_section(section: &str) -> bool {
    section.contains("active_memory_revival")
        && !section.ends_with("agent_context_projection")
        && !section.ends_with("player_visible_archive_view")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::affordance_graph::{
        AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION, AffordanceGraphPacket, AffordanceGraphPolicy,
    };
    use crate::belief_graph::{
        BELIEF_GRAPH_PACKET_SCHEMA_VERSION, BeliefGraphPacket, BeliefGraphPolicy,
    };
    use crate::world_process_clock::{
        WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION, WorldProcessClockPacket, WorldProcessClockPolicy,
    };

    #[test]
    fn reports_included_selected_memory_and_excluded_debug_sections() -> anyhow::Result<()> {
        let report = compile_prompt_context_budget_report(sample_source(
            serde_json::json!([{"source_id": "rel:current"}]),
            vec!["source_revival.memory_revival.active_memory_revival.query_recall".to_owned()],
        ))?;

        assert_eq!(
            report.schema_version,
            PROMPT_CONTEXT_BUDGET_REPORT_SCHEMA_VERSION
        );
        assert_eq!(report.budgets["selected_memory_items"].used, 1);
        assert!(report.included.iter().any(|entry| {
            entry.section == "visible_context.selected_memory_items"
                && entry.source_id == "rel:current"
                && entry.reason == PromptContextInclusionReason::SelectedMemoryRevival
        }));
        assert!(report.excluded.iter().any(|entry| {
            entry.source_id.ends_with("query_recall")
                && entry.reason == PromptContextExclusionReason::BroadSourceSection
                && entry.revivable
        }));
        Ok(())
    }

    #[test]
    fn reports_prompt_growth_budgets_for_active_runtime_sections() -> anyhow::Result<()> {
        let report =
            compile_prompt_context_budget_report(sample_source(serde_json::json!([]), Vec::new()))?;

        assert_eq!(
            report.budgets["active_scene_pressure"].byte_limit,
            Some(ACTIVE_SCENE_PRESSURE_BYTE_LIMIT)
        );
        assert_eq!(
            report.budgets["active_plot_threads"].byte_limit,
            Some(ACTIVE_PLOT_THREADS_BYTE_LIMIT)
        );
        assert_eq!(
            report.budgets["narrative_style_state"].byte_limit,
            Some(NARRATIVE_STYLE_STATE_BYTE_LIMIT)
        );
        assert_eq!(
            report.budgets["active_character_text_design"].byte_limit,
            Some(ACTIVE_CHARACTER_TEXT_DESIGN_BYTE_LIMIT)
        );
        assert_eq!(
            report.budgets["pressure_obligations"].byte_limit,
            Some(PRESSURE_OBLIGATIONS_BYTE_LIMIT)
        );
        Ok(())
    }

    #[test]
    fn rejects_over_byte_budget_prompt_runtime_sections() {
        let oversized = Box::leak(Box::new(Value::String(
            "x".repeat(NARRATIVE_STYLE_STATE_BYTE_LIMIT + 1),
        )));
        let mut source = sample_source(serde_json::json!([]), Vec::new());
        source.narrative_style_state = oversized;

        let result = compile_prompt_context_budget_report(source);
        let Err(err) = result else {
            panic!("oversized narrative style state should fail prompt assembly");
        };

        assert!(err.to_string().contains("byte budget overflow"));
    }

    #[test]
    fn reports_selected_and_rejected_context_capsules() -> anyhow::Result<()> {
        let report = compile_prompt_context_budget_report(sample_source_with_capsules(
            serde_json::json!([]),
            serde_json::json!({
                "selected_capsules": [{"capsule_id": "world_lore:gate"}],
                "rejected_capsules": [{"capsule_id": "relationship:stale"}]
            }),
            Vec::new(),
        ))?;

        assert!(report.included.iter().any(|entry| {
            entry.section == "visible_context.selected_context_capsules"
                && entry.source_id == "world_lore:gate"
                && entry.reason == PromptContextInclusionReason::SelectedContextCapsule
        }));
        assert!(report.excluded.iter().any(|entry| {
            entry.source_id == "relationship:stale"
                && entry.reason == PromptContextExclusionReason::ContextCapsuleRejected
                && entry.revivable
        }));
        Ok(())
    }

    #[test]
    fn rejects_selected_memory_without_source_id() {
        let result = compile_prompt_context_budget_report(sample_source(
            serde_json::json!([{"payload": {"visible_summary": "missing id"}}]),
            Vec::new(),
        ));
        let Err(err) = result else {
            panic!("missing selected memory source_id should fail prompt assembly");
        };

        assert!(err.to_string().contains("missing required source_id"));
    }

    #[test]
    fn rejects_over_budget_selected_memory() {
        let items = (0..=SELECTED_MEMORY_ITEMS_PROMPT_LIMIT)
            .map(|index| serde_json::json!({"source_id": format!("mem:{index}")}))
            .collect::<Vec<_>>();

        let result =
            compile_prompt_context_budget_report(sample_source(Value::Array(items), Vec::new()));
        let Err(err) = result else {
            panic!("selected memory overflow should fail prompt assembly");
        };

        assert!(err.to_string().contains("budget overflow"));
    }

    fn sample_source(
        selected_memory_items: Value,
        omitted_debug_sections: Vec<String>,
    ) -> PromptContextBudgetReportSource<'static> {
        sample_source_with_capsules(
            selected_memory_items,
            serde_json::json!({"selected_capsules": []}),
            omitted_debug_sections,
        )
    }

    fn sample_source_with_capsules(
        selected_memory_items: Value,
        selected_context_capsules: Value,
        omitted_debug_sections: Vec<String>,
    ) -> PromptContextBudgetReportSource<'static> {
        let selected_memory_items = Box::leak(Box::new(selected_memory_items));
        let selected_context_capsules = Box::leak(Box::new(selected_context_capsules));
        let omitted_debug_sections = Box::leak(Box::new(omitted_debug_sections));
        let affordance_graph = Box::leak(Box::new(AffordanceGraphPacket {
            schema_version: AFFORDANCE_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_budget".to_owned(),
            turn_id: "turn_0100".to_owned(),
            ordinary_choice_slots: Vec::new(),
            compiler_policy: AffordanceGraphPolicy::default(),
        }));
        let belief_graph = Box::leak(Box::new(BeliefGraphPacket {
            schema_version: BELIEF_GRAPH_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_budget".to_owned(),
            turn_id: "turn_0100".to_owned(),
            protagonist_visible_beliefs: Vec::new(),
            narrator_knowledge_limits: Vec::new(),
            compiler_policy: BeliefGraphPolicy::default(),
        }));
        let world_process_clock = Box::leak(Box::new(WorldProcessClockPacket {
            schema_version: WORLD_PROCESS_CLOCK_PACKET_SCHEMA_VERSION.to_owned(),
            world_id: "stw_budget".to_owned(),
            turn_id: "turn_0100".to_owned(),
            visible_processes: Vec::new(),
            adjudication_only_processes: Vec::new(),
            compiler_policy: WorldProcessClockPolicy::default(),
        }));
        let active_change_ledger = Box::leak(Box::new(serde_json::json!({"active_changes": []})));
        let active_pattern_debt = Box::leak(Box::new(serde_json::json!({"active_patterns": []})));
        let active_scene_pressure = Box::leak(Box::new(serde_json::json!([])));
        let active_plot_threads = Box::leak(Box::new(serde_json::json!([])));
        let narrative_style_state = Box::leak(Box::new(serde_json::json!({})));
        let active_character_text_design = Box::leak(Box::new(serde_json::json!({})));
        let pressure_obligations = Box::leak(Box::new(serde_json::json!([])));
        PromptContextBudgetReportSource {
            world_id: "stw_budget",
            turn_id: "turn_0100",
            selected_memory_items,
            affordance_graph,
            belief_graph,
            world_process_clock,
            active_change_ledger,
            active_pattern_debt,
            selected_context_capsules,
            active_scene_pressure,
            active_plot_threads,
            narrative_style_state,
            active_character_text_design,
            pressure_obligations,
            omitted_debug_sections,
        }
    }
}
