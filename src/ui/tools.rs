use super::audio_widgets::bell_response;
use super::*;
use crate::tools::{
    self as workbench,
    analysis::Reading,
    icons::{self, Design, Glyph},
    session::{Config, Mode, Session, Snapshot, Wave},
    Tool,
};

pub(super) struct ToolsUi {
    filter: String,
    category: String,
    config: Config,
    session: Option<Session>,
    owner: Option<Tool>,
    last_view: Option<Tool>,
    results: HashMap<Tool, Snapshot>,
    suite: HashMap<Mode, Snapshot>,
    pub snapshot: Snapshot,
    frozen: bool,
    held: Vec<f32>,
    shown: Vec<f32>,
    latest_frames: u64,
    devices: Vec<audio_io::EndpointReport>,
    scan: Option<mpsc::Receiver<Vec<audio_io::EndpointReport>>>,
    scanned: bool,
    endpoint_filter: String,
    message: String,
    buffer: u32,
    rate: u32,
    stages: u32,
    region: usize,
    eq_gain: f32,
    eq_q: f32,
    digital_db: f32,
    acoustic: usize,
    voice_preset: usize,
    design: Design,
    preview: Option<egui::TextureHandle>,
    rendered: Option<Design>,
    export: Option<mpsc::Receiver<Result<String, String>>>,
    headphone_confirmed: bool,
    smoke_prepared: bool,
}
impl Default for ToolsUi {
    fn default() -> Self {
        Self {
            filter: String::new(),
            category: "All".into(),
            config: Config::default(),
            session: None,
            owner: None,
            last_view: None,
            results: HashMap::new(),
            suite: HashMap::new(),
            snapshot: Snapshot::default(),
            frozen: false,
            held: vec![],
            shown: vec![],
            latest_frames: 0,
            devices: vec![],
            scan: None,
            scanned: false,
            endpoint_filter: String::new(),
            message: String::new(),
            buffer: 512,
            rate: 48000,
            stages: 2,
            region: 4,
            eq_gain: 0.,
            eq_q: 0.707,
            digital_db: -18.,
            acoustic: 2,
            voice_preset: 0,
            design: Design::default(),
            preview: None,
            rendered: None,
            export: None,
            headphone_confirmed: false,
            smoke_prepared: false,
        }
    }
}
impl ToolsUi {
    pub fn stop(&self) {
        if let Some(s) = &self.session {
            s.stop();
        }
    }
    pub fn poll(&mut self, current: Option<Tool>) {
        if self.owner.is_some() && self.owner != current {
            self.stop();
        }
        if self.last_view != current {
            self.last_view = current;
            self.snapshot = current
                .and_then(|tool| self.results.get(&tool))
                .cloned()
                .unwrap_or_default();
            self.shown = self.snapshot.spectrum.clone();
            self.held = self.shown.clone();
            self.latest_frames = 0;
            self.frozen = false;
        }
        if let Some(session) = &self.session {
            let next = (**session.state.load()).clone();
            if self.owner == current {
                self.snapshot = next.clone();
                if self.latest_frames != next.reading.frames {
                    self.latest_frames = next.reading.frames;
                    if !self.frozen {
                        self.shown = next.spectrum.clone();
                    }
                    if self.held.len() != next.spectrum.len() {
                        self.held = vec![-120.; next.spectrum.len()];
                    }
                    if !self.frozen {
                        for (held, value) in self.held.iter_mut().zip(&next.spectrum) {
                            *held = held.max(*value);
                        }
                    }
                }
            } else {
                self.snapshot.message = "Closing the previous tool's streams…".into();
                self.snapshot.running = false;
            }
            if session.finished() {
                if let Some(owner) = self.owner {
                    self.results.insert(owner, next.clone());
                    if owner == Tool::SoundCheck {
                        self.suite.insert(self.config.mode, next);
                    }
                }
                self.session = None;
                self.owner = None;
            }
        }
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok(devices) => {
                    self.devices = devices;
                    self.scanned = true;
                    self.scan = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.message = "Endpoint scan failed; refresh to retry.".into();
                }
                _ => {}
            }
        }
        if let Some(rx) = &self.export {
            match rx.try_recv() {
                Ok(result) => {
                    self.message = result.unwrap_or_else(|e| format!("Export failed: {e}"));
                    self.export = None;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.message = "Export worker stopped unexpectedly".into();
                    self.export = None;
                }
                _ => {}
            }
        }
    }
    fn scan(&mut self) {
        if self.scan.is_some() {
            return;
        }
        let (tx, rx) = mpsc::sync_channel(1);
        self.scan = Some(rx);
        thread::spawn(move || {
            let devices = audio_io::inspect_devices();
            let _ = tx.send(devices);
        });
    }
    fn update(&mut self) {
        if let Some(s) = &self.session {
            if let Err(e) = s.update(self.config.clone()) {
                self.message = e;
            }
        }
    }
    fn start(&mut self, tool: Tool, mode: Mode, smoke: bool) {
        if self.session.is_some() {
            self.message =
                "Stop the current tool and wait for its devices to close before starting another."
                    .into();
            return;
        }
        self.config.mode = mode;
        self.snapshot = Snapshot::default();
        self.frozen = false;
        self.shown.clear();
        self.held.clear();
        self.latest_frames = 0;
        self.message.clear();
        if smoke {
            self.message = "Screenshot fixture: no audio devices opened.".into();
            return;
        }
        match Session::start(self.config.clone()) {
            Ok(session) => {
                self.owner = Some(tool);
                self.session = Some(session);
            }
            Err(error) => self.message = error,
        }
    }
    fn busy(&self) -> bool {
        self.session.is_some()
    }
}

