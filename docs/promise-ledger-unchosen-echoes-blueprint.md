# Promise Ledger and Unchosen Echoes Blueprint

Last updated: `2026-04-30`

This document adapts `promise_ledger_unchosen_echoes_design.docx` into the
active `projects2/singulari-world` runtime contract.

## One Sentence

The world leaves promises with the player, and the player leaves silences in the
world. Singulari World records both so return hooks become memory and payoff,
not bait.

## Scope

Included:

- Promise Ledger: player-visible unresolved world obligations.
- Unchosen Echoes: meaningful choices the player saw and did not take.
- Hook Orchestrator: a bounded editor that decides what may return.
- Tea Recap: player-visible session aftertaste split into promise and silence.

Excluded:

- No second player-facing chat UI.
- No replacement for World Court, resolution, scene pressure, or relationship
  graph.
- No real-time FOMO or punishment for being away.
- No hidden-truth reveal through hook summaries, echo summaries, Codex View,
  search snippets, image prompts, or VN text.

## Current Runtime Fit

The active loop is:

```text
VN app -> pending turn -> host-worker/WebGPT -> validated commit
-> append-only store/projections/world.db -> VN/MCP/Codex surfaces
```

Promise/Echo belongs in this loop as a projection family:

```text
visible scene facts + scene pressure + relationships + offered choices
-> hook_ledger projection
-> PromptContext.visible_context.active_hook_ledger
-> WebGPT choice/prose/payoff hints
-> World Court guard
-> tea_recap in VN Codex surface
```

The projection is advisory and player-visible. It must not become hidden truth,
and it must not override the hard resolution/court path.

## Data Model

### Promise Ledger

`HookThread` records what the world now owes the player.

Core fields:

- `hook_id`
- `kind`
- `visible_promise`
- `anchor_refs`
- `evidence_refs`
- `opened_by_event`
- `payoff_contract`
- `return_rights`
- `fatigue_score`
- `status`

Lifecycle:

```text
opened -> progressing -> payoff_due -> paid_off -> archived
opened/progressing -> suppressed
```

MVP rule: a Promise cannot open without player-visible evidence and a payoff
contract.

### Unchosen Echoes

`OfferedChoiceSet` records choices that were actually shown. `UnchosenEcho`
records at most one meaningful road-not-taken from that set.

Core fields:

- `source_turn_id`
- `unchosen_choice_id`
- `visible_summary`
- `implied_meaning`
- `anchor_refs`
- `evidence_refs`
- `return_conditions`
- `possible_payoffs`
- `decay`
- `status`

MVP rule: slots `1..5` may create Echoes. Slot `6` freeform and slot `7`
delegated judgment do not create Echoes.

## Storage

Per world:

| File | Role | Write mode |
| --- | --- | --- |
| `hook_threads.json` | active Promise state | atomic write |
| `hook_events.jsonl` | Promise lifecycle journal | append |
| `offered_choice_sets.jsonl` | actually displayed choice sets | append |
| `unchosen_echoes.json` | active Echo state | atomic write |
| `session_receipt.json` | Tea Recap packet | atomic write |

## Hook Budget

Defaults:

- New Promise per turn: `0..1`
- New Echo per turn: max `1`
- Scene active hooks: center `1`, support `1`, long-anchor touch `1`
- Teases without progress: max `3`
- Payoff/progress target: at least one per session when available
- FOMO: no real-time punishment

## Hook Orchestrator

Input:

- active promises
- active unchosen echoes
- current encounter surface
- scene pressure
- relationship graph
- body/resource state
- active processes/consequences
- recent accepted events

Decision order:

1. Place `payoff_due` promises first.
2. Return at most one Echo whose condition matches the current contact.
3. Open new Promise only inside turn/scene budget.
4. Suppress or force payoff for high-fatigue hooks.
5. Emit `choice_biases` and `tea_recap` fragments.

Output:

- `HookPacket`
- `choice_biases`
- `session_receipt`
- hook/echo event journal entries

## Tea Recap

Player-facing wording is deliberately not a quest log.

Labels:

- `찻잔에 남은 향`: questions the world left behind.
- `찻잔에 남은 말`: silences the player left behind.
- `잔열`: living pressure for next contact.

Wording rules:

- Avoid `실패`, `놓침`, `벌점`, `나중에 문제가 된다`.
- Promise wording should say a question remains or waits for contact.
- Echo wording should say a silence or unasked question remains.
- Hidden truth must be rendered only as symptoms, rumors, or observable pressure.

## Safety Contract

Threat model:

- Hidden-truth leak: reject visible summaries containing hidden needles.
- Punitive omission: Echo defaults to `non_punitive=true` and cannot encode
  irreversible punishment.
- Hook spam: turn/scene budgets and fatigue suppress overuse.
- Hallucinated hook: every Promise/Echo needs evidence refs from visible state.
- Choice injection: offered choice text is data; returned refs must be allowlisted.

Fail-closed behavior:

- Missing evidence rejects the hook/echo.
- Missing payoff contract rejects a Promise.
- Echo from unshown choice is rejected.
- Same-turn Echo count above one is rejected.
- Hidden or punitive text is rejected before commit.

## MVP Implementation Steps

1. Add `hook_ledger.rs` with model, load, append, rebuild, and packet compile
   functions.
2. Initialize hook state files in world creation.
3. Add `OmissionProfile` to `ChoiceContract`.
4. Record `OfferedChoiceSet` and create at most one Echo when a player selects
   one of the visible slots.
5. Add `AgentTurnResponse.hook_events` for Promise lifecycle events.
6. Add `active_hook_ledger` to `AgentVisibleContext` and
   `PromptVisibleContext`.
7. Add `tea_recap` to `VnCodexSurface` and render it in the VN Codex panel.
8. Extend World Court checks for hook event evidence/payoff and visible leak
   guards.
9. Cover with targeted tests and then run the standard smoke.

## Acceptance Criteria

- A choice never shown to the player cannot create an Echo.
- Same turn creates at most one Echo.
- Slot `6` and slot `7` do not create Echoes.
- Echo visible text is non-punitive.
- Promise events require evidence refs and payoff contract.
- `active_hook_ledger` appears in prompt context.
- Tea Recap separates Promise and Echo text.
- Hidden truth does not appear in Promise/Echo visible summaries.
