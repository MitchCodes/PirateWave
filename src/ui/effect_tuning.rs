//! Reusable effect tuning aids and one explicitly selected live diagnostic tap.
use super::{
    audio_widgets::{self, Axis, Curve},
    egui, theme,
};
use crate::{
    engine::{Command, Engine},
    tools::analysis::Analyzer,
};
use piratewave_core::{
    effects::{EffectFactory, EffectSpec},
    graph::EffectMonitor,
    SAMPLE_RATE,
};
use std::{
    collections::{BTreeMap, HashMap},
    time::{Duration, Instant},
};

#[derive(Default)]
pub struct TuningUi {
    snapshots: HashMap<(u64, u64), BTreeMap<String, f32>>,
    selected: Option<(u64, u64)>,
    visible: bool,
    subscribed: bool,
    subscription: Option<(u64, u64)>,
    token: u64,
    heartbeat: Option<Instant>,
    updated: Option<Instant>,
    analyzed: Option<Instant>,
    analyzers: Option<[Analyzer; 2]>,
    spectra: [Vec<[f32; 2]>; 2],
    block: Option<EffectMonitor>,
    error: Option<String>,
    tempo: f32,
    right_channel: bool,
    fixture: bool,
    fixture_stage: Option<usize>,
}
fn value(spec: &EffectSpec, factory: &EffectFactory, id: &str) -> f32 {
    spec.parameters.get(id).copied().unwrap_or_else(|| {
        factory
            .parameters
            .iter()
            .find(|p| p.id == id)
            .map_or(0., |p| p.default)
    })
}
fn db(amplitude: f32) -> f32 {
    20. * amplitude.max(1e-6).log10()
}

impl TuningUi {
    pub fn begin_frame(&mut self, project: &piratewave_core::project::Project) {
        self.visible = false;
        let exists = |target: &(u64, u64)| {
            project
                .nodes
                .iter()
                .any(|n| n.id == target.0 && n.effects.iter().any(|e| e.id == target.1))
        };
        self.snapshots.retain(|target, _| exists(target));
        self.selected = self.selected.filter(exists);
    }