impl PirateWaveApp {
    pub(super) fn tools_smoke_tick(&mut self, ctx: &egui::Context) {
        let mut pages = vec![
            (View::Tools, "gallery-wide".to_owned(), 1380.),
            (View::Tools, "gallery-narrow".to_owned(), 1000.),
        ];
        pages.extend(Tool::ALL.into_iter().map(|tool| {
            (
                View::Tool(tool),
                tool.name().to_lowercase().replace(' ', "-"),
                1380.,
            )
        }));
        pages.push((View::Tool(Tool::VoiceSandbox), "voice-narrow".into(), 1000.));
        pages.push((View::Tool(Tool::KeyCanvas), "icons-narrow".into(), 1000.));
        if self.smoke_stage >= pages.len() {
            let _=std::fs::write("dist/tools-ui-smoke-result.txt",format!("PASS: {} native Tools screenshots. Generated signal and endpoint fixtures only; no audio devices opened.\n",pages.len()));
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }
        for event in ctx.input(|i| i.events.clone()) {
            if let egui::Event::Screenshot { image, .. } = event {
                if !self.smoke_waiting || image.width() == 0 || image.height() == 0 {
                    continue;
                }
                let bytes: Vec<u8> = image.pixels.iter().flat_map(|p| p.to_array()).collect();
                if let Err(e) = image::save_buffer(
                    format!("dist/native-tools-{}.png", pages[self.smoke_stage].1),
                    &bytes,
                    image.width() as u32,
                    image.height() as u32,
                    image::ColorType::Rgba8,
                ) {
                    let _ = std::fs::write("dist/tools-ui-smoke-result.txt", format!("FAIL: {e}"));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    return;
                }
                self.smoke_stage += 1;
                self.smoke_waiting = false;
                self.tools_ui.smoke_prepared = false;
                self.smoke_at = Instant::now();
                return;
            }
        }
        if !self.tools_ui.smoke_prepared {
            let (view, _, width) = pages[self.smoke_stage];
            self.view = view;
            self.tools_ui.last_view = match view {
                View::Tool(tool) => Some(tool),
                _ => None,
            };
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(Vec2::new(
                width,
                if width == 1000. { 700. } else { 960. },
            )));
            self.devices = audio_io::Devices {
                inputs: vec![audio_io::DeviceInfo {
                    id: "fixture-in".into(),
                    name: "Reference microphone".into(),
                }],
                outputs: vec![audio_io::DeviceInfo {
                    id: "fixture-out".into(),
                    name: "Headphone reference output".into(),
                }],
            };
            self.tools_ui.devices = vec![
                audio_io::EndpointReport {
                    id: "fixture-in".into(),
                    name: "Reference microphone".into(),
                    output: false,
                    default_name_match: true,
                    rate: 48000,
                    channels: 2,
                    format: "F32".into(),
                    buffer: "Driver-managed shared-mode format".into(),
                    supported: vec!["48000 Hz · 2 channels · F32".into()],
                    error: String::new(),
                },
                audio_io::EndpointReport {
                    id: "fixture-out".into(),
                    name: "Headphone reference output".into(),
                    output: true,
                    default_name_match: true,
                    rate: 44100,
                    channels: 2,
                    format: "F32".into(),
                    buffer: "Driver-managed shared-mode format".into(),
                    supported: vec!["44100 Hz · 2 channels · F32".into()],
                    error: String::new(),
                },
            ];
            self.tools_ui.scanned = true;
            let mut a = workbench::analysis::Analyzer::new(48000, 2048);
            for i in 0..48000 {
                let value = if i < 24000 { 0.15 } else { 0.001 }
                    * (i as f32 * std::f32::consts::TAU * 750. / 48000.).sin();
                a.push([value, -value]);
            }
            let bins = a.spectrum();
            self.tools_ui.snapshot = Snapshot {
                reading: a.reading(),
                spectrum: bins.clone(),
                waveform: a.waveform(),
                input_rate: 48000,
                output_rate: 48000,
                message: "Generated preview fixture · no microphone or output opened.".into(),
                ..Snapshot::default()
            };
            self.tools_ui.shown = bins.clone();
            self.tools_ui.held = bins;
            self.tools_ui.smoke_prepared = true;
            self.smoke_at = Instant::now();
            return;
        }
        if !self.smoke_waiting && self.smoke_at.elapsed() > Duration::from_millis(220) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(Default::default()));
            self.smoke_waiting = true;
        }
    }
    pub(super) fn tools_page(&mut self, ui: &mut egui::Ui) {
        ui.heading("Your audio workbench.");
        ui.label("Independent tools for checking a setup, understanding a signal and creating controller artwork.");
        ui.small("Tools do not alter your project or start audio until you ask. Leaving a tool stops its test streams.");
        ui.add_space(12.);
        ui.horizontal_wrapped(|ui| {
            ui.label("Find a tool");
            ui.add(
                egui::TextEdit::singleline(&mut self.tools_ui.filter)
                    .hint_text("Name or purpose")
                    .desired_width(240.),
            );
            for category in ["All", "Microphone", "Setup", "Reference", "Controller"] {
                ui.selectable_value(&mut self.tools_ui.category, category.into(), category);
            }
        });
        ui.add_space(12.);
        let filter = self.tools_ui.filter.to_lowercase();
        let tools: Vec<Tool> = Tool::ALL
            .into_iter()
            .filter(|t| {
                (self.tools_ui.category == "All" || t.category() == self.tools_ui.category)
                    && format!("{} {}", t.name(), t.description())
                        .to_lowercase()
                        .contains(&filter)
            })
            .collect();
        let columns = ((ui.available_width() / 300.).floor() as usize).clamp(1, 3);
        ui.small(format!("{} of {} tools", tools.len(), Tool::ALL.len()));
        for row in tools.chunks(columns) {
            ui.columns(columns, |columns| {
                for (tool, column) in row.iter().zip(columns) {
                    card().show(column, |ui| {
                        ui.set_max_width(ui.available_width());
                        ui.set_min_height(155.);
                        caption(ui, tool.category());
                        ui.heading(tool.name());
                        ui.add_sized(
                            [ui.available_width(), 55.],
                            egui::Label::new(tool.description()).wrap(),
                        );
                        if ui.button(format!("Open {}", tool.name())).clicked() {
                            self.tools_ui.stop();
                            self.view = View::Tool(*tool);
                            self.tools_ui.message.clear();
                        }
                    });
                }
            });
        }
        if tools.is_empty() {
            ui.label("No tools match. Clear the search or choose All.");
        }
    }
    pub(super) fn tool_page(&mut self, ui: &mut egui::Ui, tool: Tool) {
        self.tools_ui.poll(Some(tool));
        ui.horizontal_wrapped(|ui| {
            if ui.button("All tools").clicked() {
                self.tools_ui.stop();
                self.view = View::Tools;
            }
            ui.heading(tool.name());
            ui.small(tool.category());
        });
        ui.label(tool.description());
        ui.add_space(12.);
        if matches!(
            tool,
            Tool::DeviceAtlas | Tool::ClockScope | Tool::SoundCheck
        ) && !self.tools_ui.scanned
            && self.tools_ui.scan.is_none()
            && !self.smoke
        {
            self.tools_ui.scan();
        }
        match tool {
            Tool::BufferBudget => self.buffer_tool(ui),
            Tool::VocalMap => self.eq_map_tool(ui),
            Tool::LevelGuide => self.level_tool(ui),
            Tool::DeviceAtlas | Tool::ClockScope => {
                self.endpoint_tool(ui, tool == Tool::ClockScope)
            }
            Tool::KeyCanvas => self.icon_tool(ui),
            Tool::SoundCheck => {
                ui.collapsing("1 · Endpoint overview", |ui| self.endpoint_tool(ui, true));
                self.audio_tool(ui, tool);
                egui::CollapsingHeader::new("Completed checks")
                    .default_open(true)
                    .show(ui, |ui| {
                        for (mode, label) in [
                            (Mode::Meter, "Microphone"),
                            (Mode::Quiet, "Quiet floor"),
                            (Mode::Latency, "Return timing"),
                        ] {
                            if let Some(result) = self.tools_ui.suite.get(&mode) {
                                ui.strong(label);
                                if mode == Mode::Latency {
                                    if let Some(ms) = result.latency_ms {
                                        ui.label(format!("{ms:.0} ms"));
                                    }
                                    ui.small(&result.message);
                                } else {
                                    reading_panel(ui, &result.reading);
                                }
                            }
                        }
                    });
            }
            _ => self.audio_tool(ui, tool),
        }
        if !self.tools_ui.message.is_empty() {
            ui.add_space(8.);
            ui.colored_label(theme::palette().accent, &self.tools_ui.message);
        }
    }
    fn audio_tool(&mut self, ui: &mut egui::Ui, tool: Tool) {
        let output = matches!(
            tool,
            Tool::ToneForge
                | Tool::ChannelCompass
                | Tool::ListenBack
                | Tool::VoiceSandbox
                | Tool::EchoClock
                | Tool::SoundCheck
        );
        let busy = self.tools_ui.busy();
        let choices = |output: bool, fallback: &Vec<audio_io::DeviceInfo>| {
            if self.tools_ui.scanned {
                self.tools_ui
                    .devices
                    .iter()
                    .filter(|d| d.output == output && !d.id.is_empty())
                    .map(|d| audio_io::DeviceInfo {
                        id: d.id.clone(),
                        name: d.name.clone(),
                    })
                    .collect::<Vec<_>>()
            } else {
                fallback.clone()
            }
        };
        let inputs = choices(false, &self.devices.inputs);
        let outputs = choices(true, &self.devices.outputs);
        card().show(ui, |ui| {
            ui.add_enabled_ui(!busy, |ui| {
                if tool.input() {
                    device_picker(ui, "Tool input", &mut self.tools_ui.config.input, &inputs);
                }
                if output {
                    device_picker(
                        ui,
                        "Tool output",
                        &mut self.tools_ui.config.output,
                        &outputs,
                    );
                }
            });
            ui.small("Device selections belong to this workbench, not your mixer routes.");
            if ui
                .add_enabled(
                    self.tools_ui.scan.is_none(),
                    egui::Button::new("Refresh device choices"),
                )
                .clicked()
                && !self.smoke
            {
                self.tools_ui.scan();
            }
        });
        ui.add_space(8.);
        if matches!(tool, Tool::ListenBack | Tool::VoiceSandbox) {
            ui.colored_label(
                theme::palette().peach,
                "Use headphones for live monitoring to prevent a feedback loop.",
            );
            if ui
                .checkbox(
                    &mut self.tools_ui.headphone_confirmed,
                    "I am using headphones",
                )
                .changed()
                && !self.tools_ui.headphone_confirmed
            {
                self.tools_ui.stop();
            }
            if tool == Tool::VoiceSandbox {
                self.voice_sandbox(ui);
            }
        }
        if tool == Tool::ToneForge {
            ui.colored_label(theme::palette().peach,"Start with a low speaker/headphone volume. Digital level does not set room loudness.");
            let mut changed = false;
            changed |= ui
                .add(
                    egui::Slider::new(&mut self.tools_ui.config.frequency, 20.0..=20000.0)
                        .logarithmic(true)
                        .text("Frequency · Hz"),
                )
                .changed();
            ui.horizontal_wrapped(|ui| {
                for frequency in [60., 100., 440., 1000., 10000., 15000.] {
                    if ui.button(format!("{frequency:.0} Hz")).clicked() {
                        self.tools_ui.config.frequency = frequency;
                        changed = true;
                    }
                }
            });
            ui.horizontal_wrapped(|ui| {
                for wave in Wave::ALL {
                    changed |= ui
                        .selectable_value(&mut self.tools_ui.config.wave, wave, wave.name())
                        .changed();
                }
            });
            if changed {
                self.tools_ui.update();
            }
        }
        if output
            && !matches!(tool, Tool::EchoClock | Tool::SoundCheck)
            && ui
                .add(
                    egui::Slider::new(&mut self.tools_ui.config.level_db, -60.0..=-6.0).text(
                        if matches!(tool, Tool::ListenBack | Tool::VoiceSandbox) {
                            "Monitor gain · dB"
                        } else {
                            "Tool output · dBFS peak"
                        },
                    ),
                )
                .changed()
        {
            self.tools_ui.update();
        }
        if tool == Tool::SpectralLens {
            ui.horizontal_wrapped(|ui| {
                ui.label("FFT window");
                for size in [2048, 8192] {
                    if ui
                        .selectable_value(
                            &mut self.tools_ui.config.fft_size,
                            size,
                            size.to_string(),
                        )
                        .changed()
                    {
                        self.tools_ui.held.clear();
                        self.tools_ui.shown.clear();
                        self.tools_ui.frozen = false;
                        self.tools_ui.update();
                    }
                }
                if ui
                    .toggle_value(&mut self.tools_ui.frozen, "Freeze plot")
                    .changed()
                    && !self.tools_ui.frozen
                {
                    self.tools_ui.shown = self.tools_ui.snapshot.spectrum.clone();
                }
                if ui.button("Reset peak trace").clicked() {
                    self.tools_ui.held.fill(-120.);
                }
            });
        }
        if matches!(tool, Tool::EchoClock | Tool::SoundCheck) {
            ui.label("Latency test: use a speaker that the selected mic can hear. Pause other audio and monitoring paths and keep the room quiet. This tool does not monitor its input during the test.");
            ui.small("One 25 ms chirp at approximately −18 dBFS. The result includes this tool's output queue, Windows path, air and input delivery; it is not the mixer graph's latency.");
        }
        ui.horizontal_wrapped(|ui| {
            let enabled = !busy
                && (!matches!(tool, Tool::ListenBack | Tool::VoiceSandbox)
                    || self.tools_ui.headphone_confirmed);
            ui.add_enabled_ui(enabled, |ui| {
                match tool {
                    Tool::SignalCheck | Tool::SoundCheck => {
                        if ui.button("Run 5-second mic check").clicked() {
                            self.tools_ui.config.seconds = 5.;
                            self.tools_ui.start(tool, Mode::Meter, self.smoke);
                        }
                        if tool == Tool::SignalCheck && ui.button("Start live meter").clicked() {
                            self.tools_ui.start(tool, Mode::Spectrum, self.smoke);
                        }
                    }
                    Tool::RoomHush => {
                        if ui.button("Measure 5 seconds of quiet").clicked() {
                            self.tools_ui.config.seconds = 5.;
                            self.tools_ui.start(tool, Mode::Quiet, self.smoke);
                        }
                    }
                    Tool::SpectralLens => {
                        if ui.button("Start spectrum").clicked() {
                            self.tools_ui.start(tool, Mode::Spectrum, self.smoke);
                        }
                    }
                    Tool::ListenBack | Tool::VoiceSandbox => {
                        if ui.button("Start headphone preview").clicked() {
                            self.tools_ui.start(
                                tool,
                                if tool == Tool::ListenBack {
                                    Mode::Monitor
                                } else {
                                    Mode::Voice
                                },
                                self.smoke,
                            );
                        }
                    }
                    Tool::ToneForge if ui.button("Play test tone").clicked() => {
                        self.tools_ui.config.channel = 0;
                        self.tools_ui.start(tool, Mode::Tone, self.smoke);
                    }
                    _ => {}
                }
                if matches!(tool, Tool::EchoClock | Tool::SoundCheck)
                    && ui.button("Measure return time").clicked()
                {
                    self.tools_ui.config.seconds = 4.;
                    self.tools_ui.start(tool, Mode::Latency, self.smoke);
                }
                if tool == Tool::SoundCheck && ui.button("Measure quiet floor").clicked() {
                    self.tools_ui.config.seconds = 5.;
                    self.tools_ui.start(tool, Mode::Quiet, self.smoke);
                }
            });
            if ui
                .add_enabled(busy, egui::Button::new("Stop tool audio"))
                .clicked()
            {
                self.tools_ui.stop();
            }
        });
        if tool == Tool::ChannelCompass {
            ui.colored_label(theme::palette().peach,"Lower system volume before testing. Polarity checks are mainly useful with speakers; headphones do not mix in the room.");
            ui.add_enabled_ui(!busy, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for (label, channel, frequency, seconds) in [
                        ("Left only", 1, 440., 1.5),
                        ("Center · both", 0, 440., 1.5),
                        ("Right only", 2, 440., 1.5),
                        ("Matched polarity", 0, 200., 4.),
                        ("Opposite polarity", 3, 200., 4.),
                    ] {
                        if ui.button(label).clicked() {
                            self.tools_ui.config.channel = channel;
                            self.tools_ui.config.frequency = frequency;
                            self.tools_ui.config.wave = Wave::Sine;
                            self.tools_ui.config.seconds = seconds;
                            self.tools_ui.start(tool, Mode::Stereo, self.smoke);
                        }
                    }
                })
            });
            ui.small("The unused channel is digital silence. For speakers, compare the strength of the bass between the two polarity tests; reflections and speaker placement also affect the result.");
        }
        ui.add_space(8.);
        let snapshot = &self.tools_ui.snapshot;
        card().show(ui,|ui|{
            ui.colored_label(if snapshot.message.starts_with("Tool stopped:"){theme::palette().peach}else{theme::palette().accent},&snapshot.message);
            if busy {ui.label(format!("{:.1} seconds · tool streams active",snapshot.elapsed));}
            if snapshot.input_rate>0||snapshot.output_rate>0{ui.small(format!("Input: {} Hz · output: {} Hz · analysis follows input · PirateWave DSP: {} Hz",snapshot.input_rate,snapshot.output_rate,piratewave_core::SAMPLE_RATE));}
            if snapshot.output_channels==1 {ui.colored_label(theme::palette().peach,"This output is mono. Left and right are folded together, so use a stereo output to test channel placement or polarity.");}
            if snapshot.dropped>0{ui.colored_label(theme::palette().peach,format!("{} samples dropped: repeat timed measurements after reducing system load.",snapshot.dropped));}
            if snapshot.underruns>0{ui.colored_label(theme::palette().peach,format!("{} output underrun callbacks. If playback stutters, stop other test streams or reduce load and repeat.",snapshot.underruns));}
            if tool.input(){reading_panel(ui,&snapshot.reading);waveform(ui,&snapshot.waveform);}
            if let Some(ms)=snapshot.latency_ms{ui.heading(format!("{ms:.0} ms tool round trip"));ui.label(format!("Chirp correlation: {:.0}% · callback/worker timing, not sub-millisecond hardware certification.",snapshot.confidence*100.));}
        });
        if tool == Tool::SpectralLens {
            spectrum(
                ui,
                &self.tools_ui.shown,
                &self.tools_ui.held,
                self.tools_ui.snapshot.input_rate,
                self.tools_ui.config.fft_size,
            );
        }
        ui.collapsing("Method & limitations",|ui|{
            ui.label("Input analysis bypasses project faders and effects. Windows or interface DSP may still be active. Stereo levels use both channels; the spectrum takes the stronger channel at each frequency, preserving opposite-polarity content.");
            ui.label("Noise floor averages the quietest quarter of 20 ms RMS windows, up to 30 seconds. The peak-to-floor span is not a calibrated SNR or acoustic sound-pressure measurement. A quiet-only test cannot tell how far speech rises above it.");
            ui.label("The Hann-windowed spectrum is amplitude-corrected for a bin-centered sine. A signal between bins spreads across them. Test streams stop when you leave this tool and after ten minutes at most. No audio is saved or uploaded.");
        });
    }
    fn buffer_tool(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.tools_ui;
        card().show(ui,|ui|{
            ui.horizontal_wrapped(|ui|{ui.label("Sample rate");for rate in [44100,48000,88200,96000,192000]{ui.selectable_value(&mut s.rate,rate,format!("{} kHz",rate as f32/1000.));}});
            ui.horizontal_wrapped(|ui|{ui.label("Buffer frames");ui.add(egui::DragValue::new(&mut s.buffer).range(1..=65536));ui.label("Stages");ui.add(egui::DragValue::new(&mut s.stages).range(1..=16));});
            ui.heading(format!("{:.3} ms per buffer",workbench::buffer_ms(s.buffer,s.rate)));
            ui.label(format!("{:.3} ms across {} buffer stages",workbench::buffer_ms(s.buffer,s.rate)*s.stages as f64,s.stages));
            ui.small("Arithmetic only: frames ÷ sample rate × 1000. Driver, converter, resampling and effect delays are additional. This does not change your device buffer.");
        });
        egui::Grid::new("buffer_presets")
            .striped(true)
            .min_col_width(120.)
            .show(ui, |ui| {
                ui.strong("Frames");
                ui.strong("One stage");
                ui.strong("Selected stages");
                ui.end_row();
                for frames in [16, 32, 64, 128, 256, 512, 1024, 2048] {
                    if ui
                        .selectable_label(s.buffer == frames, frames.to_string())
                        .clicked()
                    {
                        s.buffer = frames;
                    }
                    ui.label(format!("{:.3} ms", workbench::buffer_ms(frames, s.rate)));
                    ui.label(format!(
                        "{:.3} ms",
                        workbench::buffer_ms(frames, s.rate) * s.stages as f64
                    ));
                    ui.end_row();
                }
            });
    }
    fn endpoint_tool(&mut self, ui: &mut egui::Ui, rates: bool) {
        let s = &mut self.tools_ui;
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    s.scan.is_none(),
                    egui::Button::new("Refresh endpoint details"),
                )
                .clicked()
                && !self.smoke
            {
                s.scan();
            }
            ui.add(
                egui::TextEdit::singleline(&mut s.endpoint_filter)
                    .hint_text("Filter devices")
                    .desired_width(220.),
            );
        });
        if s.scan.is_some() {
            ui.spinner();
            ui.label("Reading native endpoint formats…");
        }
        if rates {
            ui.label(format!("PirateWave's processing clock is fixed at {} Hz. Endpoints can use a different shared-mode format; the engine resamples between them.",piratewave_core::SAMPLE_RATE));
        }
        ui.small("Reported formats are the Windows shared-mode defaults and advertised capabilities, not a claim about an exclusive/ASIO session. Default indicators match endpoint names; identical names can be ambiguous.");
        let filter = s.endpoint_filter.to_lowercase();
        for endpoint in s
            .devices
            .iter()
            .filter(|d| d.name.to_lowercase().contains(&filter))
        {
            card().show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.strong(&endpoint.name);
                    ui.small(if endpoint.output { "OUTPUT" } else { "INPUT" });
                    if endpoint.default_name_match {
                        ui.colored_label(theme::palette().accent, "Default name match");
                    }
                });
                if !endpoint.error.is_empty() {
                    ui.colored_label(theme::palette().peach, &endpoint.error);
                } else {
                    ui.label(format!(
                        "{} Hz · {} channel(s) · {}",
                        endpoint.rate, endpoint.channels, endpoint.format
                    ));
                    ui.small(&endpoint.buffer);
                    if rates && endpoint.rate != piratewave_core::SAMPLE_RATE {
                        ui.small(
                            "Different from PirateWave's processing rate; resampling is expected.",
                        );
                    }
                }
                ui.collapsing("Identity & supported formats", |ui| {
                    ui.monospace(&endpoint.id);
                    for format in &endpoint.supported {
                        ui.small(format);
                    }
                });
            });
        }
        if s.devices.is_empty() && s.scan.is_none() {
            ui.label("No endpoint details loaded. Refresh to inspect available hardware and virtual devices.");
        }
    }
    fn eq_map_tool(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.tools_ui;
        ui.horizontal_wrapped(|ui| {
            for (i, (name, ..)) in workbench::EQ_REGIONS.iter().enumerate() {
                ui.selectable_value(&mut s.region, i, *name);
            }
        });
        let (name, min, max, meaning, move_hint) = workbench::EQ_REGIONS[s.region];
        card().show(ui, |ui| {
            ui.heading(format!("{name} · {min:.0}–{max:.0} Hz"));
            ui.label(meaning);
            ui.label(move_hint);
            ui.small(
                "Overlapping regions are starting points, not rules for every voice or microphone.",
            );
        });
        let center = (min * max).sqrt();
        ui.add(egui::Slider::new(&mut s.eq_gain, -12.0..=12.0).text("Preview bell gain · dB"));
        ui.add(
            egui::Slider::new(&mut s.eq_q, 0.2..=10.0)
                .logarithmic(true)
                .text("Preview Q"),
        );
        let (rect, _) = ui.allocate_exact_size(
            Vec2::new(ui.available_width().max(100.), 220.),
            egui::Sense::hover(),
        );
        let p = ui.painter_at(rect);
        p.rect_filled(rect, 6., theme::palette().panel);
        p.rect_filled(
            Rect::from_min_max(
                Pos2::new(frequency_x(rect, min), rect.top()),
                Pos2::new(frequency_x(rect, max), rect.bottom() - 25.),
            ),
            0.,
            theme::palette().accent.gamma_multiply(22. / 255.),
        );
        for hz in [20_f32, 100., 1000., 10000., 20000.] {
            let x = frequency_x(rect, hz);
            p.line_segment(
                [
                    Pos2::new(x, rect.top() + 10.),
                    Pos2::new(x, rect.bottom() - 25.),
                ],
                Stroke::new(1_f32, theme::palette().line),
            );
            p.text(
                Pos2::new(x, rect.bottom() - 15.),
                if hz == 20. {
                    Align2::LEFT_CENTER
                } else if hz == 20000. {
                    Align2::RIGHT_CENTER
                } else {
                    Align2::CENTER_CENTER
                },
                format!("{hz:.0}"),
                FontId::proportional(11.),
                theme::palette().muted,
            );
        }
        let points: Vec<Pos2> = (0..400)
            .map(|i| {
                let hz = 20. * 1000_f32.powf(i as f32 / 399.);
                let gain = bell_response(hz, center, s.eq_gain, s.eq_q);
                Pos2::new(
                    egui::lerp(rect.x_range(), i as f32 / 399.),
                    rect.center().y - gain * 6.,
                )
            })
            .collect();
        p.add(egui::Shape::line(
            points,
            Stroke::new(2_f32, theme::palette().accent),
        ));
        ui.small(format!("Real RBJ bell response at {center:.0} Hz; this preview does not change any rack. Small moves with level-matched comparisons are easier to judge."));
    }
    fn level_tool(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.tools_ui;
        card().show(ui,|ui|{ui.heading("Digital headroom");ui.add(egui::Slider::new(&mut s.digital_db,-120.0..=0.0).text("dBFS · peak amplitude"));let amplitude=10_f32.powf(s.digital_db/20.);ui.label(format!("{:.6} × full-scale amplitude · {:.3}%",amplitude,amplitude*100.));ui.small("20 log10(amplitude). A −6.02 dB change halves amplitude. RMS, LUFS and peak readings describe different properties.");});
        ui.horizontal_wrapped(|ui| {
            for (db, label) in [
                (0., "Full scale"),
                (-1., "Ceiling margin"),
                (-12., "Speech peak example"),
                (-18., "Average-level convention"),
                (-60., "Low digital floor"),
            ] {
                if ui.button(format!("{db:.0} · {label}")).clicked() {
                    s.digital_db = db;
                }
            }
        });
        ui.add_space(12.);
        let acoustic = [
            (
                "Quiet room",
                30,
                "A rough environmental example; distance and surroundings change the result.",
            ),
            (
                "Conversation nearby",
                60,
                "A general sound-pressure reference, not a digital recording target.",
            ),
            (
                "Busy roadway",
                80,
                "An environmental comparison, not a reading from your microphone.",
            ),
            (
                "Loud music venue",
                105,
                "Actual sound pressure depends strongly on location and the sound system.",
            ),
        ];
        card().show(ui,|ui|{ui.heading("Acoustic landmarks · dB SPL");for (i,(label,level,_)) in acoustic.iter().enumerate(){ui.selectable_value(&mut s.acoustic,i,format!("~{level} dB SPL · {label}"));}ui.label(acoustic[s.acoustic].2);ui.colored_label(theme::palette().peach,"There is no fixed conversion from dBFS to dB SPL. Calibration, microphone sensitivity, gain and distance are required.");});
    }
    fn voice_sandbox(&mut self, ui: &mut egui::Ui) {
        let presets = [
            ("Uncolored", None),
            ("Copper Relay", Some("vocoder")),
            ("Pocket Broadcast", Some("distortion")),
            ("Deep Orbit", Some("pitch")),
            ("Glass Hall", Some("reverb")),
            ("Slow Tide", Some("chorus")),
            ("Signal Flare", Some("tremolo")),
            ("Pulse Echo", Some("delay")),
        ];
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for (index, (name, _)) in presets.iter().enumerate() {
                if ui
                    .selectable_value(&mut self.tools_ui.voice_preset, index, *name)
                    .changed()
                {
                    self.tools_ui.config.effects = presets[index]
                        .1
                        .and_then(|id| self.registry.get(id))
                        .map(|factory| vec![EffectSpec::new(factory)])
                        .unwrap_or_default();
                    if let Some(pitch) = self
                        .tools_ui
                        .config
                        .effects
                        .iter_mut()
                        .find(|e| e.kind == "pitch")
                    {
                        pitch.parameters.insert("semitones".into(), -5.);
                    }
                    changed = true;
                }
            }
        });
        for spec in &mut self.tools_ui.config.effects {
            if let Some(factory) = self.registry.get(&spec.kind) {
                ui.strong(factory.name);
                for parameter in factory.parameters {
                    let value = spec
                        .parameters
                        .entry(parameter.id.into())
                        .or_insert(parameter.default);
                    if let Some(options) =
                        piratewave_core::voice_effects::choices(&spec.kind, parameter.id)
                    {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(parameter.label);
                            for (i, label) in options.iter().enumerate() {
                                changed |= ui.selectable_value(value, i as f32, *label).changed();
                            }
                        });
                    } else {
                        changed |= ui
                            .add(
                                egui::Slider::new(value, parameter.min..=parameter.max)
                                    .text(format!("{} · {}", parameter.label, parameter.unit)),
                            )
                            .changed();
                    }
                }
            }
        }
        if changed {
            self.tools_ui.update();
        }
        ui.small("These are independent DSP auditions. Pitch changes do not preserve formants; no AI model or voice cloning is used. Nothing is added to your project.");
    }
    fn icon_tool(&mut self, ui: &mut egui::Ui) {
        let s = &mut self.tools_ui;
        ui.horizontal_wrapped(|ui| {
            for glyph in Glyph::ALL {
                ui.selectable_value(&mut s.design.glyph, glyph, glyph.name());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Label");
            ui.add(
                egui::TextEdit::singleline(&mut s.design.label)
                    .char_limit(24)
                    .desired_width(220.),
            );
            ui.label("Export size");
            for size in [144, 288, 576] {
                ui.selectable_value(&mut s.design.size, size, format!("{size} px"));
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label("Background");
            ui.color_edit_button_srgba_unmultiplied(&mut s.design.background);
            ui.label("Symbol");
            ui.color_edit_button_srgba_unmultiplied(&mut s.design.foreground);
            ui.label("Label color");
            ui.color_edit_button_srgba_unmultiplied(&mut s.design.label_color);
        });
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut s.design.gradient, "Gradient");
            ui.checkbox(&mut s.design.rounded, "Rounded corners");
            ui.checkbox(&mut s.design.transparent, "Transparent background");
        });
        if s.rendered.as_ref() != Some(&s.design) {
            let mut preview = s.design.clone();
            preview.size = 288;
            match icons::render(&preview) {
                Ok(image) => {
                    let color =
                        egui::ColorImage::from_rgba_unmultiplied([288, 288], image.as_raw());
                    s.preview = Some(ui.ctx().load_texture(
                        "key_canvas_preview",
                        color,
                        Default::default(),
                    ));
                    s.rendered = Some(s.design.clone());
                }
                Err(e) => s.message = e,
            }
        }
        if let Some(texture) = &s.preview {
            ui.add(egui::Image::new(texture).fit_to_exact_size(Vec2::splat(230.)));
        }
        ui.small("Preview uses the same renderer as export. Matching sets include ten original action symbols with shared colors, background and size.");
        ui.horizontal_wrapped(|ui| {
            ui.add_enabled_ui(s.export.is_none(), |ui| {
                if ui.button("Save icon PNG…").clicked() && !self.smoke {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("PNG image", &["png"])
                        .set_file_name(format!(
                            "piratewave-{}.png",
                            s.design.glyph.name().to_lowercase()
                        ))
                        .save_file()
                    {
                        s.export = Some(export_icons(s.design.clone(), path, false));
                    }
                }
                if ui.button("Save matching set ZIP…").clicked() && !self.smoke {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("ZIP archive", &["zip"])
                        .set_file_name("piratewave-controller-icons.zip")
                        .save_file()
                    {
                        s.export = Some(export_icons(s.design.clone(), path, true));
                    }
                }
            });
            if s.export.is_some() {
                ui.spinner();
                ui.label("Rendering export…");
            }
        });
        ui.small("Assign exported icons explicitly in Stream Deck or your controller software. Existing profiles are not modified.");
    }
}

