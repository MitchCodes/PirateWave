//! Deterministic, bounded automation scheduler. Host calls this off the audio thread.
use crate::{
    project::Project,
    workflows::{Condition, Operation, Rule, Step, Trigger},
};
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Origin {
    #[default]
    Ui,
    Keyboard,
    Controller,
    Obs,
    Automation,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Context {
    pub generation: u64,
    pub operation: u64,
    pub origin: Origin,
    pub chain: [u64; 8],
    pub depth: u8,
}
impl Context {
    pub fn child(self, rule: u64) -> Result<Self, String> {
        if self.depth >= 8 || self.chain[..self.depth as usize].contains(&rule) {
            return Err("Automation causal loop detected; automation disarmed".into());
        }
        let mut next = self;
        next.chain[next.depth as usize] = rule;
        next.depth += 1;
        next.origin = Origin::Automation;
        Ok(next)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventKind {
    Audio(bool),
    Scene(u64),
    Device(u64, bool),
    Completion(Operation, u64, bool),
    Manual(u64),
}
#[derive(Clone, Copy, Debug)]
pub struct Event {
    pub context: Context,
    pub kind: EventKind,
}
#[derive(Clone, Default)]
pub struct Facts {
    pub causes: BTreeMap<u64, Context>,
    pub activity: BTreeMap<u64, bool>,
    pub running: bool,
    pub scene: Option<u64>,
    pub peaks: BTreeMap<u64, f32>,
    pub devices: BTreeMap<u64, bool>,
    pub recording: BTreeSet<u64>,
}
#[derive(Clone, Debug)]
pub struct Ready {
    pub rule: u64,
    pub context: Context,
    pub step: Step,
}
#[derive(Clone, Debug, Default)]
pub struct RuleStatus {
    pub running: bool,
    pub skipped: u64,
    pub error: Option<String>,
    pub executions: u64,
}
struct State {
    status: RuleStatus,
    active: Option<bool>,
    since: Option<u64>,
    cause: Option<Context>,
    last: Option<u64>,
    timer: u64,
    fired: bool,
    invalid: bool,
}
struct Run {
    rule: usize,
    context: Context,
    next: usize,
    due: u64,
    waiting: Option<u64>,
}
#[derive(Default)]
pub struct Scheduler {
    pub armed: bool,
    pub error: Option<String>,
    pub generation: u64,
    registry: crate::effects::EffectRegistry,
    rules: Vec<Rule>,
    states: Vec<State>,
    runs: Vec<Run>,
    events: VecDeque<Event>,
}
impl Scheduler {
    pub fn arm(&mut self, rules: Vec<Rule>, generation: u64, now: u64, facts: &Facts) {
        self.disarm();
        self.rules = rules;
        self.generation = generation;
        self.error = None;
        self.armed = true;
        self.states = self
            .rules
            .iter()
            .map(|r| State {
                status: RuleStatus::default(),
                active: match r.trigger {
                    Trigger::Level {
                        node,
                        above,
                        threshold_db,
                        ..
                    } => Some((level(facts, node) >= threshold_db) == above),
                    Trigger::Silence {
                        node, threshold_db, ..
                    } => Some(level(facts, node) < threshold_db),
                    _ => None,
                },
                since: None,
                cause: None,
                last: None,
                timer: now,
                fired: false,
                invalid: false,
            })
            .collect();
    }
    pub fn resolve_targets(&mut self, project: &Project) {
        for (rule, state) in self.rules.iter().zip(&mut self.states) {
            if let Err(e) = rule.validate_targets(project, &self.registry) {
                state.status.error = Some(e);
                state.invalid = true;
            }
        }
    }
    pub fn disarm(&mut self) {
        self.armed = false;
        self.runs.clear();
        self.events.clear();
        for s in &mut self.states {
            s.status.running = false;
        }
    }
    pub fn fail(&mut self, error: String) {
        self.disarm();
        self.error = Some(error);
    }
    pub fn requires_levels(&self) -> bool {
        self.armed
            && self.rules.iter().filter(|r| r.enabled).any(|r| {
                matches!(r.trigger, Trigger::Level { .. } | Trigger::Silence { .. })
                    || r.conditions
                        .iter()
                        .any(|c| matches!(c, Condition::Level { .. } | Condition::Activity { .. }))
            })
    }
    pub fn statuses(&self) -> BTreeMap<u64, RuleStatus> {
        self.rules
            .iter()
            .zip(&self.states)
            .map(|(r, s)| (r.id, s.status.clone()))
            .collect()
    }
    pub fn push(&mut self, event: Event) -> Result<(), String> {
        if !self.armed || event.context.generation != self.generation {
            return Ok(());
        }
        if self.events.len() >= 256 {
            let error = "Automation event queue is full; automation disarmed".to_string();
            self.fail(error.clone());
            return Err(error);
        }
        self.events.push_back(event);
        Ok(())
    }
    fn fire(
        &mut self,
        index: usize,
        now: u64,
        context: Context,
        project: &Project,
        facts: &Facts,
    ) -> Result<(), String> {
        let rule = &self.rules[index];
        if !rule.enabled || self.states[index].invalid {
            return Ok(());
        }
        if let Err(error) = rule.validate_targets(project, &self.registry) {
            self.states[index].status.error = Some(error);
            self.states[index].invalid = true;
            return Ok(());
        }
        let context = context.child(rule.id)?;
        if self.states[index].status.running {
            self.states[index].status.skipped += 1;
            return Ok(());
        }
        if self.states[index]
            .last
            .is_some_and(|last| now.saturating_sub(last) < rule.cooldown_ms as u64)
        {
            return Ok(());
        }
        let checks = rule.conditions.iter().map(|c| condition(c, project, facts));
        let pass = if rule.match_all {
            checks.clone().all(|v| v)
        } else {
            rule.conditions.is_empty() || checks.clone().any(|v| v)
        };
        if !pass {
            return Ok(());
        }
        self.states[index].status.error = None;
        self.states[index].last = Some(now);
        self.states[index].status.running = true;
        self.states[index].status.executions += 1;
        self.runs.push(Run {
            rule: index,
            context,
            next: 0,
            due: now,
            waiting: None,
        });
        Ok(())
    }
    pub fn tick(&mut self, now: u64, project: &Project, facts: &Facts) -> Vec<Ready> {
        if !self.armed {
            return vec![];
        }
        let mut fired = Vec::new();
        while let Some(event) = self.events.pop_front() {
            for (i, r) in self.rules.iter().enumerate() {
                if matches_event(&r.trigger, event.kind)
                    || matches!((&r.trigger,event.kind),(Trigger::Manual,EventKind::Manual(id)) if r.id==id)
                {
                    fired.push((i, event.context));
                }
            }
        }
        for (i, rule) in self.rules.iter().enumerate().filter(|(_, r)| r.enabled) {
            let state = &mut self.states[i];
            match rule.trigger {
                Trigger::Timer {
                    interval_ms,
                    repeating,
                } => {
                    if (!state.fired || repeating) && now.saturating_sub(state.timer) >= interval_ms
                    {
                        state.timer = now;
                        state.fired = true;
                        fired.push((
                            i,
                            Context {
                                generation: self.generation,
                                operation: crate::effects::new_effect_id(),
                                ..Default::default()
                            },
                        ));
                    }
                }
                Trigger::Level {
                    node,
                    above,
                    threshold_db,
                    hysteresis_db,
                    hold_ms,
                } => {
                    let value = level(facts, node);
                    let active = state.active.unwrap_or(false);
                    let desired = if above {
                        value
                            >= if active {
                                threshold_db - hysteresis_db
                            } else {
                                threshold_db
                            }
                    } else {
                        value
                            <= if active {
                                threshold_db + hysteresis_db
                            } else {
                                threshold_db
                            }
                    };
                    if desired != active {
                        if state.since.is_none() {
                            state.cause = facts.causes.get(&node).copied();
                        }
                        let since = *state.since.get_or_insert(now);
                        if now.saturating_sub(since) >= hold_ms as u64 {
                            state.active = Some(desired);
                            state.since = None;
                            if desired {
                                fired.push((
                                    i,
                                    state.cause.unwrap_or(Context {
                                        generation: self.generation,
                                        operation: crate::effects::new_effect_id(),
                                        ..Default::default()
                                    }),
                                ));
                            }
                        }
                    } else {
                        state.since = None;
                    }
                }
                Trigger::Silence {
                    node,
                    threshold_db,
                    hold_ms,
                } => {
                    let desired = level(facts, node) < threshold_db;
                    if desired {
                        if state.since.is_none() {
                            state.cause = facts.causes.get(&node).copied();
                        }
                        let since = *state.since.get_or_insert(now);
                        if !state.active.unwrap_or(false)
                            && now.saturating_sub(since) >= hold_ms as u64
                        {
                            state.active = Some(true);
                            fired.push((
                                i,
                                state.cause.unwrap_or(Context {
                                    generation: self.generation,
                                    operation: crate::effects::new_effect_id(),
                                    ..Default::default()
                                }),
                            ));
                        }
                    } else {
                        state.active = Some(false);
                        state.since = None;
                    }
                }
                _ => {}
            }
        }
        if fired.len() > 256 {
            self.fail("Automation event fan-out exceeded its limit".into());
            return vec![];
        }
        for (i, context) in fired {
            if let Err(error) = self.fire(i, now, context, project, facts) {
                self.fail(error);
                return vec![];
            }
        }
        let mut ready = Vec::new();
        for run in &mut self.runs {
            if run.due > now || run.waiting.is_some() {
                continue;
            }
            let rule = &self.rules[run.rule];
            // One actionable command per sequence per control tick preserves
            // ordered host acknowledgments; a Wait never blocks this worker.
            if let Some(step) = rule.actions.get(run.next) {
                run.next += 1;
                if let Step::Wait { milliseconds } = step {
                    run.due = now.saturating_add(*milliseconds as u64);
                } else {
                    let mut context = run.context;
                    context.operation = crate::effects::new_effect_id();
                    run.waiting = Some(context.operation);
                    ready.push(Ready {
                        rule: rule.id,
                        context,
                        step: step.clone(),
                    });
                }
            }
            if run.next >= rule.actions.len() && run.waiting.is_none() && run.due <= now {
                self.states[run.rule].status.running = false;
            }
        }
        self.runs.retain(|r| self.states[r.rule].status.running);
        ready
    }
    pub fn acknowledge(&mut self, operation: u64, result: Result<(), String>) {
        if let Some(index) = self.runs.iter().position(|r| r.waiting == Some(operation)) {
            let rule = self.runs[index].rule;
            self.runs[index].waiting = None;
            if let Err(error) = result {
                self.states[rule].status.error = Some(error);
                self.states[rule].status.running = false;
                self.runs.remove(index);
            }
        }
    }
    pub fn action_failed(&mut self, id: u64, error: String) {
        if let Some(i) = self.rules.iter().position(|r| r.id == id) {
            self.states[i].status.error = Some(error);
            self.states[i].status.running = false;
            self.runs.retain(|r| r.rule != i);
        }
    }
}
fn level(facts: &Facts, id: u64) -> f32 {
    20. * facts
        .peaks
        .get(&id)
        .copied()
        .unwrap_or(0.)
        .max(1e-8)
        .log10()
}
fn condition(c: &Condition, p: &Project, f: &Facts) -> bool {
    match c {
        Condition::Activity { node, active } => f.activity.get(node).is_some_and(|v| v == active),
        Condition::NodeEnabled { node, enabled } => p
            .nodes
            .iter()
            .find(|n| n.id == *node)
            .is_some_and(|n| !n.effects_disabled == *enabled),
        Condition::Recorder { node, active } => p
            .nodes
            .iter()
            .find(|n| n.id == *node && matches!(n.kind, crate::project::NodeKind::Recording))
            .is_some_and(|n| (f.running && n.recording.enabled) == *active),
        Condition::Audio { running } => f.running == *running,
        Condition::Scene { id } => f.scene == Some(*id),
        Condition::NodeMute { node, value } => p
            .nodes
            .iter()
            .find(|n| n.id == *node)
            .is_some_and(|n| n.mute == *value),
        Condition::Effect {
            node,
            effect,
            enabled,
        } => p
            .nodes
            .iter()
            .find(|n| n.id == *node)
            .and_then(|n| n.effects.iter().find(|e| e.id == *effect))
            .is_some_and(|e| e.enabled == *enabled),
        Condition::Level { node, above, db } => {
            if *above {
                level(f, *node) >= *db
            } else {
                level(f, *node) <= *db
            }
        }
        Condition::Device { node, available } => {
            f.devices.get(node).is_some_and(|v| v == available)
        }
        Condition::Recording { session, active } => f.recording.contains(session) == *active,
    }
}
fn matches_event(t: &Trigger, e: EventKind) -> bool {
    match (t, e) {
        (Trigger::Manual, EventKind::Manual(_)) => false,
        (Trigger::Audio { running }, EventKind::Audio(value)) => *running == value,
        (Trigger::Scene { id }, EventKind::Scene(value)) => *id == value,
        (Trigger::Device { node, available }, EventKind::Device(id, value)) => {
            *node == id && *available == value
        }
        (
            Trigger::Completion {
                operation,
                node,
                success,
            },
            EventKind::Completion(op, id, ok),
        ) => *operation == op && node.is_none_or(|n| n == id) && *success == ok,
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn rule(trigger: Trigger) -> Rule {
        Rule {
            id: 1,
            name: "Rule".into(),
            enabled: true,
            trigger,
            match_all: true,
            conditions: vec![],
            actions: vec![
                Step::Notify {
                    text: "first".into(),
                },
                Step::Wait { milliseconds: 200 },
                Step::Notify {
                    text: "second".into(),
                },
            ],
            cooldown_ms: 1000,
            keybind: String::new(),
        }
    }
    #[test]
    fn sequence_waits_for_acknowledgment_and_delay_and_cancels_on_disarm() {
        let p = Project::default();
        let f = Facts::default();
        let mut s = Scheduler::default();
        s.arm(vec![rule(Trigger::Manual)], 7, 0, &f);
        s.push(Event {
            context: Context {
                generation: 7,
                ..Default::default()
            },
            kind: EventKind::Manual(1),
        })
        .unwrap();
        let first = s.tick(0, &p, &f);
        assert_eq!(first.len(), 1);
        assert!(s.tick(100, &p, &f).is_empty());
        s.acknowledge(first[0].context.operation, Ok(()));
        assert!(s.tick(100, &p, &f).is_empty());
        assert!(s.tick(299, &p, &f).is_empty());
        let second = s.tick(300, &p, &f);
        assert_eq!(second.len(), 1);
        s.disarm();
        s.acknowledge(second[0].context.operation, Ok(()));
        assert!(s.tick(1000, &p, &f).is_empty());
    }
    #[test]
    fn level_requires_crossing_hold_and_hysteresis_without_arming_trigger() {
        let p = Project::default();
        let mut f = Facts::default();
        f.peaks.insert(1, 1.);
        let mut s = Scheduler::default();
        s.arm(
            vec![rule(Trigger::Level {
                node: 1,
                above: true,
                threshold_db: -35.,
                hysteresis_db: 6.,
                hold_ms: 150,
            })],
            2,
            0,
            &f,
        );
        assert!(s.tick(200, &p, &f).is_empty());
        f.peaks.insert(1, 0.);
        s.tick(250, &p, &f);
        s.tick(400, &p, &f);
        f.peaks.insert(1, 0.1);
        assert!(s.tick(500, &p, &f).is_empty());
        assert!(s.tick(649, &p, &f).is_empty());
        assert_eq!(s.tick(650, &p, &f).len(), 1);
    }
    #[test]
    fn generation_and_queue_saturation_never_replay_events() {
        let p = Project::default();
        let f = Facts::default();
        let mut s = Scheduler::default();
        s.arm(vec![rule(Trigger::Manual)], 3, 0, &f);
        s.push(Event {
            context: Context {
                generation: 2,
                ..Default::default()
            },
            kind: EventKind::Manual(1),
        })
        .unwrap();
        assert!(s.tick(1, &p, &f).is_empty());
        let e = Event {
            context: Context {
                generation: 3,
                ..Default::default()
            },
            kind: EventKind::Manual(1),
        };
        for _ in 0..256 {
            s.push(e).unwrap();
        }
        assert!(s.push(e).is_err());
        assert!(!s.armed);
        assert!(s.tick(10, &p, &f).is_empty());
    }
    #[test]
    fn running_triggers_are_counted_and_downstream_self_trigger_disarms() {
        let p = Project::default();
        let f = Facts::default();
        let mut s = Scheduler::default();
        s.arm(vec![rule(Trigger::Manual)], 9, 0, &f);
        let event = Event {
            context: Context {
                generation: 9,
                ..Default::default()
            },
            kind: EventKind::Manual(1),
        };
        s.push(event).unwrap();
        let ready = s.tick(0, &p, &f);
        assert_eq!(ready.len(), 1);
        s.push(event).unwrap();
        assert!(s.tick(1, &p, &f).is_empty());
        assert_eq!(s.states[0].status.skipped, 1);
        s.push(Event {
            context: ready[0].context,
            ..event
        })
        .unwrap();
        assert!(s.tick(2, &p, &f).is_empty());
        assert!(!s.armed);
        assert!(s.error.as_ref().unwrap().contains("loop"));
        s.acknowledge(ready[0].context.operation, Ok(()));
        assert!(s.tick(10_000, &p, &f).is_empty());
    }
    #[test]
    fn conditions_support_all_any_and_operation_failures_stop_sequences() {
        let p = Project::default();
        let f = Facts::default();
        let mut r = rule(Trigger::Manual);
        r.conditions = vec![
            Condition::Audio { running: false },
            Condition::Scene { id: 99 },
        ];
        let event = Event {
            context: Context {
                generation: 4,
                ..Default::default()
            },
            kind: EventKind::Manual(1),
        };
        let mut s = Scheduler::default();
        s.arm(vec![r.clone()], 4, 0, &f);
        s.push(event).unwrap();
        assert!(s.tick(0, &p, &f).is_empty());
        // The scene condition must resolve before either matching mode can run.
        r.conditions[1] = Condition::Audio { running: true };
        r.match_all = false;
        s.arm(vec![r], 4, 0, &f);
        s.push(event).unwrap();
        let ready = s.tick(0, &p, &f);
        assert_eq!(ready.len(), 1);
        s.acknowledge(ready[0].context.operation, Err("disk unavailable".into()));
        assert!(s.tick(10_000, &p, &f).is_empty());
        assert!(!s.states[0].status.running);
        assert_eq!(
            s.states[0].status.error.as_deref(),
            Some("disk unavailable")
        );
    }
    #[test]
    fn causal_cycles_and_depth_are_rejected() {
        let c = Context::default().child(1).unwrap().child(2).unwrap();
        assert!(c.child(1).is_err());
        let mut c = Context::default();
        for id in 1..=8 {
            c = c.child(id).unwrap();
        }
        assert!(c.child(9).is_err());
    }
}