    /// Shared snapshots and defaults operate through the same rack change path as sliders.
    pub fn controls(
        &mut self,
        ui: &mut egui::Ui,
        node: u64,
        spec: &mut EffectSpec,
        factory: &EffectFactory,
    ) -> bool {
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button("Save comparison")
                .on_hover_text(
                    "Remember these settings for this session, then tweak and swap to compare.",
                )
                .clicked()
            {
                self.snapshots
                    .insert((node, spec.id), spec.parameters.clone());
            }
            if ui
                .add_enabled(
                    self.snapshots.contains_key(&(node, spec.id)),
                    egui::Button::new("Swap comparison").small(),
                )
                .clicked()
            {
                if let Some(saved) = self.snapshots.get_mut(&(node, spec.id)) {
                    changed = *saved != spec.parameters;
                    std::mem::swap(saved, &mut spec.parameters);
                }
            }
            if ui.small_button("Reset effect").clicked() {
                let defaults = factory
                    .parameters
                    .iter()
                    .map(|p| (p.id.to_string(), p.default))
                    .collect();
                changed |= spec.parameters != defaults;
                spec.parameters = defaults;
            }
        });
        if has_guide(&spec.kind) {
            egui::CollapsingHeader::new("Tuning guide")
                .default_open(self.fixture && self.selected.is_none())
                .id_salt((node, spec.id, "tuning"))
                .show(ui, |ui| {
                    changed |= guide(ui, spec, factory, &mut self.tempo);
                });
        }
        changed
    }

    pub fn live_panel(&mut self, ui: &mut egui::Ui, node: u64, effect: u64, recording: bool) {
        // Recording racks process each playing slot separately, not their capture path.
        // Do not present the live capture signal as playback-effect telemetry.
        if recording {
            return;
        }
        let response = egui::CollapsingHeader::new("Live before / after").default_open(self.fixture && self.selected.is_some()).id_salt((node,effect,"monitor")).show(ui, |ui| {
            let target = (node,effect);
            let mut active = self.selected == Some(target);
            if ui.checkbox(&mut active, "Monitor this effect").changed() {
                self.selected = active.then_some(target);
            }
            ui.small("One effect at a time. Purple: input; green: output. Analysis pauses offscreen.");
            if !active { return; }
            if let Some(error) = &self.error { ui.colored_label(theme::palette().peach,error); }
            if !self.updated.is_some_and(|at| at.elapsed() < Duration::from_millis(500)) {
                ui.label("Waiting for audio. Start audio and route a source through this rack.");
            }
            if let Some(block) = self.block.as_ref().filter(|b| (b.node,b.effect) == target) {
                let stats = |frames: &[[f32;2]]| {
                    let mut power = [0_f32;2]; let mut peak = 0_f32;
                    for frame in frames { for c in 0..2 { power[c] += frame[c]*frame[c]; peak = peak.max(frame[c].abs()); } }
                    (db((power[0].max(power[1]) / frames.len() as f32).sqrt()), db(peak))
                };
                let before = stats(&block.before); let after = stats(&block.after);
                let cross: f32 = block.after.iter().map(|f| f[0]*f[1]).sum();
                let left: f32 = block.after.iter().map(|f| f[0]*f[0]).sum();
                let right: f32 = block.after.iter().map(|f| f[1]*f[1]).sum();
                let correlation = (left*right > 1e-12).then(|| (cross/(left*right).sqrt()).clamp(-1.,1.));
                ui.horizontal_wrapped(|ui| {
                    ui.colored_label(theme::palette().accent,format!("In {:.1} dBFS RMS / {:.1} peak",before.0,before.1));
                    ui.colored_label(theme::palette().green,format!("Out {:.1} dBFS RMS / {:.1} peak",after.0,after.1));
                    if before.0 > -80. { ui.label(format!("Level change {:+.1} dB",after.0-before.0)); }
                    if let Some(correlation) = correlation { ui.label(format!("Stereo correlation {correlation:+.2}")); }
                    if block.bypassed { ui.colored_label(theme::palette().peach,"Bypassed"); }
                    if after.1 >= 0. { ui.colored_label(theme::palette().peach,"Above 0 dBFS: reduce output gain"); }
                });
                ui.small("Correlation near -1 can cancel in mono. RMS uses the louder channel. Level change includes wet/dry, output gain and tails; it is not an internal gain-reduction meter.");
            }
            audio_widgets::plot(ui,Axis::Log(20.,20000.),Axis::Linear(-100.,0.), &[
                Curve{points:&self.spectra[0],color:theme::palette().accent},Curve{points:&self.spectra[1],color:theme::palette().green}
            ],"Spectrum: Hz / dBFS; stereo channels analyzed independently");
            ui.small("Spectrum: 2048 samples, Hann window, louder channel per bin. Display frames may be skipped under load.");
            {
                let empty = EffectMonitor::new(node,effect,0);
                let block = self.block.as_ref().filter(|b| (b.node,b.effect) == target).unwrap_or(&empty);
                let wave = |frames: &[[f32;2]], channel: usize| frames.iter().enumerate().map(|(i,f)| [i as f32 * 1000. / SAMPLE_RATE as f32,f[channel]]).collect::<Vec<_>>();
                ui.checkbox(&mut self.right_channel, "Right channel waveform");
                let channel = usize::from(self.right_channel);
                let before = wave(&block.before,channel); let after = wave(&block.after,channel);
                audio_widgets::plot(ui,Axis::Linear(0.,block.before.len() as f32*1000./SAMPLE_RATE as f32),Axis::Linear(-1.,1.),&[
                    Curve{points:&before,color:theme::palette().accent},Curve{points:&after,color:theme::palette().green}
                ],"Waveform: milliseconds / amplitude");
                ui.small("Waveform: selected channel, latest block; delayed effects are not time-aligned to their input.");
            }
        });
        if self.selected == Some((node, effect))
            && egui::collapsing_header::CollapsingState::load(ui.ctx(), response.header_response.id)
                .is_some_and(|state| state.is_open())
            && response
                .body_response
                .is_some_and(|r| ui.is_rect_visible(r.rect))
        {
            self.visible = true;
        }
    }

    pub fn finish_frame(&mut self, engine: &Engine, running: bool) {
        if self.fixture {
            self.updated = Some(Instant::now());
            return;
        }
        let target = self.selected.filter(|_| self.visible && running);
        if target.is_none() {
            if self.subscribed {
                if let Err(error) = engine.send(Command::MonitorEffect(None)) {
                    self.error = Some(error);
                }
            }
            self.subscribed = false;
            self.subscription = None;
            self.heartbeat = None;
            self.updated = None;
            self.block = None;
            self.analyzers = None;
            self.spectra = Default::default();
            return;
        }
        let (node, effect) = target.unwrap();
        let changed = self.subscription != Some((node, effect));
        if changed || !self.subscribed {
            self.token = self.token.wrapping_add(1);
            self.analyzers = Some([
                Analyzer::new(SAMPLE_RATE, 2048),
                Analyzer::new(SAMPLE_RATE, 2048),
            ]);
            self.block = None;
            self.updated = None;
            self.spectra = Default::default();
            self.heartbeat = None;
            while engine.effect_audio.pop().is_some() {}
        }
        if self
            .heartbeat
            .is_none_or(|at| at.elapsed() >= Duration::from_millis(200))
        {
            match engine.send(Command::MonitorEffect(Some((node, effect, self.token)))) {
                Ok(()) => {
                    self.error = None;
                    self.subscribed = true;
                    self.subscription = Some((node, effect));
                    self.heartbeat = Some(Instant::now());
                }
                Err(error) => self.error = Some(error),
            }
        }
        while let Some(block) = engine.effect_audio.pop() {
            if (block.node, block.effect, block.token) != (node, effect, self.token) {
                continue;
            }
            if let Some(analyzers) = &mut self.analyzers {
                for frame in block.before {
                    analyzers[0].push(frame);
                }
                for frame in block.after {
                    analyzers[1].push(frame);
                }
            }
            self.block = Some(block);
            self.updated = Some(Instant::now());
        }
        if self.updated.is_some()
            && self
                .analyzed
                .is_none_or(|at| at.elapsed() >= Duration::from_millis(50))
        {
            if let Some(analyzers) = &mut self.analyzers {
                for (i, analyzer) in analyzers.iter_mut().enumerate() {
                    self.spectra[i] = analyzer
                        .spectrum()
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(bin, db)| [bin as f32 * SAMPLE_RATE as f32 / 2048., *db])
                        .collect();
                }
            }
            self.analyzed = Some(Instant::now());
        }
    }
}