fn device_picker(
    ui: &mut egui::Ui,
    label: &str,
    selected: &mut Option<String>,
    devices: &[audio_io::DeviceInfo],
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(label);
        egui::ComboBox::from_id_salt(label)
            .width(300_f32.min(ui.available_width().max(100.)))
            .selected_text(
                selected
                    .as_ref()
                    .and_then(|id| devices.iter().find(|d| &d.id == id))
                    .map_or(
                        if selected.is_some() {
                            "Unavailable device"
                        } else {
                            "Windows default"
                        },
                        |d| d.name.as_str(),
                    ),
            )
            .show_ui(ui, |ui| {
                ui.selectable_value(selected, None, "Windows default");
                for device in devices {
                    ui.selectable_value(selected, Some(device.id.clone()), &device.name);
                }
            });
    });
}
fn reading_panel(ui: &mut egui::Ui, r: &Reading) {
    if r.frames == 0 {
        ui.small("No measurements yet. Start a test to read actual input samples.");
        return;
    }
    ui.horizontal_wrapped(|ui| {
        for (label, value) in [
            ("Peak", r.peak_db),
            ("Quiet floor", r.floor_db),
            ("Peak-to-floor span", r.separation_db),
        ] {
            ui.vertical(|ui| {
                caption(ui, label);
                ui.strong(format!(
                    "{value:.1} dB{}",
                    if label == "Peak-to-floor span" {
                        ""
                    } else {
                        "FS"
                    }
                ));
            });
        }
    });
    ui.horizontal_wrapped(|ui| {
        for ch in 0..2 {
            let db = workbench::db(r.rms[ch] as f64);
            ui.label(if ch == 0 { "L" } else { "R" });
            ui.add(
                egui::ProgressBar::new(((db + 72.) / 72.).clamp(0., 1.))
                    .desired_width(150.)
                    .text(format!("{db:.1} dBFS RMS")),
            );
        }
    });
    ui.label(format!(
        "{} clipped frames · {:.2}% of samples · observed peaks L {:.1} / R {:.1} dBFS",
        r.clipped,
        r.clipped as f64 / r.frames.max(1) as f64 * 100.,
        workbench::db(r.peak[0] as f64),
        workbench::db(r.peak[1] as f64)
    ));
    ui.small(format!(
        "Recent spectral energy: low {:.0}% · mid {:.0}% · high {:.0}%",
        r.bands[0] * 100.,
        r.bands[1] * 100.,
        r.bands[2] * 100.
    ));
    if r.clipped > 0 {
        ui.colored_label(
            theme::palette().peach,
            "Clipping was observed. Lower gain before this capture path and repeat.",
        );
    }
}
fn waveform(ui: &mut egui::Ui, values: &[f32]) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(100.), 70.),
        egui::Sense::hover(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 5., theme::palette().bg);
    painter.line_segment(
        [rect.left_center(), rect.right_center()],
        Stroke::new(1_f32, theme::palette().line),
    );
    if values.len() > 1 {
        painter.add(egui::Shape::line(
            values
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    Pos2::new(
                        rect.left() + i as f32 / (values.len() - 1) as f32 * rect.width(),
                        rect.center().y - v.clamp(-1., 1.) * rect.height() * 0.45,
                    )
                })
                .collect(),
            Stroke::new(1.5_f32, theme::palette().accent),
        ));
    }
}
fn frequency_x(rect: Rect, hz: f32) -> f32 {
    rect.left() + ((hz / 20.).log10() / 3.).clamp(0., 1.) * rect.width()
}
fn spectrum(ui: &mut egui::Ui, values: &[f32], held: &[f32], rate: u32, size: usize) {
    let (rect, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(100.), 260.),
        egui::Sense::hover(),
    );
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 5., theme::palette().panel);
    for hz in [20_f32, 100., 1000., 10000., 20000.] {
        let x = frequency_x(rect, hz);
        p.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom() - 25.)],
            Stroke::new(1_f32, theme::palette().line),
        );
        p.text(
            Pos2::new(x, rect.bottom() - 15.),
            if hz == 20. {
                Align2::LEFT_CENTER
            } else if hz == 20000. {
                Align2::RIGHT_CENTER
            } else {
                Align2::CENTER_CENTER
            },
            format!("{hz:.0}"),
            FontId::proportional(11.),
            theme::palette().muted,
        );
    }
    for db in [-120, -90, -60, -30, 0] {
        let y = rect.bottom() - 25. - (db as f32 + 120.) / 120. * (rect.height() - 35.);
        p.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1_f32, theme::palette().line),
        );
        p.text(
            Pos2::new(rect.left() + 4., y),
            Align2::LEFT_TOP,
            db.to_string(),
            FontId::proportional(10.),
            theme::palette().muted,
        );
    }
    if rate > 0 && values.len() == size / 2 + 1 {
        for (bins, color, width) in [
            (held, theme::palette().muted, 1_f32),
            (values, theme::palette().accent, 2_f32),
        ] {
            let points: Vec<Pos2> = bins
                .iter()
                .enumerate()
                .filter_map(|(i, v)| {
                    let hz = i as f32 * rate as f32 / size as f32;
                    (20. ..=20000.).contains(&hz).then(|| {
                        Pos2::new(
                            frequency_x(rect, hz),
                            rect.bottom()
                                - 25.
                                - (v.clamp(-120., 0.) + 120.) / 120. * (rect.height() - 35.),
                        )
                    })
                })
                .collect();
            if points.len() > 1 {
                p.add(egui::Shape::line(points, Stroke::new(width, color)));
            }
        }
        if let Some(pos) = response.hover_pos() {
            let hz = 20. * 1000_f32.powf(((pos.x - rect.left()) / rect.width()).clamp(0., 1.));
            let index = (hz * size as f32 / rate as f32).round() as usize;
            if let Some(value) = values.get(index) {
                response.on_hover_text(format!("{hz:.0} Hz · {value:.1} dBFS"));
            }
        }
    }
    ui.small(format!(
        "Purple: live · gray: peak hold · {} Hz Nyquist · {:.2} Hz/bin · {:.1} ms/window",
        rate / 2,
        rate as f32 / size as f32,
        if rate > 0 {
            size as f32 / rate as f32 * 1000.
        } else {
            0.
        }
    ));
}

