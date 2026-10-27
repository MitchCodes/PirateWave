//! Bounded control worker; never called by the audio renderer.
use piratewave_core::{
    automation::{Event, Facts, Ready, RuleStatus, Scheduler},
    project::Project,
};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc, Arc,
    },
    thread,
    time::{Duration, Instant},
};
pub enum Command {
    Arm {
        project: Arc<Project>,
        generation: u64,
        facts: Facts,
    },
    Disarm,
    Reset,
    State {
        project: Arc<Project>,
        facts: Facts,
    },
    Event(Event),
    Acknowledge {
        operation: u64,
        result: Result<(), String>,
    },
}
pub struct ReadyAction {
    pub epoch: u64,
    pub action: Ready,
}
struct Envelope {
    epoch: u64,
    command: Command,
}
#[derive(Clone)]
pub struct History {
    pub at_ms: u64,
    pub rule: u64,
    pub context: piratewave_core::automation::Context,
    pub description: String,
}
pub struct Status {
    pub history: Vec<History>,
    pub epoch: u64,
    pub armed: bool,
    pub error: Option<String>,
    pub rules: BTreeMap<u64, RuleStatus>,
}
pub struct Worker {
    commands: mpsc::SyncSender<Envelope>,
    pub epoch: Arc<AtomicU64>,
    pub enabled: Arc<AtomicBool>,
    pub ready: mpsc::Receiver<ReadyAction>,
    pub statuses: mpsc::Receiver<Status>,
    pub levels: Arc<AtomicBool>,
    pub overflow: Arc<AtomicBool>,
}
impl Default for Worker {
    fn default() -> Self {
        let epoch = Arc::new(AtomicU64::new(0));
        let current_epoch = epoch.clone();
        let enabled = Arc::new(AtomicBool::new(false));
        let active = enabled.clone();
        let (commands, rx) = mpsc::sync_channel::<Envelope>(256);
        let (tx, ready) = mpsc::sync_channel(256);
        let (status_tx, statuses) = mpsc::sync_channel(8);
        let levels = Arc::new(AtomicBool::new(false));
        let measure = levels.clone();
        let overflow = Arc::new(AtomicBool::new(false));
        let saturated = overflow.clone();
        thread::Builder::new()
            .name("piratewave-automation".into())
            .spawn(move || {
                let mut scheduler = Scheduler::default();
                let mut worker_epoch = 0;
                let mut history: VecDeque<History> = VecDeque::new();
                let mut pending = HashMap::new();
                let mut project = Arc::new(Project::default());
                let mut facts = Facts::default();
                let start = Instant::now();
                let mut published = Instant::now();
                loop {
                    let command = match rx.recv_timeout(Duration::from_millis(10)) {
                        Ok(c) => Some(c),
                        Err(mpsc::RecvTimeoutError::Timeout) => None,
                        Err(_) => break,
                    };
                    let now = start.elapsed().as_millis() as u64;
                    if let Some(envelope) = command {
                        if envelope.epoch != current_epoch.load(Ordering::Acquire) {
                            continue;
                        }
                        worker_epoch = envelope.epoch;
                        match envelope.command {
                            Command::Arm {
                                project: p,
                                generation,
                                facts: f,
                            } => {
                                project = p;
                                facts = f;
                                scheduler.arm(
                                    project.workflows.rules.clone(),
                                    generation,
                                    now,
                                    &facts,
                                );
                                scheduler.resolve_targets(&project);
                            }
                            Command::Disarm => {
                                scheduler.disarm();
                                pending.clear();
                            }
                            Command::Reset => {
                                scheduler = Scheduler::default();
                                pending.clear();
                                history.clear();
                            }
                            Command::State {
                                project: p,
                                facts: f,
                            } => {
                                project = p;
                                facts = f;
                            }
                            Command::Event(event) => {
                                let _ = scheduler.push(event);
                            }
                            Command::Acknowledge { operation, result } => {
                                if let Some((rule, context)) = pending.remove(&operation) {
                                    history.push_back(History {
                                        at_ms: now,
                                        rule,
                                        context,
                                        description: result.as_ref().map_or_else(
                                            |e| format!("Failed: {e}"),
                                            |_| "Step completed".into(),
                                        ),
                                    });
                                }
                                scheduler.acknowledge(operation, result)
                            }
                        }
                    }
                    if saturated.swap(false, Ordering::AcqRel) {
                        scheduler.fail("Automation control queue saturated; disarmed".into());
                    }
                    for action in scheduler.tick(now, &project, &facts) {
                        pending.insert(action.context.operation, (action.rule, action.context));
                        history.push_back(History {
                            at_ms: now,
                            rule: action.rule,
                            context: action.context,
                            description: format!(
                                "Executing {}",
                                match &action.step {
                                    piratewave_core::workflows::Step::Scene { .. } =>
                                        "scene recall",
                                    piratewave_core::workflows::Step::Session { .. } =>
                                        "session control",
                                    piratewave_core::workflows::Step::Set { .. } =>
                                        "parameter change",
                                    piratewave_core::workflows::Step::Notify { .. } =>
                                        "notification",
                                    _ => "action",
                                }
                            ),
                        });
                        if tx
                            .try_send(ReadyAction {
                                epoch: worker_epoch,
                                action,
                            })
                            .is_err()
                        {
                            scheduler.fail("Automation action queue saturated; disarmed".into());
                            break;
                        }
                    }
                    while history.len() > 128 {
                        history.pop_front();
                    }
                    active.store(
                        scheduler.armed && worker_epoch == current_epoch.load(Ordering::Acquire),
                        Ordering::Release,
                    );
                    measure.store(scheduler.requires_levels(), Ordering::Release);
                    if published.elapsed() >= Duration::from_millis(50) {
                        let _ = status_tx.try_send(Status {
                            history: history.iter().cloned().collect(),
                            epoch: worker_epoch,
                            armed: scheduler.armed,
                            error: scheduler.error.clone(),
                            rules: scheduler.statuses(),
                        });
                        published = Instant::now();
                    }
                }
            })
            .expect("Automation worker");
        Self {
            commands,
            epoch,
            enabled,
            ready,
            statuses,
            levels,
            overflow,
        }
    }
}
impl Worker {
    pub fn send(&self, command: Command) -> Result<(), String> {
        if matches!(
            command,
            Command::Arm { .. } | Command::Disarm | Command::Reset
        ) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
            self.enabled.store(false, Ordering::Release);
        }
        let epoch = self.epoch.load(Ordering::Acquire);
        self.commands
            .try_send(Envelope { epoch, command })
            .map_err(|_| {
                self.enabled.store(false, Ordering::Release);
                self.overflow.store(true, Ordering::Release);
                "Automation control queue is full; disarming".into()
            })
    }
}