fn has_guide(kind: &str) -> bool {
    matches!(
        kind,
        "equalizer"
            | "high_pass"
            | "compressor"
            | "gate"
            | "limiter"
            | "gain"
            | "automatic_gain_control"
            | "expander"
            | "upward_compressor"
            | "delay"
            | "tremolo"
            | "flanger"
            | "chorus"
            | "wahwah"
            | "pitch"
            | "resonator"
            | "stereo"
            | "de_esser"
    )
}

fn guide(
    ui: &mut egui::Ui,
    spec: &mut EffectSpec,
    factory: &EffectFactory,
    tempo: &mut f32,
) -> bool {
    let v = |id| value(spec, factory, id);
    let mut changed = false;
    match spec.kind.as_str() {
        "equalizer" | "high_pass" => {
            ui.small("Frequency response: Hz / dB. Hover to read values; this is the configured filter response, not live audio.");
            let points: Vec<_> = (0..256)
                .map(|i| {
                    let hz = Axis::Log(20., 20000.).value(i as f32 / 255.);
                    [
                        hz,
                        filter_response(
                            &spec.kind,
                            hz,
                            v("frequency"),
                            v("gain"),
                            v("q"),
                            v("cutoff"),
                        ),
                    ]
                })
                .collect();
            audio_widgets::plot(
                ui,
                Axis::Log(20., 20000.),
                Axis::Linear(-36., 24.),
                &[Curve {
                    points: &points,
                    color: theme::palette().accent,
                }],
                "Frequency response: Hz / dB",
            );
            ui.small("Cut rumble with High Pass. Use a wide Q for gentle tone shaping; narrow Q isolates a problem frequency.");
        }
        "compressor"
        | "gate"
        | "limiter"
        | "gain"
        | "automatic_gain_control"
        | "expander"
        | "upward_compressor" => {
            let points: Vec<_> = (0..241)
                .map(|i| {
                    let input = -80. + i as f32 / 3.;
                    [input, transfer(spec, factory, input)]
                })
                .collect();
            let unity = [[-80., -80.], [0., 0.]];
            audio_widgets::plot(
                ui,
                Axis::Linear(-80., 0.),
                Axis::Linear(-80., 12.),
                &[
                    Curve {
                        points: &unity,
                        color: theme::palette().muted,
                    },
                    Curve {
                        points: &points,
                        color: theme::palette().accent,
                    },
                ],
                "Input dBFS / settled output dBFS",
            );
            ui.small("Static target curve; attack, release and detector history affect real audio. Gray: unchanged level. Listen while adjusting threshold, then match loudness with output gain.");
        }
        "delay" => {
            let time = v("time");
            let feedback = v("feedback");
            let points: Vec<_> = (0..12)
                .map(|i| {
                    [
                        time * (i + 1) as f32,
                        db((v("mix") * feedback.powi(i)).abs()),
                    ]
                })
                .collect();
            audio_widgets::plot(
                ui,
                Axis::Linear(0., time * 12.),
                Axis::Linear(-60., 0.),
                &[Curve {
                    points: &points,
                    color: theme::palette().accent,
                }],
                "Echo arrival milliseconds / level dB (impulse prediction)",
            );
            if *tempo == 0. {
                *tempo = 120.;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label("Tempo");
                ui.add(
                    egui::DragValue::new(tempo)
                        .range(30. ..=300.)
                        .suffix(" BPM"),
                );
                for (label, beats) in [
                    ("1/4", 1.),
                    ("1/8", 0.5),
                    ("Dotted 1/8", 0.75),
                    ("1/8 triplet", 1. / 3.),
                ] {
                    let ms = 60000. / *tempo * beats;
                    let valid = factory
                        .parameters
                        .iter()
                        .find(|p| p.id == "time")
                        .is_some_and(|p| (p.min..=p.max).contains(&ms));
                    if ui
                        .add_enabled(valid, egui::Button::new(label).small())
                        .on_hover_text(format!("{ms:.1} ms"))
                        .clicked()
                    {
                        spec.parameters.insert("time".into(), ms);
                        changed = ms != time;
                    }
                }
            });
            ui.small("Tempo is a local calculator. Click a note to apply its time; future tempo changes do not alter audio.");
        }
        "tremolo" => {
            let rate = v("rate");
            let depth = v("depth");
            let square = v("mode") >= 0.5;
            let points: Vec<_> = (0..256)
                .map(|i| {
                    let t = i as f32 / 255. * 2. / rate;
                    let sine = (std::f32::consts::TAU * t * rate).sin();
                    let lfo = if square {
                        if sine >= 0. {
                            1.
                        } else {
                            -1.
                        }
                    } else {
                        sine
                    };
                    [t * 1000., 1. - depth * (0.5 - 0.5 * lfo)]
                })
                .collect();
            audio_widgets::plot(
                ui,
                Axis::Linear(0., 2000. / rate),
                Axis::Linear(0., 1.),
                &[Curve {
                    points: &points,
                    color: theme::palette().accent,
                }],
                "Modulation cycle: milliseconds / gain",
            );
            ui.label(format!(
                "{:.1} ms per pulse; {:.0} pulses/minute",
                1000. / rate,
                rate * 60.
            ));
        }
        "flanger" | "chorus" | "wahwah" => {
            ui.label(format!(
                "Sweep cycle: {:.0} ms ({:.1} cycles/minute)",
                1000. / v("rate"),
                60. * v("rate")
            ));
            if spec.kind == "wahwah" {
                ui.label(format!(
                    "Sweep range: {:.0} to {:.0} Hz",
                    v("frequency"),
                    v("frequency") * 2_f32.powf(v("depth"))
                ));
            }
            ui.small("Slower rates make gentle movement; reduce depth or wet/dry for a more natural voice.");
        }
        "pitch" => {
            ui.label(format!(
                "Frequency multiplier: {:.3}x",
                2_f32.powf(v("semitones") / 12.)
            ));
            ui.small("12 semitones is one octave. Blend dry audio for a layered voice; this processor does not preserve formants.");
        }
        "resonator" => {
            ui.label(format!(
                "Tuning: {:.1} Hz / MIDI note {:.1}",
                v("frequency"),
                69. + 12. * (v("frequency") / 440.).log2()
            ));
            ui.small("Higher resonance rings longer. Raise damping to soften brightness; lower wet/dry to retain intelligibility.");
        }
        "stereo" => {
            ui.small("Check the live stereo spectrum and output level while listening in mono. Width above 1 and right-channel delay can weaken mono compatibility.");
        }
        "de_esser" => {
            ui.small("Use the live spectrum to locate harsh S sounds, then set Sibilance cutoff near that band. Start with low maximum reduction to avoid a lisp.");
        }
        _ => {
            ui.small("Save a comparison before tuning. Match output loudness when comparing settings; louder is not always clearer.");
        }
    }
    changed
}