fn export_icons(
    design: Design,
    path: PathBuf,
    set: bool,
) -> mpsc::Receiver<Result<String, String>> {
    let (tx, rx) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let result = (|| -> Result<String, String> {
            if set {
                use std::io::Write;
                let file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
                let mut zip = zip::ZipWriter::new(file);
                for glyph in Glyph::ALL {
                    let d = Design {
                        glyph,
                        label: glyph.name().to_uppercase(),
                        ..design.clone()
                    };
                    let image = icons::render(&d)?;
                    let mut png = std::io::Cursor::new(Vec::new());
                    image
                        .write_to(&mut png, image::ImageFormat::Png)
                        .map_err(|e| e.to_string())?;
                    zip.start_file(
                        format!("piratewave-{}.png", glyph.name().to_lowercase()),
                        zip::write::FileOptions::default()
                            .compression_method(zip::CompressionMethod::Deflated),
                    )
                    .map_err(|e| e.to_string())?;
                    zip.write_all(&png.into_inner())
                        .map_err(|e| e.to_string())?;
                }
                zip.finish().map_err(|e| e.to_string())?;
            } else {
                icons::render(&design)?
                    .save_with_format(&path, image::ImageFormat::Png)
                    .map_err(|e| e.to_string())?;
            }
            Ok(format!("Saved {}", path.display()))
        })();
        let _ = tx.send(result);
    });
    rx
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::tests::{app, at, click, labels, render};
    #[test]
    fn gallery_navigation_search_and_all_workspaces_leave_project_and_engine_untouched() {
        let ctx = egui::Context::default();
        let mut app = app();
        let (engine, rx) = Engine::test_channel();
        app.engine = engine;
        let before = serde_json::to_string(&app.project).unwrap();
        app.view = View::Tools;
        let _ = render(&ctx, &mut app, vec![]);
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "Open Signal Check"),
            egui::PointerButton::Primary,
        );
        assert_eq!(app.view, View::Tool(Tool::SignalCheck));
        assert!(app.tools_ui.session.is_none());
        let output = render(&ctx, &mut app, vec![]);
        click(
            &ctx,
            &mut app,
            at(&output, "All tools"),
            egui::PointerButton::Primary,
        );
        assert_eq!(app.view, View::Tools);
        app.tools_ui.filter = "there-is-no-such-tool".into();
        let output = render(&ctx, &mut app, vec![]);
        assert!(labels(&output)
            .iter()
            .any(|(s, _)| s.starts_with("No tools match")));
        app.tools_ui.filter.clear();
        for (width, height, zoom) in [(1000., 700., 1.), (1380., 960., 1.25), (1920., 1080., 1.)] {
            ctx.set_zoom_factor(zoom);
            for tool in Tool::ALL {
                app.view = View::Tool(tool);
                let output = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(
                            Pos2::ZERO,
                            Vec2::new(width, height),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        app.sidebar(ctx);
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.header(ui);
                            egui::ScrollArea::vertical().show(ui, |ui| app.tool_page(ui, tool));
                        });
                    },
                );
                assert!(
                    labels(&output).iter().any(|(s, _)| s == tool.name()),
                    "{} at {width}",
                    tool.name()
                );
                assert!(app.tools_ui.session.is_none());
            }
        }
        assert_eq!(serde_json::to_string(&app.project).unwrap(), before);
        assert!(!app.dirty && !app.settings_dirty);
        assert!(rx.try_recv().is_err());
    }
    #[test]
    fn navigation_and_global_stop_cancel_tool_streams_independently() {
        let mut app = app();
        let config = Config {
            mode: Mode::Tone,
            output: Some("PirateWave diagnostic nonexistent endpoint".into()),
            ..Config::default()
        };
        app.tools_ui.owner = Some(Tool::ToneForge);
        app.tools_ui.session = Some(Session::start(config.clone()).unwrap());
        app.handle_shortcut(ShortcutAction::App(AppAction::StopSounds));
        assert!(app.tools_ui.session.as_ref().unwrap().stop_requested());
        app.tools_ui.session = None;
        app.tools_ui.owner = Some(Tool::ToneForge);
        app.tools_ui.session = Some(Session::start(config).unwrap());
        app.tools_ui.poll(None);
        assert!(app
            .tools_ui
            .session
            .as_ref()
            .is_none_or(|s| s.stop_requested()));
    }
    #[test]
    fn reference_math_and_real_eq_response_are_calibrated() {
        assert!((workbench::buffer_ms(512, 48000) - 10.6666667).abs() < 1e-6);
        assert!((workbench::db(0.5) + 6.0206).abs() < 0.001);
        for frequency in [25., 80., 1000., 12000.] {
            assert!(bell_response(frequency, frequency, 0., 0.707).abs() < 0.001);
            assert!((bell_response(frequency, frequency, 6., 0.707) - 6.).abs() < 0.01);
        }
    }
    #[test]
    fn completed_readings_are_scoped_to_their_tool_and_gallery_fits_narrow_windows() {
        let mut app = app();
        app.tools_ui.results.insert(
            Tool::SignalCheck,
            Snapshot {
                reading: Reading {
                    frames: 48000,
                    peak_db: -12.,
                    ..Reading::default()
                },
                ..Snapshot::default()
            },
        );
        app.tools_ui.poll(Some(Tool::SignalCheck));
        assert_eq!(app.tools_ui.snapshot.reading.frames, 48000);
        app.tools_ui.poll(Some(Tool::RoomHush));
        assert_eq!(app.tools_ui.snapshot.reading.frames, 0);
        app.tools_ui.poll(Some(Tool::SignalCheck));
        assert_eq!(app.tools_ui.snapshot.reading.peak_db, -12.);
        let ctx = egui::Context::default();
        app.view = View::Tools;
        for width in [1000., 1380., 1920.] {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 700.))),
                    ..Default::default()
                },
                |ctx| {
                    app.sidebar(ctx);
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.header(ui);
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            app.tools_page(ui);
                            assert!(
                                ui.min_rect().right() <= ui.clip_rect().right() + 1.,
                                "Tools overflow at {width}"
                            );
                        });
                    });
                },
            );
        }
    }
    #[test]
    fn png_and_matching_set_exports_decode_and_preserve_alpha() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-tool-export-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        std::fs::create_dir_all(&root).unwrap();
        let d = Design {
            size: 144,
            transparent: true,
            ..Design::default()
        };
        export_icons(d.clone(), root.join("one.png"), false)
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let png = image::open(root.join("one.png")).unwrap().to_rgba8();
        assert_eq!(png.dimensions(), (144, 144));
        assert_eq!(png.get_pixel(0, 0)[3], 0);
        export_icons(d, root.join("set.zip"), true)
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .unwrap();
        let mut archive =
            zip::ZipArchive::new(std::fs::File::open(root.join("set.zip")).unwrap()).unwrap();
        assert_eq!(archive.len(), 10);
        for i in 0..archive.len() {
            use std::io::Read;
            let mut bytes = vec![];
            archive
                .by_index(i)
                .unwrap()
                .read_to_end(&mut bytes)
                .unwrap();
            assert_eq!(image::load_from_memory(&bytes).unwrap().width(), 144);
        }
        drop(archive);
        assert!(root
            .canonicalize()
            .unwrap()
            .starts_with(std::env::temp_dir().canonicalize().unwrap()));
        std::fs::remove_dir_all(root).unwrap();
    }
}
