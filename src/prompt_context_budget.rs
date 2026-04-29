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
const OMITTED_DEBUG_SECTIONS_PROMPT_LIMIT: usize = 16;

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
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptContextExclusionReason {
    DebugOnlySection,
    BroadSourceSection,
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
        excluded: compile_exclusions(source.omitted_debug_sections),
        compiler_policy: PromptContextBudgetPolicy::default(),
    })
}

fn compile_budget_buckets(
    source: PromptContextBudgetReportSource<'_>,
) -> Result<BTreeMap<String, PromptContextBudgetBucket>> {
    let mut budgets = BTreeMap::new();
    insert_budget(
        &mut budgets,
        "selected_memory_items",
        SELECTED_MEMORY_ITEMS_PROMPT_LIMIT,
        array_len(source.selected_memory_items, "selected_memory_items")?,
    )?;
    insert_budget(
        &mut budgets,
        "belief_nodes",
        BELIEF_NODES_PROMPT_LIMIT,
        source.belief_graph.protagonist_visible_beliefs.len(),
    )?;
    insert_budget(
        &mut budgets,
        "visible_world_processes",
        WORLD_PROCESSES_PROMPT_LIMIT,
        source.world_process_clock.visible_processes.len(),
    )?;
    insert_budget(
        &mut budgets,
        "hidden_world_processes",
        WORLD_PROCESSES_PROMPT_LIMIT,
        source.world_process_clock.adjudication_only_processes.len(),
    )?;
    insert_budget(
        &mut budgets,
        "affordance_slots",
        AFFORDANCE_SLOTS_PROMPT_LIMIT,
        source.affordance_graph.ordinary_choice_slots.len(),
    )?;
    insert_budget(
        &mut budgets,
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
        &mut budgets,
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
        &mut budgets,
        "omitted_debug_sections",
        OMITTED_DEBUG_SECTIONS_PROMPT_LIMIT,
        source.omitted_debug_sections.len(),
    )?;
    Ok(budgets)
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

fn compile_exclusions(omitted_debug_sections: &[String]) -> Vec<PromptContextBudgetExclusion> {
    omitted_debug_sections
        .iter()
        .map(|section| PromptContextBudgetExclusion {
            section: section.clone(),
            source_id: section.clone(),
            reason: exclusion_reason(section),
            revivable: revivable_debug_section(section),
        })
        .collect()
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
        PromptContextBudgetBucket { limit, used },
    );
    Ok(())
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
        let selected_memory_items = Box::leak(Box::new(selected_memory_items));
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

        PromptContextBudgetReportSource {
            world_id: "stw_budget",
            turn_id: "turn_0100",
            selected_memory_items,
            affordance_graph,
            belief_graph,
            world_process_clock,
            active_change_ledger,
            active_pattern_debt,
            omitted_debug_sections,
        }
    }
}