fn transfer(spec: &EffectSpec, factory: &EffectFactory, input: f32) -> f32 {
    let v = |id| value(spec, factory, id);
    let threshold = v("threshold");
    let ratio = v("ratio").max(1.);
    match spec.kind.as_str() {
        "gain" => input + v("gain"),
        "compressor" => input + (threshold - input).min(0.) * (1. - 1. / ratio) + v("makeup"),
        "limiter" => input.min(v("ceiling")),
        "gate" => {
            if input > threshold {
                input
            } else {
                -120.
            }
        }
        "automatic_gain_control" => {
            input
                + if input > v("floor") {
                    (v("target") - input).clamp(-v("reduction"), v("boost"))
                } else {
                    0.
                }
        }
        "upward_compressor" => {
            input + ((threshold - input).max(0.) * (1. - 1. / ratio)).min(v("range")) + v("output")
        }
        "expander" => {
            input
                + if v("mode") >= 0.5 {
                    ((input - threshold).max(0.) * (ratio - 1.)).min(v("range"))
                } else {
                    ((input - threshold).min(0.) * (ratio - 1.)).max(-v("range"))
                }
                + v("output")
        }
        _ => input,
    }
}

fn filter_response(kind: &str, hz: f32, center: f32, gain: f32, q: f32, cutoff: f32) -> f32 {
    if kind == "high_pass" {
        let a = 1. / (1. + std::f64::consts::TAU * cutoff as f64 / SAMPLE_RATE as f64);
        let w = std::f64::consts::TAU * hz as f64 / SAMPLE_RATE as f64;
        return (10.
            * ((a * a * (2. - 2. * w.cos())) / (1. + a * a - 2. * a * w.cos()))
                .max(1e-24)
                .log10()) as f32;
    }
    audio_widgets::bell_response(hz, center, gain, q)
}

