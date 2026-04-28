# World Simulator Engine V2

Last updated: 2026-04-28

This design replaces trope-first generation with pressure-first simulation. The
engine must not become bland: every turn still needs friction, sensory density,
consequence, and a reason to continue. The change is that those forces come from
world state instead of defaulting to reincarnation, guide, system, hidden
heroine, or mastermind patterns.

## Core Contract

The engine may only treat user seed text and player-visible canon as facts.
Sparse seeds stay sparse.

Example:

```text
중세 남자주인공
```

This establishes only low-level facts:

```text
setting_hint: medieval
protagonist_gender_hint: male
unknowns: many
```

It does not establish modern reincarnation, possession, regression, system
windows, cheat powers, lost memories, a hidden guide, a destined heroine, or a
mastermind. Those may appear only when the seed or later player-visible canon
actually establishes them.

## Pressure Vectors

Fun comes from pressure, not from a fixed genre template. Each narrative turn
should activate one or more pressure vectors:

| Vector | Meaning |
| --- | --- |
| `survival` | body, hunger, thirst, cold, injury, exhaustion |
| `social` | trust, rank, reputation, consent, misunderstanding |
| `material` | money, tools, food, maps, documents, shelter |
| `threat` | pursuit, hostile force, monster, soldier, disaster |
| `mystery` | incomplete evidence, strange trace, missing context |
| `desire` | what someone wants, protects, fears, or refuses |
| `moral_cost` | promise, harm, dignity, witness, debt |
| `time_pressure` | nightfall, closing gates, approaching patrol, deadline |

At least one vector should visibly move each turn. Quiet scenes are allowed, but
they must still move sensory density, relationship tension, knowledge, location,
resources, or risk.

## Dramatic Focus

The old `anchor_character` storage field remains for compatibility, but V2
treats it as a dormant dramatic-focus slot, not as a guaranteed hidden person.
New worlds start with unresolved focus:

```text
dramatic_focus: none_yet
```

Focus can later emerge from player-visible canon as:

| Kind | Example |
| --- | --- |
| `character` | a named rival, ally, witness, patron |
| `place` | a gate, tower, shrine, port, mine |
| `object` | sealed document, broken sword, map |
| `faction` | guard, guild, church, family, caravan |
| `oath` | promise, debt, curse, law |
| `threat` | plague, pursuer, weather, siege |
| `question` | who paid, why the road is closed, what changed |

The engine must not write "anchor character", "hidden identity", "destined
guide", or equivalent focus text into player-facing output unless canon already
made it visible.

## Slot 7

Slot 7 remains useful and should not be removed. Its active label is
`판단 위임`.

It is a meta-GM judgment slot, not an in-world guide. It means:

```text
Pick the most narratively strong, law-respecting, dignity-preserving move from
the visible state. Do not reveal the details before selection.
```

The visible intent stays:

```text
맡긴다. 세부 내용은 선택 후 드러난다.
```

## Choice Generation

Choices must be generated from current affordances, not copied from a fixed
genre menu.

Affordance examples:

```text
move, inspect, talk, hide, fight, trade, rest, craft, remember, ask_for_help,
take_risk, wait, use_object, freeform
```

Slots 1, 2, 3, 5, and 6 must be scene-specific. Their labels are UI text; the
slot numbers carry the stable contract.

## Prompt Layers

All text backends should receive the same conceptual packet:

1. Hard laws: death, body, distance, time, knowledge, hidden-truth redaction.
2. Seed facts: only explicit seed facts and low-level parsed hints.
3. Current world state: place, body, resources, threats, time, recent visible
   canon.
4. Dramatic pressure: active pressure vectors, cadence target, visible stakes.
5. Output contract: `AgentTurnResponse`, narrative budget, scene-specific
   choices.

WebGPT may revive memory more aggressively than Codex App, but evidence tiers
must be explicit:

```text
CanonVisible > PlayerAction > EngineState > DerivedHypothesis > StyleHint
```

Derived hypotheses must never be promoted into facts without visible evidence.

## Cadence Rule

The engine should prevent boring neutrality with a cadence check:

```text
If nothing dangerous happens, something must become clearer.
If nothing becomes clearer, pressure must rise.
If pressure does not rise, a relationship, resource, body state, place state, or
time cost must change.
```

This keeps the simulator from drifting into generic logs while still avoiding
unrequested genre injection.

## Implementation Notes

- Keep compatibility fields, but change new-world default wording to dramatic
  focus rather than hidden anchor character.
- Replace public examples and tests that repeatedly use "modern reincarnation /
  gifted protagonist" with neutral simulator seeds.
- Stop deterministic entity updates from adding protagonist-anchor story tension
  every turn.
- Treat slots 1-5 as scene-specific presented choices, slot 6 as inline
  `자유서술`, and slot 7 as `판단 위임`; keep legacy `안내자의 선택` readable only
  for old worlds and tests.
- Prompt all text backends with the anti-trope rule and pressure-vector rule.
