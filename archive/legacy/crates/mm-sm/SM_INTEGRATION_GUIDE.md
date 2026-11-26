# `mm-sm` Integration Guide

`mm-sm` is the state-machine substrate under the conversation engine. It owns the
primitives — states, transitions, guards, actions, orthogonal regions, spawned
children, history and snapshots — and nothing about conversations. `mm-cme`
(Phase 3) defines the Session, Turn and Chunk machines with it.

The crate is a workspace member and is consumed as a path dependency:

```toml
[dependencies]
mm-sm = { workspace = true }
```

## A machine in one page

```rust
use mm_sm::{SmInstance, StateMachine, Transition};

let machine = StateMachine::new("turn", "planning")
    .state("routing")
    .state("executing")
    .transition(Transition::new("planning", "plan_ready", "routing"))
    .transition(Transition::new("routing", "route_selected", "executing"));

let mut turn = SmInstance::start(machine)?;
let records = turn.dispatch("plan_ready")?;
assert!(turn.is_in("routing"));
assert_eq!(records[0].from.as_str(), "planning");
```

`dispatch` returns one [`TransitionRecord`] per instance that moved, in parent →
orthogonal regions → children order. An event with no eligible transition is
ignored (`Ok` with no record), so an event aimed at one region is harmless to the
others. A guard that refuses is also ignored, not an error.

## States and events

States are [`StateId`]s compared by name. Events are [`SmEvent`]s matched by name;
the optional payload is application data that guards and actions can read.

`mm-cme`'s typed enums map to these names, which keeps the definitions readable
and the snapshots stable:

```rust
impl TurnState {
    fn as_name(&self) -> &'static str {
        match self {
            TurnState::Planning => "planning",
            TurnState::Executing => "executing",
            // …
        }
    }
}

impl From<TurnState> for StateId {
    fn from(state: TurnState) -> Self {
        StateId::new(state.as_name())
    }
}
```

Transitions are checked in declaration order. Put state-specific transitions
before `Transition::any`, which matches any source state.

## Guards and actions

```rust
use serde_json::json;
use mm_sm::Transition;

let transition = Transition::new("routing", "route_selected", "executing")
    .guard(|ctx| ctx.event.payload["tier"].as_u64().unwrap_or(3) <= 2)
    .action(|ctx| {
        ctx.data["routed_at_tier"] = ctx.event.payload["tier"].clone();
        Ok(())
    });
```

A guard sees `&GuardContext` (instance id, machine, `from`, `to`, event and the
instance's `data`) and cannot mutate. An action sees `&mut ActionContext` and may
mutate `data`; it runs **before** the instance enters `to`. If an action returns
an error, dispatch fails with `SmError::Action` and the state is unchanged, but
any mutation the action already made to `data` is kept — actions that must be
atomic should compute first and assign last.

The engine is synchronous on purpose. Model calls, tool execution and timers
belong to the driver that owns the instance; when one finishes it dispatches the
resulting event. That keeps a turn's transitions deterministic and testable.

## Orthogonal regions

Regions are machines that run in parallel with their parent:

```rust
let machine = StateMachine::new("session", "active")
    .region(
        StateMachine::new("proactive", "idle")
            .state("evaluating")
            .transition(Transition::new("idle", "tick", "evaluating")),
    )
    .region(
        StateMachine::new("memory", "episodic")
            .state("compressing")
            .transition(Transition::new("episodic", "compress", "compressing")),
    );

let mut session = SmInstance::start(machine)?;
session.dispatch("tick")?; // proactive moves, memory ignores the event

assert_eq!(session.orthogonal_current("proactive").unwrap().as_str(), "evaluating");
assert_eq!(session.orthogonal_current("memory").unwrap().as_str(), "episodic");
```

Regions can be added to a running instance with `enter_region`, removed with
`remove_region`, and inspected with `orthogonal_states`.

## Hierarchical children

`spawn_child` starts a child machine, records the parent id on it, and returns a
handle. Children see every event the parent dispatches, after the parent and its
regions. `children`, `child`, `child_mut`, `descendants` and `remove_child`
inspect and detach them.

Children are stored inside the parent, so a child's guards and actions stay alive
as long as the parent. A driver that needs to own children separately can remove
them and drive them directly; `parent_id` still records the relationship.

## History

A state declared with `.history(..)` remembers the state it was entered from.
`Transition::to_history` returns there:

```rust
let machine = StateMachine::new("turn", "streaming")
    .state("interrupted")
    .history("interrupted")
    .transition(Transition::new("streaming", "barge_in", "interrupted"))
    .transition(Transition::to_history("interrupted", "resume"));

let mut turn = SmInstance::start(machine)?;
turn.dispatch("barge_in")?;
assert_eq!(turn.resume_target().unwrap().as_str(), "streaming");
turn.dispatch("resume")?;
assert!(turn.is_in("streaming"));
```

This is shallow history: the remembered predecessor is the state the instance
left, not a stack of nested regions. If a deeper history is needed later, it
belongs in a new pseudostate, not in this one.

## Snapshots

`snapshot()` captures state, the remembered predecessor, application data,
transition count, regions and children. It is `serde`-serializable:

```rust
let json = serde_json::to_string(&instance.snapshot())?;
let decoded: mm_sm::SmSnapshot = serde_json::from_str(&json)?;
let restored = SmInstance::restore(machine, &decoded, &child_catalog)?;
```

Definitions are code, not data, so restoring takes the definition again. Regions
come from that definition; spawned children come from `child_catalog` (a slice of
definitions looked up by machine name). A region declared without a snapshot
starts in its initial state; a snapshot naming an unknown region or child machine
is rejected rather than silently dropped.

## Tracing

Every instance opens a `sm_instance` span carrying `instance` and `machine`, and
every taken transition opens `sm_transition` carrying `instance`, `machine`,
`from`, `to` and `event` while its action runs. An OpenTelemetry subscriber sees
the composition as nested spans; without a subscriber the spans are no-ops. No
global subscriber is installed by this crate.

## Validation

`SmInstance::start` validates the definition: unique non-empty state names, the
initial state declared, every transition endpoint and history state declared,
non-empty event names, unique region names, and regions validated recursively. A
malformed machine fails before it can run.

## What Phase 3 builds on top

- `Session` is an instance whose regions are Turn, Proactive, Voice, Memory,
  Preferences, Plugins and Improvement.
- `Turn` is an instance (or region) with a `Chunk` child per planned chunk; the
  `SessionDriver` dispatches LLM, tool and timer events into it.
- `TurnState::Interrupted` uses the history state so barge-in resumes the stream
  it interrupted.
- `SessionDriver` owns the async loop; `mm-sm` only records the transitions.