impl super::PirateWaveApp {
    /// Native visual fixtures never open devices or subscribe to engine audio.
    pub(super) fn effect_tuning_smoke_tick(&mut self, ctx: &egui::Context) {
        let pages = [
            ("equalizer", 1380., false),
            ("compressor", 1000., false),
            ("delay", 1380., false),
            ("tremolo", 1000., false),
            ("gain", 1380., true),
            ("gain", 1000., true),
        ];
        if self.smoke_stage >= pages.len() {
            let _ = std::fs::write(
                "dist/effect-tuning-smoke-result.txt",
                "PASS: six native tuning screenshots; generated audio only, no devices opened.\n",
            );
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if !self.smoke_waiting {
                    continue;
                }
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(error) = image::save_buffer(
                    format!("dist/native-effect-tuning-{}.png", self.smoke_stage),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    let _ = std::fs::write(
                        "dist/effect-tuning-smoke-result.txt",
                        format!("FAIL: {error}"),
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                self.smoke_stage += 1;
                self.smoke_waiting = false;
                self.smoke_at = Instant::now();
                return;
            }
        }
        if self.effect_tuning.fixture_stage != Some(self.smoke_stage) {
            let (kind, width, live) = pages[self.smoke_stage];
            let mut spec = EffectSpec::new(self.registry.get(kind).unwrap());
            if live {
                spec.parameters.insert("gain".into(), -6.0206);
            }
            if kind == "equalizer" {
                spec.parameters.insert("gain".into(), 6.);
            }
            let effect = spec.id;
            let mut node = piratewave_core::project::Node::new(
                999,
                "Generated diagnostics fixture",
                piratewave_core::project::NodeKind::Processor,
                [0.; 2],
            );
            node.effects.push(spec);
            self.project.nodes.retain(|n| n.id != 999);
            self.project.nodes.push(node);
            self.selected = 999;
            self.view = super::View::Effects;
            self.effect_tuning.fixture = true;
            self.effect_tuning.fixture_stage = Some(self.smoke_stage);
            self.effect_tuning.selected = live.then_some((999, effect));
            if live {
                let mut tap = EffectMonitor::new(999, effect, 0);
                tap.valid = true;
                let mut analyzers = [
                    Analyzer::new(SAMPLE_RATE, 2048),
                    Analyzer::new(SAMPLE_RATE, 2048),
                ];
                for i in 0..2048 {
                    let x =
                        (std::f32::consts::TAU * 750. * i as f32 / SAMPLE_RATE as f32).sin() * 0.6;
                    let dry = [x, -x];
                    let wet = dry.map(|x| x * 0.5);
                    analyzers[0].push(dry);
                    analyzers[1].push(wet);
                    if i < tap.before.len() {
                        tap.before[i] = dry;
                        tap.after[i] = wet;
                    }
                }
                for (i, a) in analyzers.iter_mut().enumerate() {
                    self.effect_tuning.spectra[i] = a
                        .spectrum()
                        .iter()
                        .enumerate()
                        .skip(1)
                        .map(|(bin, db)| [bin as f32 * SAMPLE_RATE as f32 / 2048., *db])
                        .collect();
                }
                self.effect_tuning.block = Some(tap);
                self.effect_tuning.updated = Some(Instant::now());
            }
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(
                width,
                if width == 1000. { 700. } else { 960. },
            )));
            self.smoke_at = Instant::now();
            return;
        }
        if !self.smoke_waiting && self.smoke_at.elapsed() > Duration::from_millis(250) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.smoke_waiting = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use piratewave_core::{
        effects::EffectRegistry,
        project::{Node, NodeKind, Project},
        BLOCK_SIZE,
    };

