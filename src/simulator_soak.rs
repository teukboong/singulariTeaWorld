use crate::agent_bridge::{
    AGENT_TURN_RESPONSE_SCHEMA_VERSION, AgentCommitTurnOptions, AgentSubmitTurnOptions,
    AgentTurnResponse, PendingAgentTurn, commit_agent_turn, enqueue_agent_turn,
};
use crate::models::{GUIDE_CHOICE_TAG, NARRATIVE_SCENE_SCHEMA_VERSION, NarrativeScene, TurnChoice};
use crate::prompt_context::{CompilePromptContextPacketOptions, compile_prompt_context_packet};
use crate::resolution::{
    ActionAmbiguity, ActionInputKind, ActionIntent, ChoicePlan, ChoicePlanKind, NarrativeBrief,
    PressureNoopReason, RESOLUTION_PROPOSAL_SCHEMA_VERSION, ResolutionOutcome,
    ResolutionOutcomeKind, ResolutionProposal,
};
use crate::store::{InitWorldOptions, init_world, resolve_store_paths, world_file_paths};
use crate::transfer::{ExportWorldOptions, ImportWorldOptions, export_world, import_world};
use crate::turn_commit::repair_turn_materializations;
use crate::validate::{ValidationStatus, validate_world};
use crate::vn::{BuildVnPacketOptions, VnPacket, build_vn_packet};
use crate::world_db::{repair_world_db, world_db_stats};
use anyhow::{Result, bail};
use std::path::Path;
use tempfile::tempdir;

fn seed_body(world_id: &str) -> String {
    format!(
        r#"
schema_version: singulari.world_seed.v1
world_id: {world_id}
title: "simulator soak"
premise:
  genre: "casual fantasy"
  protagonist: "traveler at a guarded old road"
"#
    )
}

fn scene_choices() -> Vec<TurnChoice> {
    vec![
        TurnChoice {
            slot: 1,
            tag: "길가 표식".to_owned(),
            intent: "눈앞의 오래된 길가 표식을 살핀다".to_owned(),
        },
        TurnChoice {
            slot: 2,
            tag: "몸 상태".to_owned(),
            intent: "지친 몸으로 지금 가능한 행동을 가늠한다".to_owned(),
        },
        TurnChoice {
            slot: 3,
            tag: "낮은 인사".to_owned(),
            intent: "가까운 사람에게 낮게 인사를 건넨다".to_owned(),
        },
        TurnChoice {
            slot: 4,
            tag: "가진 것".to_owned(),
            intent: "손에 든 물건과 기억나는 단서를 대조한다".to_owned(),
        },
        TurnChoice {
            slot: 5,
            tag: "흐름 읽기".to_owned(),
            intent: "길목의 긴장과 시간 압력을 살핀다".to_owned(),
        },
        TurnChoice {
            slot: 6,
            tag: "자유서술".to_owned(),
            intent: "6 뒤에 직접 행동, 말, 내면 독백을 서술한다".to_owned(),
        },
        TurnChoice {
            slot: 7,
            tag: GUIDE_CHOICE_TAG.to_owned(),
            intent: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
        },
    ]
}

