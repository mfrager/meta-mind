//! The core state-machine primitives: definitions, instances, guards and actions.
//!
//! A [`StateMachine`] is a definition: states, transitions, orthogonal regions and
//! the history states whose predecessor is remembered. An [`SmInstance`] is one
//! running copy of a definition. Definitions are immutable once an instance
//! starts; a new state is reached only by dispatching an event.

use std::collections::{BTreeMap, HashSet};
use std::fmt;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{SmError, SmResult};

/// The identifier of a state within one machine.
///
/// States are compared and serialized by name. Concrete `mm-cme` enums map onto
/// these names (see `SM_INTEGRATION_GUIDE.md`) so a definition can be built,
/// snapshotted and compared without generic plumbing at the call site.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StateId(String);

impl StateId {
    /// Build a state name.
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    /// The name as a string slice.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<&str> for StateId {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for StateId {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl fmt::Display for StateId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// An event that can trigger transitions.
///
/// The payload is application data: guards and actions see the whole event, while
/// transition matching uses only [`SmEvent::name`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct SmEvent {
    /// The event name a transition is keyed on.
    pub name: String,
    /// Optional structured payload, visible to guards and actions.
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub payload: Value,
}

impl SmEvent {
    /// Build a payload-free event.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            payload: Value::Null,
        }
    }

    /// Attach a payload.
    #[must_use]
    pub fn with_payload(mut self, payload: Value) -> Self {
        self.payload = payload;
        self
    }

    /// The event name.
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl From<&str> for SmEvent {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for SmEvent {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

/// The result an action may return.
pub type ActionResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// What a guard sees.
pub struct GuardContext<'a> {
    /// The instance the transition belongs to.
    pub instance: &'a str,
    /// The machine definition name.
    pub machine: &'a str,
    /// The state being left.
    pub from: &'a StateId,
    /// The state being entered.
    pub to: &'a StateId,
    /// The event being dispatched.
    pub event: &'a SmEvent,
    /// The instance's application data.
    pub data: &'a Value,
}

/// A condition that must hold for a transition to be taken.
pub trait SmGuard: Send + Sync {
    /// Whether the transition may fire.
    fn allows(&self, ctx: &GuardContext<'_>) -> bool;
}

impl<F> SmGuard for F
where
    F: Fn(&GuardContext<'_>) -> bool + Send + Sync,
{
    fn allows(&self, ctx: &GuardContext<'_>) -> bool {
        self(ctx)
    }
}

struct GuardFn<F>(F);

impl<F> SmGuard for GuardFn<F>
where
    F: Fn(&GuardContext<'_>) -> bool + Send + Sync,
{
    fn allows(&self, ctx: &GuardContext<'_>) -> bool {
        (self.0)(ctx)
    }
}

/// What an action sees and may mutate.
pub struct ActionContext<'a> {
    /// The instance the transition belongs to.
    pub instance: &'a str,
    /// The machine definition name.
    pub machine: &'a str,
    /// The state being left.
    pub from: &'a StateId,
    /// The state being entered.
    pub to: &'a StateId,
    /// The event being dispatched.
    pub event: &'a SmEvent,
    /// The instance's application data.
    pub data: &'a mut Value,
}

/// Work performed as part of a transition, before the new state is entered.
pub trait SmAction: Send + Sync {
    /// Run the action.
    fn apply(&self, ctx: &mut ActionContext<'_>) -> ActionResult;
}

impl<F> SmAction for F
where
    F: Fn(&mut ActionContext<'_>) -> ActionResult + Send + Sync,
{
    fn apply(&self, ctx: &mut ActionContext<'_>) -> ActionResult {
        self(ctx)
    }
}

struct ActionFn<F>(F);

impl<F> SmAction for ActionFn<F>
where
    F: Fn(&mut ActionContext<'_>) -> ActionResult + Send + Sync,
{
    fn apply(&self, ctx: &mut ActionContext<'_>) -> ActionResult {
        (self.0)(ctx)
    }
}

/// A shareable guard.
pub type Guard = Arc<dyn SmGuard + Send + Sync>;

/// A shareable action.
pub type Action = Arc<dyn SmAction + Send + Sync>;

/// Where a transition goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Destination {
    /// A named state.
    State(StateId),
    /// The state this instance was in before it entered a history state.
    History,
}

/// One transition in a definition.
#[derive(Clone)]
pub struct Transition {
    from: Option<StateId>,
    event: String,
    to: Destination,
    guard: Option<Guard>,
    action: Option<Action>,
}

impl Transition {
    /// A transition out of one named state.
    pub fn new(from: impl Into<StateId>, event: impl Into<String>, to: impl Into<StateId>) -> Self {
        Self {
            from: Some(from.into()),
            event: event.into(),
            to: Destination::State(to.into()),
            guard: None,
            action: None,
        }
    }

    /// A transition out of any state. Checked after the state-specific transitions.
    pub fn any(event: impl Into<String>, to: impl Into<StateId>) -> Self {
        Self {
            from: None,
            event: event.into(),
            to: Destination::State(to.into()),
            guard: None,
            action: None,
        }
    }

    /// A transition back to the predecessor recorded by a history state.
    pub fn to_history(from: impl Into<StateId>, event: impl Into<String>) -> Self {
        Self {
            from: Some(from.into()),
            event: event.into(),
            to: Destination::History,
            guard: None,
            action: None,
        }
    }

    /// Require `guard` for this transition.
    #[must_use]
    pub fn guard<F>(mut self, guard: F) -> Self
    where
        F: Fn(&GuardContext<'_>) -> bool + Send + Sync + 'static,
    {
        self.guard = Some(Arc::new(GuardFn(guard)));
        self
    }

    /// Run `action` when this transition is taken.
    #[must_use]
    pub fn action<F>(mut self, action: F) -> Self
    where
        F: Fn(&mut ActionContext<'_>) -> ActionResult + Send + Sync + 'static,
    {
        self.action = Some(Arc::new(ActionFn(action)));
        self
    }

    /// The source state, or `None` when the transition matches any state.
    pub fn from(&self) -> Option<&StateId> {
        self.from.as_ref()
    }

    /// The event name.
    pub fn event(&self) -> &str {
        &self.event
    }

    /// The destination.
    pub fn destination(&self) -> &Destination {
        &self.to
    }
}

/// A machine definition.
#[derive(Clone)]
pub struct StateMachine {
    /// The definition name, unique among a parent's regions and children.
    pub name: String,
    /// The state a fresh instance starts in.
    pub initial: StateId,
    /// Every declared state.
    pub states: Vec<StateId>,
    /// Transitions, checked in declaration order.
    pub transitions: Vec<Transition>,
    /// Orthogonal regions entered with the machine and dispatched in parallel.
    pub regions: Vec<StateMachine>,
    /// States whose predecessor is remembered, for [`Destination::History`].
    pub history: Vec<StateId>,
}

impl StateMachine {
    /// Start a definition with its initial state.
    pub fn new(name: impl Into<String>, initial: impl Into<StateId>) -> Self {
        let initial = initial.into();
        Self {
            name: name.into(),
            initial: initial.clone(),
            states: vec![initial],
            transitions: Vec::new(),
            regions: Vec::new(),
            history: Vec::new(),
        }
    }

    /// Declare a state. Declaring the initial state again is a no-op.
    #[must_use]
    pub fn state(mut self, state: impl Into<StateId>) -> Self {
        let state = state.into();
        if !self.states.contains(&state) {
            self.states.push(state);
        }
        self
    }

    /// Add a transition.
    #[must_use]
    pub fn transition(mut self, transition: Transition) -> Self {
        self.transitions.push(transition);
        self
    }

    /// Add an orthogonal region.
    #[must_use]
    pub fn region(mut self, region: StateMachine) -> Self {
        self.regions.push(region);
        self
    }

    /// Mark a state as remembering its predecessor.
    #[must_use]
    pub fn history(mut self, state: impl Into<StateId>) -> Self {
        let state = state.into();
        if !self.history.contains(&state) {
            self.history.push(state);
        }
        self
    }

    pub(crate) fn declares(&self, state: &StateId) -> bool {
        self.states.contains(state)
    }

    /// Reject a definition that cannot run.
    ///
    /// Every transition endpoint, history state and initial state must be
    /// declared; names must be unique; regions are validated recursively.
    pub fn validate(&self) -> SmResult<()> {
        if self.name.trim().is_empty() {
            return Err(SmError::invalid(&self.name, "the machine has no name"));
        }
        if self.states.is_empty() {
            return Err(SmError::invalid(
                &self.name,
                "the machine declares no states",
            ));
        }
        let mut seen: HashSet<&StateId> = HashSet::new();
        for state in &self.states {
            if state.as_str().trim().is_empty() {
                return Err(SmError::invalid(&self.name, "a state has an empty name"));
            }
            if !seen.insert(state) {
                return Err(SmError::DuplicateState {
                    machine: self.name.clone(),
                    state: state.to_string(),
                });
            }
        }
        if !seen.contains(&self.initial) {
            return Err(SmError::undeclared(&self.name, self.initial.to_string()));
        }
        for transition in &self.transitions {
            if transition.event.trim().is_empty() {
                return Err(SmError::invalid(
                    &self.name,
                    "a transition has an empty event name",
                ));
            }
            if let Some(from) = transition.from() {
                if !seen.contains(from) {
                    return Err(SmError::undeclared(&self.name, from.to_string()));
                }
            }
            if let Destination::State(to) = &transition.to {
                if !seen.contains(to) {
                    return Err(SmError::undeclared(&self.name, to.to_string()));
                }
            }
        }
        for history in &self.history {
            if !seen.contains(history) {
                return Err(SmError::undeclared(&self.name, history.to_string()));
            }
        }
        let mut regions = HashSet::new();
        for region in &self.regions {
            if !regions.insert(region.name.as_str()) {
                return Err(SmError::DuplicateRegion {
                    machine: self.name.clone(),
                    region: region.name.clone(),
                });
            }
            region.validate()?;
        }
        Ok(())
    }
}

/// One taken transition, as reported by [`SmInstance::dispatch`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct TransitionRecord {
    /// The instance that moved.
    pub instance: String,
    /// The machine definition that moved it.
    pub machine: String,
    /// The state left.
    pub from: StateId,
    /// The state entered.
    pub to: StateId,
    /// The event name.
    pub event: String,
    /// How deep in the composition the instance sits (`0` is the dispatched root).
    pub depth: usize,
}

/// A running instance of one machine.
#[derive(Clone)]
pub struct SmInstance {
    pub(crate) id: String,
    pub(crate) parent: Option<String>,
    pub(crate) machine: StateMachine,
    pub(crate) state: StateId,
    /// The predecessor of the current history state, restored by
    /// [`Destination::History`].
    pub(crate) resume: Option<StateId>,
    pub(crate) data: Value,
    pub(crate) transitions: u64,
    pub(crate) regions: BTreeMap<String, SmInstance>,
    pub(crate) children: Vec<SmInstance>,
    pub(crate) span: ::tracing::Span,
}

impl SmInstance {
    /// Start an instance with null application data.
    pub fn start(machine: StateMachine) -> SmResult<Self> {
        Self::start_with_data(machine, Value::Null)
    }

    /// Start an instance with application data.
    pub fn start_with_data(machine: StateMachine, data: Value) -> SmResult<Self> {
        machine.validate()?;
        let id = ulid::Ulid::gen().to_string().to_ascii_lowercase();
        let span = crate::tracing::instance_span(&id, &machine.name);
        let _entered = span.enter();
        let state = machine.initial.clone();
        let mut instance = Self {
            id,
            parent: None,
            machine,
            state,
            resume: None,
            data,
            transitions: 0,
            regions: BTreeMap::new(),
            children: Vec::new(),
            span: span.clone(),
        };
        let definitions = instance.machine.regions.clone();
        for region in definitions {
            let mut started = Self::start(region.clone())?;
            started.parent = Some(instance.id.clone());
            instance.regions.insert(region.name.clone(), started);
        }
        Ok(instance)
    }

    /// The instance identifier (lowercase ULID).
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The parent instance's identifier, when this instance was spawned.
    pub fn parent_id(&self) -> Option<&str> {
        self.parent.as_deref()
    }

    /// The definition this instance runs.
    pub fn machine(&self) -> &StateMachine {
        &self.machine
    }

    /// The current state.
    pub fn state(&self) -> &StateId {
        &self.state
    }

    /// Whether the instance is in `state`.
    pub fn is_in(&self, state: &str) -> bool {
        self.state.as_str() == state
    }

    /// The application data.
    pub fn data(&self) -> &Value {
        &self.data
    }

    /// Mutable application data.
    pub fn data_mut(&mut self) -> &mut Value {
        &mut self.data
    }

    /// The predecessor recorded by the current history state, if any.
    pub fn resume_target(&self) -> Option<&StateId> {
        self.resume.as_ref()
    }

    /// How many transitions this instance has taken itself.
    pub fn transition_count(&self) -> u64 {
        self.transitions
    }

    /// The orthogonal regions, by name.
    pub fn regions(&self) -> &BTreeMap<String, SmInstance> {
        &self.regions
    }

    /// The spawned children.
    pub fn children(&self) -> &[SmInstance] {
        &self.children
    }

    /// Dispatch `event` to this instance, its orthogonal regions and its
    /// descendants, in that order.
    ///
    /// An event with no eligible transition (no match, or a guard that refuses)
    /// is ignored and produces no record; this keeps unrelated region events
    /// harmless.
    pub fn dispatch(&mut self, event: impl Into<SmEvent>) -> SmResult<Vec<TransitionRecord>> {
        let event = event.into();
        let mut records = Vec::new();
        self.dispatch_into(&event, 0, &mut records)?;
        Ok(records)
    }

    fn dispatch_into(
        &mut self,
        event: &SmEvent,
        depth: usize,
        records: &mut Vec<TransitionRecord>,
    ) -> SmResult<()> {
        let span = self.span.clone();
        let _entered = span.enter();
        if let Some(record) = self.transition(event, depth)? {
            records.push(record);
        }
        for region in self.regions.values_mut() {
            region.dispatch_into(event, depth + 1, records)?;
        }
        for child in &mut self.children {
            child.dispatch_into(event, depth + 1, records)?;
        }
        Ok(())
    }

    fn transition(&mut self, event: &SmEvent, depth: usize) -> SmResult<Option<TransitionRecord>> {
        let from = self.state.clone();
        let mut chosen = None;
        for (index, candidate) in self.machine.transitions.iter().enumerate() {
            if candidate.event != event.name {
                continue;
            }
            if let Some(source) = candidate.from() {
                if source != &from {
                    continue;
                }
            }
            let to = self.destination(candidate)?;
            let allowed = match &candidate.guard {
                Some(guard) => guard.allows(&GuardContext {
                    instance: &self.id,
                    machine: &self.machine.name,
                    from: &from,
                    to: &to,
                    event,
                    data: &self.data,
                }),
                None => true,
            };
            if allowed {
                chosen = Some(index);
                break;
            }
        }
        let Some(index) = chosen else {
            return Ok(None);
        };
        let transition = self.machine.transitions[index].clone();
        let to = self.destination(&transition)?;

        let span =
            crate::tracing::transition_span(&self.id, &self.machine.name, &from, &to, &event.name);
        let _entered = span.entered();

        if let Some(action) = &transition.action {
            let mut ctx = ActionContext {
                instance: &self.id,
                machine: &self.machine.name,
                from: &from,
                to: &to,
                event,
                data: &mut self.data,
            };
            action.apply(&mut ctx).map_err(|source| SmError::Action {
                machine: self.machine.name.clone(),
                from: from.to_string(),
                to: to.to_string(),
                event: event.name.clone(),
                source,
            })?;
        }

        if self.machine.history.contains(&to) {
            self.resume = Some(from.clone());
        } else if self.resume.as_ref() == Some(&to) {
            self.resume = None;
        }
        self.state = to.clone();
        self.transitions += 1;

        Ok(Some(TransitionRecord {
            instance: self.id.clone(),
            machine: self.machine.name.clone(),
            from,
            to,
            event: event.name.clone(),
            depth,
        }))
    }

    fn destination(&self, transition: &Transition) -> SmResult<StateId> {
        match &transition.to {
            Destination::State(state) => Ok(state.clone()),
            Destination::History => Ok(self
                .resume
                .clone()
                .unwrap_or_else(|| self.machine.initial.clone())),
        }
    }
}

impl fmt::Debug for SmInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SmInstance")
            .field("id", &self.id)
            .field("machine", &self.machine.name)
            .field("state", &self.state)
            .field("resume", &self.resume)
            .field("regions", &self.regions.keys().collect::<Vec<_>>())
            .field("children", &self.children.len())
            .finish_non_exhaustive()
    }
}