    #[test]
    fn configured_response_matches_actual_filters_at_audio_frequencies() {
        let registry = EffectRegistry::default();
        for kind in ["equalizer", "high_pass"] {
            let factory = registry.get(kind).unwrap();
            let mut spec = EffectSpec::new(factory);
            if kind == "equalizer" {
                spec.parameters.insert("gain".into(), 9.);
            }
            for hz in [100., 1000., 10000.] {
                let mut effect = registry.create(&spec, SAMPLE_RATE as f32).unwrap();
                let mut input = 0_f64;
                let mut output = 0_f64;
                for block in 0..800 {
                    let mut frames = std::array::from_fn::<_, BLOCK_SIZE, _>(|i| {
                        let sample = (std::f32::consts::TAU * hz * (block * BLOCK_SIZE + i) as f32
                            / SAMPLE_RATE as f32)
                            .sin()
                            * 0.1;
                        if block > 400 {
                            input += (sample as f64).powi(2);
                        }
                        [sample, -sample]
                    });
                    effect.process(&mut frames);
                    if block > 400 {
                        output += frames.iter().map(|f| (f[0] as f64).powi(2)).sum::<f64>();
                    }
                }
                let measured = (10. * (output / input).log10()) as f32;
                let v = |id| value(&spec, factory, id);
                let predicted =
                    filter_response(kind, hz, v("frequency"), v("gain"), v("q"), v("cutoff"));
                assert!(
                    (measured - predicted).abs() < 0.04,
                    "{kind} {hz}: {measured} vs {predicted}"
                );
            }
        }
    }

    #[test]
    fn dynamics_curves_match_thresholds_limits_and_defaults() {
        let registry = EffectRegistry::default();
        for kind in [
            "compressor",
            "limiter",
            "gain",
            "automatic_gain_control",
            "expander",
            "upward_compressor",
        ] {
            let factory = registry.get(kind).unwrap();
            let spec = EffectSpec::new(factory);
            for input in [-65., -45., -20., -5.] {
                let mut effect = registry.create(&spec, SAMPLE_RATE as f32).unwrap();
                let mut frames = [[10_f32.powf(input / 20.); 2]; BLOCK_SIZE];
                for _ in 0..6000 {
                    frames.fill([10_f32.powf(input / 20.); 2]);
                    effect.process(&mut frames);
                }
                let measured = db(frames[BLOCK_SIZE - 1][0].abs());
                assert!(
                    (measured - transfer(&spec, factory, input)).abs() < 0.12,
                    "{kind}: {measured} for {input}"
                );
            }
        }
    }

