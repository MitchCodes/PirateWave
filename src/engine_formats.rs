//! Selective format replacement: prepare off-thread and accept only a still-current target.
use crate::{audio_io, engine::Session};
use crossbeam_queue::ArrayQueue;
use piratewave_core::{audio_format::AudioFormat, project::NodeKind};
use std::{
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};

#[derive(Clone, PartialEq)]
pub(crate) struct Target {
    id: u64,
    kind: NodeKind,
    listen: bool,
    device: Option<String>,
    preference: AudioFormat,
}
impl Target {
    fn matches(&self, node: &piratewave_core::project::Node) -> bool {
        node.id == self.id
            && node.kind == self.kind
            && if self.listen {
                node.listen_enabled
                    && node.listen_device == self.device
                    && node.listen_format == self.preference
            } else {
                node.audio_format == self.preference
            }
    }
}
enum Prepared {
    Input(Box<audio_io::InputStream>),
    Output(audio_io::OutputStream),
}
#[derive(Default)]
pub(crate) struct PendingFormats {
    pending: Option<(Target, mpsc::Receiver<Result<Prepared, String>>)>,
    failed: Option<(Target, Instant)>,
    dirty: bool,
    scanned: Option<Instant>,
}
impl PendingFormats {
    pub(super) fn changed(
        &mut self,
        old: &piratewave_core::project::Project,
        new: &piratewave_core::project::Project,
    ) {
        if new.nodes.iter().any(|node| {
            old.nodes.iter().any(|before| {
                before.id == node.id
                    && (before.audio_format != node.audio_format
                        || before.listen_format != node.listen_format)
            })
        }) {
            self.dirty = true;
            self.scanned = None;
        }
    }
}
impl Session {
    pub(super) fn poll_format_changes(&mut self) {
        if !self.format_changes.dirty && self.format_changes.pending.is_none() {
            return;
        }
        if let Some((target, rx)) = &self.format_changes.pending {
            let result = match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("Device preparation stopped".into()))
                }
            };
            if let Some(result) = result {
                let target = target.clone();
                self.format_changes.pending = None;
                let current = self.project.nodes.iter().any(|n| target.matches(n));
                if current {
                    match result {
                        Ok(Prepared::Input(stream)) => {
                            if let Some((_, existing)) =
                                self.inputs.iter_mut().find(|(id, _)| *id == target.id)
                            {
                                *existing = *stream;
                            }
                            self.format_discontinuity = true;
                            self.voice_changer.source_discontinuity(target.id);
                            self.message = "Input format updated; other audio continues".into();
                        }
                        Ok(Prepared::Output(stream)) => {
                            if target.listen {
                                if let Some(existing) = self
                                    .listeners
                                    .iter_mut()
                                    .find(|s| s.stream.node_id == target.id)
                                {
                                    existing.stream = stream;
                                }
                            } else if let Some(existing) =
                                self.outputs.iter_mut().find(|s| s.node_id == target.id)
                            {
                                *existing = stream;
                            }
                            self.message = "Output format updated; other audio continues".into();
                        }
                        Err(error) => {
                            crate::logging::warn(format_args!(
                                "Format update for node {}: {}",
                                target.id, error
                            ));
                            self.message = format!("Format update failed: {error}. Previous stream continues; retrying.");
                            self.format_changes.failed = Some((target, Instant::now()));
                        }
                    }
                }
            }
        }
        if self.format_changes.pending.is_some() {
            return;
        }
        if self
            .format_changes
            .scanned
            .is_some_and(|t| t.elapsed() < Duration::from_millis(500))
        {
            return;
        }
        self.format_changes.scanned = Some(Instant::now());
        let mut outstanding = false;
        for node in &self.project.nodes {
            let primary_changed = match &node.kind {
                NodeKind::Input { .. } => self
                    .inputs
                    .iter()
                    .any(|(id, s)| *id == node.id && s.requested_format != node.audio_format),
                NodeKind::Output { .. } => self
                    .outputs
                    .iter()
                    .any(|s| s.node_id == node.id && s.requested_format != node.audio_format),
                _ => false,
            };
            let listen_changed = node.listen_enabled
                && self.listeners.iter().any(|s| {
                    s.stream.node_id == node.id && s.stream.requested_format != node.listen_format
                });
            if !primary_changed && !listen_changed {
                continue;
            }
            outstanding = true;
            let listen = !primary_changed;
            let device = if listen {
                node.listen_device.clone()
            } else {
                match &node.kind {
                    NodeKind::Input { device, .. } | NodeKind::Output { device } => device.clone(),
                    _ => None,
                }
            };
            let target = Target {
                id: node.id,
                kind: node.kind.clone(),
                listen,
                device,
                preference: if listen {
                    node.listen_format.clone()
                } else {
                    node.audio_format.clone()
                },
            };
            if self
                .format_changes
                .failed
                .as_ref()
                .is_some_and(|(t, at)| t == &target && at.elapsed() < Duration::from_secs(5))
            {
                continue;
            }
            let (tx, rx) = mpsc::sync_channel(1);
            let counters = self.counters.clone();
            let request = target.clone();
            self.format_changes.pending = Some((target, rx));
            std::thread::spawn(move || {
                let errors = Arc::new(ArrayQueue::new(4));
                let result = if !request.listen {
                    if let NodeKind::Input { loopback, .. } = request.kind {
                        audio_io::open_input_with_format(
                            &request.device,
                            loopback,
                            &request.preference,
                            counters,
                            errors,
                        )
                        .map(|s| Prepared::Input(Box::new(s)))
                    } else {
                        audio_io::open_output_with_format(
                            request.id,
                            &request.device,
                            &request.preference,
                            counters,
                            errors,
                        )
                        .map(Prepared::Output)
                    }
                } else {
                    audio_io::open_output_with_format(
                        request.id,
                        &request.device,
                        &request.preference,
                        counters,
                        errors,
                    )
                    .map(Prepared::Output)
                };
                let _ = tx.send(result.map_err(|e| e.to_string()));
            });
            break;
        }
        self.format_changes.dirty = outstanding;
        if !outstanding {
            self.format_changes.failed = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use piratewave_core::project::Node;
    #[test]
    fn stale_format_results_cannot_replace_current_assignment() {
        let mut node = Node::new(
            1,
            "Mic",
            NodeKind::Input {
                device: Some("mic".into()),
                loopback: false,
            },
            [0.; 2],
        );
        let target = Target {
            id: 1,
            kind: node.kind.clone(),
            listen: false,
            device: Some("mic".into()),
            preference: AudioFormat::default(),
        };
        assert!(target.matches(&node));
        node.audio_format.sample_rate = Some(96000);
        assert!(!target.matches(&node));
        node.audio_format = AudioFormat::default();
        node.id = 2;
        assert!(!target.matches(&node));
        node.id = 1;
        node.kind = NodeKind::Input {
            device: Some("other".into()),
            loopback: false,
        };
        assert!(!target.matches(&node));
        let mut target = target;
        target.kind = node.kind.clone();
        target.listen = true;
        assert!(!target.matches(&node));
        node.listen_enabled = true;
        node.listen_device = Some("mic".into());
        assert!(target.matches(&node));
        node.listen_format.sample_rate = Some(44100);
        assert!(!target.matches(&node));
    }
}
