//! Optional workbench utilities. Device streams and analysis are isolated from
//! the project engine; opening a page never starts a stream or edits a project.
pub mod analysis;
pub mod icons;
pub mod session;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tool {
    SoundCheck,
    SignalCheck,
    EchoClock,
    RoomHush,
    BufferBudget,
    ToneForge,
    VocalMap,
    SpectralLens,
    ChannelCompass,
    ListenBack,
    ClockScope,
    DeviceAtlas,
    LevelGuide,
    VoiceSandbox,
    KeyCanvas,
}
impl Tool {
    pub const ALL: [Self; 15] = [
        Self::SoundCheck,
        Self::SignalCheck,
        Self::RoomHush,
        Self::SpectralLens,
        Self::ListenBack,
        Self::VoiceSandbox,
        Self::EchoClock,
        Self::ToneForge,
        Self::ChannelCompass,
        Self::DeviceAtlas,
        Self::ClockScope,
        Self::BufferBudget,
        Self::VocalMap,
        Self::LevelGuide,
        Self::KeyCanvas,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::SoundCheck => "Sound Check",
            Self::SignalCheck => "Signal Check",
            Self::EchoClock => "Echo Clock",
            Self::RoomHush => "Room Hush",
            Self::BufferBudget => "Buffer Budget",
            Self::ToneForge => "Tone Forge",
            Self::VocalMap => "Vocal Map",
            Self::SpectralLens => "Spectral Lens",
            Self::ChannelCompass => "Channel Compass",
            Self::ListenBack => "Listen Back",
            Self::ClockScope => "Clock Scope",
            Self::DeviceAtlas => "Device Atlas",
            Self::LevelGuide => "Level Guide",
            Self::VoiceSandbox => "Voice Sandbox",
            Self::KeyCanvas => "Key Canvas",
        }
    }
    pub fn category(self) -> &'static str {
        match self {
            Self::SignalCheck
            | Self::RoomHush
            | Self::SpectralLens
            | Self::ListenBack
            | Self::VoiceSandbox => "Microphone",
            Self::BufferBudget | Self::VocalMap | Self::LevelGuide => "Reference",
            Self::KeyCanvas => "Controller",
            _ => "Setup",
        }
    }
    pub fn description(self) -> &'static str {
        match self {
            Self::SoundCheck => {
                "Inspect endpoints, check a microphone and measure a test loop in one workspace."
            }
            Self::SignalCheck => {
                "Watch your input and run a five-second level, clipping and tonal check."
            }
            Self::EchoClock => {
                "Send a short chirp to a speaker and measure its return to a microphone."
            }
            Self::RoomHush => {
                "Measure the quiet part of your input without PirateWave effects or automatic gain."
            }
            Self::BufferBudget => {
                "Translate sample counts into buffer time and compare common configurations."
            }
            Self::ToneForge => {
                "Generate adjustable sine, square, triangle and sawtooth test signals."
            }
            Self::VocalMap => "Explore voice frequency regions and preview useful EQ curves.",
            Self::SpectralLens => {
                "Inspect a live logarithmic spectrum with peak hold, freeze and cursor readout."
            }
            Self::ChannelCompass => "Check left, right, center and polarity with short test tones.",
            Self::ListenBack => {
                "Monitor a selected microphone through a separate headphone output."
            }
            Self::ClockScope => {
                "Compare endpoint formats, supported rates and PirateWave's processing clock."
            }
            Self::DeviceAtlas => {
                "Explore native input and output endpoints, defaults and supported formats."
            }
            Self::LevelGuide => "Explore digital level, amplitude and acoustic reference scales.",
            Self::VoiceSandbox => {
                "Audition character effects locally without changing your project racks."
            }
            Self::KeyCanvas => {
                "Create original controller icons and export a matching set of PNGs."
            }
        }
    }
    pub fn input(self) -> bool {
        matches!(
            self,
            Self::SignalCheck
                | Self::RoomHush
                | Self::SpectralLens
                | Self::ListenBack
                | Self::VoiceSandbox
                | Self::EchoClock
                | Self::SoundCheck
        )
    }
}

pub fn buffer_ms(samples: u32, rate: u32) -> f64 {
    samples as f64 / rate.max(1) as f64 * 1000.
}
pub fn db(amplitude: f64) -> f32 {
    (20. * amplitude.max(1e-6).log10()).max(-120.) as f32
}

pub const EQ_REGIONS: [(&str, f32, f32, &str, &str); 7] = [
    (
        "Foundation",
        20.,
        100.,
        "Desk bumps, breath blasts and low rumble.",
        "Try an 70–100 Hz high-pass for speech. Keep useful low fundamentals.",
    ),
    (
        "Warmth",
        100.,
        250.,
        "The weight and chest of a voice.",
        "A broad, small move can add warmth or reduce boom. Compare with bypass.",
    ),
    (
        "Clarity",
        250.,
        600.,
        "Room buildup and a closed-in or boxy quality.",
        "Sweep for the actual resonance, then try a gentle narrow cut.",
    ),
    (
        "Character",
        600.,
        2000.,
        "Vowels, intelligibility and nasal coloration.",
        "Cut an irritating resonance rather than removing the entire midrange.",
    ),
    (
        "Focus",
        2000.,
        5000.,
        "Articulation, consonants and forwardness.",
        "A small wide lift can help speech; too much can become tiring.",
    ),
    (
        "Edge",
        5000.,
        9000.,
        "S and sh sounds, brightness and sharpness.",
        "Consider a de-esser for changing consonant peaks instead of a permanent broad cut.",
    ),
    (
        "Open air",
        9000.,
        20000.,
        "Breath detail and high-frequency hiss.",
        "A subtle shelf can open a dull recording but also raises the hiss.",
    ),
];