fn fixture_resolution(store: &Path, pending: &PendingAgentTurn) -> Result<ResolutionProposal> {
    let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
        store_root: Some(store),
        pending,
        engine_session_kind: "simulator_soak_fixture",
    })?;
    let mut next_choice_plan = context
        .pre_turn_simulation
        .available_affordances
        .iter()
        .map(|affordance| ChoicePlan {
            slot: affordance.slot,
            plan_kind: ChoicePlanKind::OrdinaryAffordance,
            grounding_ref: affordance.affordance_id.clone(),
            label_seed: format!("slot {} soak action", affordance.slot),
            intent_seed: affordance.action_contract.clone(),
            evidence_refs: vec![affordance.affordance_id.clone()],
        })
        .collect::<Vec<_>>();
    if next_choice_plan.len() != 5 {
        bail!(
            "simulator soak expected five ordinary affordances, got {}",
            next_choice_plan.len()
        );
    }
    let pressure_refs = context
        .pre_turn_simulation
        .pressure_obligations
        .iter()
        .map(|obligation| obligation.pressure_id.clone())
        .collect::<Vec<_>>();
    next_choice_plan.push(ChoicePlan {
        slot: 6,
        plan_kind: ChoicePlanKind::Freeform,
        grounding_ref: "current_turn".to_owned(),
        label_seed: "자유서술".to_owned(),
        intent_seed: "직접 행동을 입력한다.".to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });
    next_choice_plan.push(ChoicePlan {
        slot: 7,
        plan_kind: ChoicePlanKind::DelegatedJudgment,
        grounding_ref: "current_turn".to_owned(),
        label_seed: "판단 위임".to_owned(),
        intent_seed: "맡긴다. 세부 내용은 선택 후 드러난다.".to_owned(),
        evidence_refs: vec!["current_turn".to_owned()],
    });

    Ok(ResolutionProposal {
        schema_version: RESOLUTION_PROPOSAL_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        interpreted_intent: ActionIntent {
            input_kind: match pending.selected_choice.as_ref().map(|choice| choice.slot) {
                Some(6) => ActionInputKind::Freeform,
                Some(7) => ActionInputKind::DelegatedJudgment,
                _ => ActionInputKind::PresentedChoice,
            },
            summary: "fixture response resolves the current casual simulation input".to_owned(),
            target_refs: Vec::new(),
            pressure_refs: pressure_refs.clone(),
            evidence_refs: vec!["current_turn".to_owned()],
            ambiguity: ActionAmbiguity::Clear,
        },
        outcome: ResolutionOutcome {
            kind: ResolutionOutcomeKind::Success,
            summary: "the scene advances without granting unsupported hidden knowledge".to_owned(),
            evidence_refs: {
                let mut refs = vec!["current_turn".to_owned()];
                refs.extend(pressure_refs.iter().cloned());
                refs
            },
        },
        gate_results: Vec::new(),
        proposed_effects: Vec::new(),
        process_ticks: Vec::new(),
        pressure_noop_reasons: pressure_refs
            .iter()
            .map(|pressure_ref| PressureNoopReason {
                pressure_ref: pressure_ref.clone(),
                reason: "soak fixture records no durable pressure movement for this turn"
                    .to_owned(),
                evidence_refs: vec![pressure_ref.clone()],
            })
            .collect(),
        narrative_brief: NarrativeBrief {
            visible_summary: "the scene advances within the compiled affordances".to_owned(),
            required_beats: Vec::new(),
            forbidden_visible_details: Vec::new(),
        },
        next_choice_plan,
    })
}

fn fixture_response(store: &Path, pending: &PendingAgentTurn) -> Result<AgentTurnResponse> {
    Ok(AgentTurnResponse {
        schema_version: AGENT_TURN_RESPONSE_SCHEMA_VERSION.to_owned(),
        world_id: pending.world_id.clone(),
        turn_id: pending.turn_id.clone(),
        resolution_proposal: Some(fixture_resolution(store, pending)?),
        scene_director_proposal: None,
        consequence_proposal: None,
        social_exchange_proposal: None,
        encounter_proposal: None,
        visible_scene: NarrativeScene {
            schema_version: NARRATIVE_SCENE_SCHEMA_VERSION.to_owned(),
            speaker: None,
            text_blocks: vec![
                "길목의 공기가 낮게 가라앉고, 선택한 행동의 여파가 장면 안에서 이어진다."
                    .to_owned(),
            ],
            tone_notes: Vec::new(),
        },
        adjudication: None,
        canon_event: None,
        entity_updates: Vec::new(),
        relationship_updates: Vec::new(),
        plot_thread_events: Vec::new(),
        scene_pressure_events: Vec::new(),
        world_lore_updates: Vec::new(),
        character_text_design_updates: Vec::new(),
        body_resource_events: Vec::new(),
        location_events: Vec::new(),
        extra_contacts: Vec::new(),
        hidden_state_delta: Vec::new(),
        needs_context: Vec::new(),
        next_choices: scene_choices(),
        actor_goal_events: Vec::new(),
        actor_move_events: Vec::new(),
    })
}

fn assert_player_projection_is_clean(packet: &VnPacket) {
    let visible = packet
        .scene
        .text_blocks
        .iter()
        .chain(packet.choices.iter().map(|choice| &choice.intent))
        .chain(packet.choices.iter().map(|choice| &choice.tag))
        .cloned()
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!visible.contains("hidden"));
    assert!(!visible.contains("adjudication"));
    assert!(!visible.contains("밀서"));
}

struct SoakScenario {
    id: &'static str,
    inputs: &'static [&'static str],
}