    #[test]
    fn monitoring_only_subscribes_when_visible_and_rejects_old_packets() {
        let (engine, rx) = Engine::test_channel();
        let mut ui = TuningUi {
            selected: Some((1, 2)),
            ..Default::default()
        };
        ui.finish_frame(&engine, true);
        assert!(rx.try_recv().is_err());
        ui.visible = true;
        ui.finish_frame(&engine, true);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::MonitorEffect(Some((1, 2, 1)))
        ));
        let mut packet = EffectMonitor::new(1, 2, 1);
        packet.valid = true;
        packet.before.fill([0.1, -0.1]);
        packet.after = packet.before;
        engine
            .effect_audio
            .push(packet)
            .unwrap_or_else(|_| panic!());
        ui.finish_frame(&engine, true);
        assert!(ui.block.is_some());
        ui.selected = Some((1, 3));
        ui.finish_frame(&engine, true);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::MonitorEffect(Some((1, 3, 2)))
        ));
        engine
            .effect_audio
            .push(packet)
            .unwrap_or_else(|_| panic!());
        ui.finish_frame(&engine, true);
        assert!(ui.block.is_none());
        ui.visible = false;
        ui.finish_frame(&engine, true);
        assert!(matches!(
            rx.try_recv().unwrap(),
            Command::MonitorEffect(None)
        ));
        assert!(ui.analyzers.is_none());
        assert!(ui.updated.is_none());
        engine
            .effect_audio
            .push(packet)
            .unwrap_or_else(|_| panic!());
        ui.finish_frame(&engine, true);
        assert_eq!(
            engine.effect_audio.len(),
            1,
            "Hidden controls must not poll audio"
        );
        ui.visible = true;
        ui.finish_frame(&engine, false);
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn rendering_guides_does_not_edit_projects_or_send_audio_updates() {
        let ctx = egui::Context::default();
        let registry = EffectRegistry::default();
        let (engine, rx) = Engine::test_channel();
        let mut tuning = TuningUi::default();
        for factory in registry.factories().iter().filter(|f| !f.id.contains('/')) {
            let mut spec = EffectSpec::new(factory);
            let original = spec.parameters.clone();
            for width in [600., 1000., 1600.] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 700.),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        assert!(!guide(ui, &mut spec, factory, &mut tuning.tempo));
                        assert!(!tuning.controls(ui, 1, &mut spec, factory));
                        tuning.live_panel(ui, 1, spec.id, false);
                    });
                });
                tuning.finish_frame(&engine, true);
                assert_eq!(spec.parameters, original);
            }
        }
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn deleting_effect_clears_monitor_selection_and_comparison_memory() {
        let mut tuning = TuningUi::default();
        let mut p = Project {
            nodes: vec![Node::new(1, "Rack", NodeKind::Processor, [0.; 2])],
            ..Project::default()
        };
        tuning.selected = Some((1, 2));
        tuning.snapshots.insert((1, 2), BTreeMap::new());
        tuning.begin_frame(&p);
        assert!(tuning.selected.is_none());
        assert!(tuning.snapshots.is_empty());
        p.nodes.clear();
        tuning.begin_frame(&p);
        assert!(!tuning.visible);
    }

    #[test]
    fn live_panel_visibility_follows_clipping_and_collapsing() {
        let ctx = egui::Context::default();
        let mut tuning = TuningUi {
            selected: Some((1, 2)),
            fixture: true,
            ..Default::default()
        };
        let draw = |tuning: &mut TuningUi, offset: f32, close: bool| {
            tuning.visible = false;
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 600.),
                )),
                ..Default::default()
            };
            let mut input = input;
            if close {
                let pos = egui::pos2(60., 18.);
                input.events = vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ];
            }
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add_space(offset);
                    tuning.live_panel(ui, 1, 2, false);
                });
            });
        };
        draw(&mut tuning, 0., false);
        assert!(tuning.visible);
        draw(&mut tuning, 900., false);
        assert!(!tuning.visible);
        draw(&mut tuning, 0., false);
        draw(&mut tuning, 0., true);
        assert!(!tuning.visible);
    }
}