fn assert_scenario_final_repair_and_transfer(
    temp_root: &Path,
    store: &Path,
    world_id: &str,
) -> Result<()> {
    let packet_before = build_vn_packet(&BuildVnPacketOptions {
        store_root: Some(store.to_path_buf()),
        world_id: world_id.to_owned(),
        turn_id: None,
        scene_image_url: None,
    })?;
    let turn_repair = repair_turn_materializations(Some(store), world_id)?;
    assert_eq!(turn_repair.render_packets_repaired, 0);
    assert_eq!(turn_repair.commit_records_repaired, 0);

    let store_paths = resolve_store_paths(Some(store))?;
    let files = world_file_paths(&store_paths, world_id);
    let db_repair = repair_world_db(files.dir.as_path(), world_id)?;
    assert!(db_repair.rebuilt);
    let stats_after_first_repair = world_db_stats(files.dir.as_path(), world_id)?;
    let second_db_repair = repair_world_db(files.dir.as_path(), world_id)?;
    assert_eq!(second_db_repair.canon_events, db_repair.canon_events);
    assert_eq!(second_db_repair.snapshots, db_repair.snapshots);
    assert_eq!(second_db_repair.render_packets, db_repair.render_packets);
    let stats_after_second_repair = world_db_stats(files.dir.as_path(), world_id)?;
    assert_eq!(
        stats_after_second_repair.search_documents,
        stats_after_first_repair.search_documents
    );
    let packet_after = build_vn_packet(&BuildVnPacketOptions {
        store_root: Some(store.to_path_buf()),
        world_id: world_id.to_owned(),
        turn_id: None,
        scene_image_url: None,
    })?;
    assert_eq!(
        packet_after.scene.text_blocks,
        packet_before.scene.text_blocks
    );
    assert_eq!(packet_after.choices.len(), packet_before.choices.len());

    let bundle = temp_root.join(format!("{world_id}_bundle"));
    export_world(&ExportWorldOptions {
        store_root: Some(store.to_path_buf()),
        world_id: world_id.to_owned(),
        output: bundle.clone(),
    })?;
    let import_store = temp_root.join(format!("{world_id}_import_store"));
    let imported = import_world(&ImportWorldOptions {
        store_root: Some(import_store.clone()),
        bundle,
        activate: false,
    })?;
    assert_eq!(imported.world_id, world_id);
    let imported_validation = validate_world(Some(import_store.as_path()), world_id)?;
    assert_eq!(imported_validation.status, ValidationStatus::Passed);

    Ok(())
}

#[test]
fn simulator_soak_replays_fixture_scenarios_without_webgpt() -> Result<()> {
    let scenarios = [
        SoakScenario {
            id: "sparse_medieval_male",
            inputs: &["1", "6 길가의 흙을 손끝으로 문질러 본다"],
        },
        SoakScenario {
            id: "missing_resource_attempt",
            inputs: &["4", "6 없는 지도를 찾으려고 품을 뒤진다"],
        },
        SoakScenario {
            id: "social_permission_push",
            inputs: &["3", "6 길목 사람에게 지나가도 되는지 낮게 묻는다"],
        },
        SoakScenario {
            id: "time_pressure_wait",
            inputs: &["5", "6 잠시 기다리며 길목의 흐름을 살핀다"],
        },
        SoakScenario {
            id: "hidden_probe",
            inputs: &["2", "6 설명되지 않은 기척의 겉 신호만 살핀다"],
        },
        SoakScenario {
            id: "route_and_return",
            inputs: &["1", "6 다시 원래 길목으로 돌아온다"],
        },
    ];

    for scenario in scenarios {
        let temp = tempdir()?;
        let store = temp.path().join("store");
        let world_id = format!("stw_soak_{}", scenario.id);
        let seed_path = temp.path().join("seed.yaml");
        std::fs::write(&seed_path, seed_body(world_id.as_str()))?;
        init_world(&InitWorldOptions {
            seed_path,
            store_root: Some(store.clone()),
            session_id: None,
        })?;

        for input in scenario.inputs {
            let pending = enqueue_agent_turn(&AgentSubmitTurnOptions {
                store_root: Some(store.clone()),
                world_id: world_id.clone(),
                input: (*input).to_owned(),
                narrative_level: None,
            })?;
            let context = compile_prompt_context_packet(&CompilePromptContextPacketOptions {
                store_root: Some(store.as_path()),
                pending: &pending,
                engine_session_kind: "simulator_soak_fixture",
            })?;
            assert!(
                context
                    .pre_turn_simulation
                    .required_resolution_fields
                    .resolution_proposal_required
            );

            commit_agent_turn(&AgentCommitTurnOptions {
                store_root: Some(store.clone()),
                world_id: world_id.clone(),
                response: fixture_response(store.as_path(), &pending)?,
            })?;

            let validation = validate_world(Some(store.as_path()), world_id.as_str())?;
            assert_eq!(validation.status, ValidationStatus::Passed);
            let packet = build_vn_packet(&BuildVnPacketOptions {
                store_root: Some(store.clone()),
                world_id: world_id.clone(),
                turn_id: Some(pending.turn_id.clone()),
                scene_image_url: None,
            })?;
            assert_player_projection_is_clean(&packet);
        }

        assert_scenario_final_repair_and_transfer(temp.path(), store.as_path(), world_id.as_str())?;
    }

    Ok(())
}
