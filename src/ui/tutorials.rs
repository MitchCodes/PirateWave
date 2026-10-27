//! Beginner guides live beside the UI, independently of real-time DSP.
use super::*;

struct Tutorial {
    effect: &'static str,
    overview: &'static str,
    steps: &'static [&'static str],
    controls: &'static [(&'static str, &'static str)],
    listen: &'static str,
    placement: &'static str,
}

const GUIDES: &[Tutorial] = &[
 Tutorial {effect:"automatic_gain_control",overview:"Automatically raises quiet speech and reduces loud speech toward a target average level. This controls the signal inside PirateWave, not the physical microphone gain in Windows.",steps:&["Start with Target RMS at -18 dB and the default boost/reduction limits. Speak softly and loudly, allowing a moment for levels to settle.","If room noise becomes audible between phrases, raise Noise floor or reduce Maximum boost. Place a limiter after AGC to catch sudden peaks."],controls:&[("target","The desired average RMS level in dB. This is not a peak ceiling."),("boost","The largest gain increase allowed for quiet speech."),("reduction","The largest gain reduction allowed for loud speech."),("floor","Below this input level the controller returns toward unity gain instead of boosting room noise."),("attack","How quickly gain drops for loud speech. Short times react sooner but can sound less natural."),("release","How slowly gain rises for quiet speech. Longer times avoid pumping between words.")],listen:"Listen for steady speech levels. AGC cannot undo clipping already recorded at the microphone; leave input headroom. Use a final limiter for peaks.",placement:"After noise suppression or a gentle gate, before creative effects and a final limiter."},

    Tutorial {
        effect: "reverb",
        overview: "Adds the lingering reflections you hear in a room after a sound stops. A little can make a dry microphone feel more spacious; more can make a voice sound distant, like it is in a hall. Unlike a delay, the reflections blend into a soft tail rather than distinct repeating echoes.",
        steps: &["For speech, start with Wet mix around 10–15%, Decay around 0.5–1 second, Pre-delay around 20 ms, Damping around 50% and Stereo width at 100%.", "Say a short phrase and pause so you can hear the tail. Increase Wet mix to hear more room, or lower it if words become harder to understand. For singing or a dramatic hall, try Decay around 2–3 seconds with a modest Wet mix."],
        controls: &[("mix", "Balances the original sound (dry) against the added room sound (wet). 0% gives the original sound; 100% gives only the reflections. Keep this low for clear speech. The reflections continue internally at 0%, so disable the card if you want to bypass and clear the effect."), ("decay", "How long the room lingers, in seconds. Short values sound tighter; long values create a larger, more lingering impression. This is approximately the time for the undamped tail to fade by 60 dB; Damping can make high notes fade sooner."), ("damping", "How quickly brightness fades from the reflections. Higher values give a darker, softer tail; lower values keep it bright and lively. This changes the reflections, not the tone of the original dry voice."), ("predelay", "The pause before reflections begin. About 15–30 ms can help the start of each word stay clear. Larger values make the room seem farther away and may sound like a separate echo. The room network adds a short reflection delay of its own, so 0 ms does not mean an immediate wet signal."), ("width", "How spread out the reflections sound between left and right. 0% makes the wet sound mono, centered equally in both channels; 100% gives a wider stereo space. Your original dry stereo sound keeps its own width.")],
        listen: "If speech sounds washed out or words run together, reduce Wet mix or Decay. If the tail is too sharp, increase Damping. A microphone already recorded in an echoey room may need very little added reverb. Keep output levels below clipping, and compare on and off at a similar volume.",
        placement: "Usually after noise suppression, a gate/expander, EQ and compression, so those effects clean the voice before reflections are added. A gate placed after reverb can cut off its tail. A limiter afterward can catch peaks. On a separate parallel reverb route, use 100% Wet mix to avoid duplicating the dry voice.",
    },
    Tutorial {
        effect: "gain",
        overview: "A volume adjustment for this point in the chain. It makes everything louder or quieter, including background noise. It does not even out changes between quiet and loud words.",
        steps: &["Start at 0 dB: this means no volume change.", "Speak at your loudest normal level. Increase Gain a little if the signal is too quiet, or decrease it if later effects or the output become overloaded."],
        controls: &[("gain", "Positive dB makes the signal louder; negative dB makes it quieter. Start with small changes of 1–3 dB.")],
        listen: "If the sound becomes harsh or crackly, turn the gain down. Turning up a quiet recording also turns up its hiss; use better microphone placement rather than relying on a large boost.",
        placement: "Before another effect to change how strongly that effect reacts, or near the end to adjust overall level. Raising gain after a limiter can undo its ceiling.",
    },
    Tutorial {
        effect: "high_pass",
        overview: "Removes very low sounds while allowing higher sounds through. Use it on a voice to reduce desk bumps, microphone rumble and some low hum.",
        steps: &["Start with Cutoff around 80 Hz for speech.", "Raise it slowly while speaking. When your voice starts sounding thin, lower it a little. A higher voice may tolerate a higher cutoff."],
        controls: &[("cutoff", "The boundary between low sounds being reduced and higher sounds being kept. Higher Hz removes more of the bottom of your voice. The boundary is gradual, not a brick wall.")],
        listen: "Keep the warmth of your voice. This will not remove keyboard clicks, high hiss or all background noise. Be cautious on music or deep voices because their low notes may be wanted.",
        placement: "Usually early in a voice chain, before a gate or compressor, so rumble is less likely to trigger them.",
    },
    Tutorial {
        effect: "equalizer",
        overview: "Changes one area of your sound's tone. Think of it as a movable bass/treble control: you choose the area, then gently boost or reduce it.",
        steps: &["Leave Gain at 0 dB to begin. Choose a Frequency: roughly 150–300 Hz for boominess, 1–3 kHz for speech presence, or higher frequencies for brightness.", "Try a small cut of 2–3 dB with a broad Width (Q), around 0.7–1. Compare Enabled on and off at a similar listening volume."],
        controls: &[("frequency", "The center of the tone area you want to change. Lower Hz means deeper sounds; higher Hz means brighter sounds."), ("gain", "How much to boost or cut that area. 0 dB leaves it unchanged. Small cuts are often easier to make sound natural than big boosts."), ("q", "How narrow the affected area is. A smaller Q changes a wider area; a larger Q targets a narrower area.")],
        listen: "A large bass boost can make speech muddy; too much upper-frequency boost can make it piercing. This changes tone, not just the total volume. You can add another EQ card to adjust a second area.",
        placement: "Often before compression for general tone shaping. Put it after compression if you prefer to shape the already-evened-out voice. Listen and compare.",
    },
    Tutorial {
        effect: "three_band_equalizer",
        overview: "Three simple tone controls: Low adds or removes weight, Mid changes the body and forwardness of speech, and High changes brightness.",
        steps: &["Start with all three controls at 0 dB.", "If your voice is boomy, try Low at −2 dB. If it sounds dull, try High at +1 or +2 dB. Change one control at a time and compare Enabled on and off."],
        controls: &[("low", "Boosts or cuts the bass region around and below 200 Hz. Too much can sound boomy."), ("mid", "Boosts or cuts a broad area centered at 1,000 Hz. Too much may sound nasal or boxy; too little can make speech less clear."), ("high", "Boosts or cuts the bright region around and above 5,000 Hz. Boosting can add clarity but also emphasize hiss and sharp S sounds.")],
        listen: "Aim for natural, clear speech. These bands have fixed frequencies; use Parametric EQ if you need to target a particular problem more precisely.",
        placement: "Use near the start for tone shaping, or after a compressor for a finishing adjustment. Large boosts can overload effects or outputs later in the route.",
    },
    Tutorial {
        effect: "gate",
        overview: "Turns quiet passages down strongly. It can hide room noise while you are not speaking. It opens again when the signal is loud enough, so noise underneath your speech remains audible.",
        steps: &["Stay quiet and listen to the room, then speak softly. Put Threshold above the room noise but below your quietest wanted words. The default −42 dB is only a starting point.", "Try Release around 100–200 ms. Read a sentence with quiet beginnings and endings, then pause. Adjust until words remain complete and pauses become quieter."],
        controls: &[("threshold", "The level required to open the gate. A higher value, such as −30 rather than −50 dB, blocks more quiet sound but can also block soft speech."), ("release", "How gradually the gate closes after the sound falls below Threshold. Longer times preserve word endings; shorter times silence pauses more quickly.")],
        listen: "If the first or last parts of words disappear, lower Threshold or lengthen Release. If noise continues during speech, try Noise Suppression. Use a downward Expander for gentler quieting instead of a gate.",
        placement: "Usually after noise cleanup and before compression. A compressor can raise noise and make a gate harder to set.",
    },
    Tutorial {
        effect: "noise_suppression",
        overview: "Uses a speech-focused noise-removal model to reduce unwanted background sound, including during speech. Unlike a gate, it tries to separate noise from your voice rather than quieting the entire signal during pauses.",
        steps: &["Use it on a microphone voice route. Begin with Strength around 50% and speak while your usual fan or room noise is present.", "Increase Strength until the noise is acceptably reduced. Back it off if your voice becomes watery, metallic or loses soft syllables."],
        controls: &[("strength", "Blends between the original sound at 0% and fully processed sound at 100%. Higher values generally remove more noise but can change your voice more.")],
        listen: "This is intended for speech, so music and sound effects may be damaged. It cannot perfectly remove every click, another person's speech or a very noisy room. Processing adds about 20 ms of delay even at 0% Strength; disable the effect to bypass it.",
        placement: "Usually early in a microphone chain, before compression and creative effects. A gate or downward expander afterward can further quiet pauses.",
    },
    Tutorial {
        effect: "compressor",
        overview: "Turns down louder parts more than quieter parts, making speech more even. It helps when ordinary words are quiet but laughter or excited speech is much louder.",
        steps: &["Start with Ratio at 3:1 and Makeup at 0 dB. Try Threshold around −18 dB, then speak quietly and loudly.", "Lower Threshold until loud phrases are controlled without flattening everything. Add a little Makeup if the processed voice is now too quiet."],
        controls: &[("threshold", "The level above which sound is turned down. Lowering it makes the compressor act on more of your speech."), ("ratio", "How strongly loud parts are controlled. At 4:1, a rise of 4 dB above the threshold becomes roughly 1 dB above it once the compressor settles. 1:1 does no compression."), ("makeup", "Turns the entire compressed signal back up afterward. It also raises room noise, so do not use it just to make the comparison sound more impressive.")],
        listen: "If the voice seems lifeless or the room noise swells between words, raise Threshold or reduce Ratio/Makeup. This compressor has built-in response timing; it is not a guaranteed output ceiling.",
        placement: "Usually after noise cleanup and EQ, before a final limiter. A limiter can catch short peaks that compression lets through.",
    },
    Tutorial {
        effect: "expander",
        overview: "Makes the gap between quiet and loud sounds bigger. Downward mode makes quiet parts quieter, useful for room noise between words. Upward mode makes loud parts louder, useful for adding punch but also capable of creating excessive peaks.",
        steps: &["For cleaner speech pauses, choose Downward, Ratio 2:1, Attack 5 ms, Release 120 ms and Maximum change around 12 dB.", "Set Threshold between room noise and soft speech. Start near −35 dB and adjust while speaking. If you want Upward mode instead, use a small Maximum change and listen for overly loud peaks."],
        controls: &[("mode", "Downward reduces sound below Threshold. Upward boosts sound above Threshold. Neither mode is the same as an upward compressor."), ("threshold", "The dividing line between quiet and loud. In Downward mode, setting it too high can suppress quiet words."), ("ratio", "The amount of expansion. Higher values separate quiet and loud more strongly; 1:1 leaves the dynamics unchanged."), ("attack", "How quickly the volume change grows: quieting in Downward mode, or boosting in Upward mode. Shorter times make expansion take effect sooner."), ("release", "How gradually the volume change returns toward zero. In Downward mode, this controls how quickly the voice comes back as speech gets louder; too long can soften word beginnings."), ("range", "Caps the amount of volume change. In Downward mode this limits how much sound can be reduced; in Upward mode it limits the boost."), ("detection", "RMS follows average loudness for smoother behavior. Peak reacts more to brief spikes such as clicks."), ("output", "A final volume adjustment after expansion. Leave at 0 dB while setting the other controls.")],
        listen: "If speech fades in or gets chopped, lower Threshold, reduce Ratio or use less Maximum change. Upward mode can exaggerate shouts and clicks; watch your output level.",
        placement: "Downward expansion often goes after noise suppression and before compression. Put a limiter later if using Upward mode to add punch.",
    },
    Tutorial {
        effect: "upward_compressor",
        overview: "Brings quieter sounds up toward the louder ones. It can make soft words easier to hear while leaving louder parts closer to their original level. It can also make room noise louder.",
        steps: &["Start with Ratio 2:1, Maximum boost around 6 dB and Output gain 0 dB.", "Try Threshold around −30 dB. Say some quiet words followed by louder ones. Adjust Threshold until the quiet words are easier to hear, then lower Maximum boost if noise becomes distracting."],
        controls: &[("threshold", "Sounds below this level can be boosted. Raising it brings more of the signal into the boosting region."), ("ratio", "How much quieter levels are drawn toward the threshold. Higher ratios produce stronger leveling; 1:1 does no compression."), ("attack", "How quickly the boost grows when sound becomes quiet. Shorter times lift quiet passages sooner; longer times make the boost arrive gradually."), ("release", "How gradually the boost backs off when sound gets louder. Too long can briefly boost the beginning of louder words."), ("range", "The largest allowed boost. This is an important limit for keeping room noise from becoming too loud."), ("detection", "RMS follows average loudness; Peak reacts to brief spikes. RMS is a useful starting point for speech."), ("output", "Changes the overall volume after leveling. Keep it at 0 dB until the quiet/loud balance is right.")],
        listen: "If breaths, hiss and room noise become too obvious, lower Maximum boost or Ratio. An upward expander does the opposite kind of job: it boosts loud parts, increasing the gap rather than reducing it.",
        placement: "After noise suppression or gentle downward expansion. Consider a final limiter if the combined chain can create loud peaks.",
    },
    Tutorial {
        effect: "de_esser",
        overview: "Reduces overly sharp S, SH and similar sounds only when the high-frequency part becomes strong. It is more selective than turning down all treble with an equalizer.",
        steps: &["Say a sentence with several S sounds. Start with Sibilance cutoff around 4,500–6,000 Hz, Split band processing, Ratio 4:1 and Maximum reduction around 6 dB.", "Lower Threshold until the sharp sounds soften. Compare Enabled on and off, keeping enough brightness for the words to remain clear."],
        controls: &[("frequency", "The high-frequency boundary used to detect sibilance. Lower values include more of your voice; higher values focus on the brightest sounds."), ("threshold", "How strong the detected high-frequency sound must be before reduction begins. Lower values cause more frequent reduction."), ("ratio", "How strongly the sharp sounds are reduced after crossing the threshold. Start moderately."), ("range", "Limits how much reduction is allowed. Smaller values help preserve a natural voice."), ("attack", "How quickly reduction reacts. A short time catches the beginning of sharp consonants."), ("release", "How gradually reduction relaxes afterward. Too long can dull following vowels."), ("mode", "Split band reduces the high-frequency portion while keeping the lower part. Broadband briefly lowers the entire signal when sharp sounds are detected.")],
        listen: "If S sounds become a lisp, reduce Maximum reduction or Ratio, or raise Threshold. If entire words dip in volume, try Split band or less reduction.",
        placement: "Often after EQ or compression if those make S sounds more noticeable. Keep it before a final limiter.",
    },
    Tutorial {
        effect: "limiter",
        overview: "Catches loud peaks so this effect's output samples stay below a chosen ceiling. It is a final guard against sudden shouts or peaks created by other effects.",
        steps: &["Place it near the end and start with Ceiling at −1 dB and Release around 60 ms.", "Speak loudly or play the loudest expected sound. If the limiter seems to work constantly, lower the level entering it instead of relying on heavy limiting."],
        controls: &[("ceiling", "The highest allowed sample level at this effect's output. −1 dB leaves some room below digital maximum; a lower value limits more strongly."), ("release", "How gradually the volume recovers after a peak. Very short times can sound rough; long times can make following speech dip for too long.")],
        listen: "Heavy limiting can sound squeezed or distorted. This is a sample-peak limiter, not a true-peak limiter or a loudness normalizer. It cannot repair distortion already recorded at the microphone.",
        placement: "Usually last in the effects chain. The node's volume control, downstream effects and combining several routes can still raise the signal after this limiter, so keep an eye on the final output meter too.",
    },
    Tutorial {
        effect: "invert_polarity",
        overview: "Flips the audio waveform upside down. A single signal usually sounds the same afterward. It is mainly useful when combining two versions of the same sound that partly cancel each other out.",
        steps: &["Leave this disabled unless you are combining related microphones or duplicate routes and something sounds unusually thin.", "While listening to the combined result, enable the effect on just one route and compare. Keep the setting that sounds fuller and more natural; if it does not help, disable it."],
        controls: &[("channels", "Both flips the left and right channels together. Left or Right flips only that side. Use a one-sided flip only when troubleshooting a specific stereo polarity problem.")],
        listen: "This is not a left/right swap, a volume change or a timing adjustment. It cannot fix every cancellation problem caused by microphone distance or delay. Flipping only one side can make centered sounds disappear when summed to mono.",
        placement: "On one of the affected routes before they are combined. It is a troubleshooting tool, not a routine voice enhancement.",
    },
    Tutorial {
        effect: "delay",
        overview: "Adds audible repeats, like an echo. Use a small amount for space or a larger amount as a creative voice or music effect.",
        steps: &["Start with Time at 250 ms, Feedback around 0.2 and Wet / dry around 0.15.", "Speak a short phrase and pause. Adjust Time for the spacing of repeats, then bring the mix down until the original words remain easy to understand."],
        controls: &[("time", "The time before each repeat. 1,000 ms is one second. Short times can sound like a doubling effect; longer times produce distinct echoes."), ("feedback", "How much of each echo is fed back to make another echo. 0 gives one repeat; larger values make longer trails. The control uses a fraction, so 0.25 means 25%."), ("mix", "Balances original (dry) and echo (wet) sound. 0 is original only; 1 is echo only. A small fraction keeps speech clearer.")],
        listen: "Too many repeats can cover up the next sentence. Lower Feedback or Wet / dry if words become hard to follow. This is an audible echo effect, not a dedicated audio/video synchronization control.",
        placement: "Usually after voice cleanup and tone/dynamics effects. A limiter afterward can help control overlapping echoes.",
    },
    Tutorial {
        effect: "automatic_voice_correction",
        overview: "Moves the pitch of a single voice toward musical notes. Slow correction can sound gentle; very fast correction makes the obvious stepped, autotuned sound. It works best on one clear singing voice, not a full song or several people together.",
        steps: &["Choose the song's Key and Major or Minor scale if you know them. Otherwise start with Chromatic, which allows every musical note, and choose the closest Voice range.", "For an obvious tuned effect, try Retune time 0–10 ms, Humanize 0%, Correction amount 100% and Wet mix 100%. Sing a held note and slide slowly between notes.", "For a gentler result, try Retune time 50–100 ms and add some Humanize. Adjust while singing with the actual backing track; a wrong key can force your voice onto unwanted notes."],
        controls: &[("key", "The musical home note of the song, such as C or D. It selects which notes are allowed together with Scale. In Chromatic mode all notes are allowed, so Key does not restrict the targets."), ("scale", "Major or Minor restricts correction to that musical scale. Chromatic allows all twelve notes in an octave and is useful when you do not know the song's scale."), ("voice", "Sets the expected pitch range to help the detector follow your voice. Start with the closest vocal range; a poor choice may cause missed notes or octave jumps."), ("retune", "How quickly your pitch moves toward a target note. Lower milliseconds gives a faster, more obvious jump; higher values leave more natural movement."), ("humanize", "Makes correction gentler on sustained notes, preserving more movement. Keep at 0% for a deliberately rigid tuned sound."), ("detune", "Shifts the tuning reference slightly. Cents are tiny pitch steps: 100 cents equals one semitone. Leave at 0 unless the music uses a different tuning."), ("transpose", "Shifts the result up or down in semitones (st). 12 semitones equals one octave. Start at 0; large shifts can sound unnatural."), ("amount", "How strongly the pitch correction is applied. 100% gives full correction; lower values leave more of the original pitch."), ("mix", "How much corrected sound you hear. 100% is fully processed; blending in the original voice can create a doubled or chorused sound because the pitches differ.")],
        listen: "If notes wobble or jump unexpectedly, try a better voice-range setting and a cleaner microphone signal. Speaking may produce odd results because speech does not follow musical notes. This effect adds about 29 ms of delay at 48 kHz and does not preserve vocal formants, so large shifts change vocal character.",
        placement: "On the vocal route, after gentle cleanup and before delay or other creative effects. Avoid feeding background music into the same correction input.",
    },
Tutorial { effect:"vocoder", overview:"Uses twelve frequency bands of your speech to shape a synthesized carrier. Classic saw is buzzy, Robot uses a sine, Chord uses three notes, and Pulse has a hollow tone.", steps:&["Speak clearly while choosing the carrier and character. Try a fully wet mix for a stronger robotic voice."], controls:&[("mode","Choose Classic saw, Robot, Chord or Pulse. These are different carrier sounds shaped by your speech."),("carrier","Sets the fundamental carrier note in Hz. Lower notes sound deeper; it does not follow your natural pitch."),("release","How long each speech band rings after a syllable. Shorter values articulate words more sharply."),("mix","0 is the original voice; 1 is fully vocoded. Start around 0.5.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"resonator", overview:"Adds a tuned ringing resonance with a damped feedback delay.", steps:&["Start at 220 Hz and gradually increase Wet / dry. Adjust the pitch to tune the ringing tone."], controls:&[("frequency","Sets the ringing pitch in Hz."),("feedback","Higher values sustain the resonance for longer. Reduce it if words sound metallic or ring too long."),("damping","Higher values soften high frequencies in the ringing tail."),("mix","Blends dry speech and the ringing sound; 0 is dry and 1 is wet.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"distortion", overview:"Adds harmonics through saturation, clipping or waveform folding.", steps:&["Start with Warm saturation and low Drive. Compare at a similar volume and keep the final output below clipping."], controls:&[("drive","Increases the level entering the nonlinear stage and therefore the amount of distortion."),("mode","Warm saturation rounds peaks. Hard clip flattens peaks. Foldback folds large waves back on themselves."),("output","Reduces the wet output level after distortion. Lower it when increasing Drive."),("mix","Blends clean and distorted audio; 0 is dry and 1 is wet.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"pitch", overview:"Transposes a single voice using period-aligned delay heads. Large shifts also change the apparent vocal character because formants are not preserved.", steps:&["Try -5 or +5 semitones and Wet / dry at 1. This algorithm adds about 29 ms of delay at 48 kHz and can color transients. It works best on one clear voice."], controls:&[("semitones","12 semitones is one octave up; -12 is one octave down. 0 leaves pitch unchanged."),("mix","0 gives original speech and 1 gives shifted speech. Blending creates a doubled voice.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"flanger", overview:"Sweeps a very short delay to create moving comb-filter notches.", steps:&["Start with the defaults, then change Sweep rate slowly. Reduce feedback if the sound rings too much."], controls:&[("rate","How fast the sweep cycles in Hz."),("depth","The delay sweep range in milliseconds."),("feedback","Recirculates the delayed audio; negative values change the notch character."),("mix","0 is dry and 1 is delayed only. Around 0.5 gives the strongest comb effect.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"chorus", overview:"Blends two slowly modulated delay voices to make speech sound doubled and wider.", steps:&["Start around Wet / dry 0.3 and increase Depth until the doubling is audible."], controls:&[("rate","The speed of the pitch-like movement in Hz."),("depth","How far delay times move in milliseconds. Larger values give more noticeable detuning."),("mix","0 is dry and 1 is the doubled voices only. A modest blend keeps speech clear.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"tremolo", overview:"Raises and lowers volume rhythmically without changing pitch.", steps:&["Try 4 Hz with Sine selected. Increase Depth for stronger pulses."], controls:&[("rate","Number of volume pulses each second."),("depth","0 leaves the level unchanged; 1 allows the quiet part to reach silence."),("mode","Sine gives smooth pulses; Square alternates abruptly for a chopping effect.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"stereo", overview:"Controls mid/side stereo width, balance and right-channel delay. Delay can widen a mono microphone in headphones.", steps:&["Try 8 ms Right channel delay on a mono microphone. Check mono playback too: delayed copies can cancel at some frequencies."], controls:&[("width","0 sums to mono, 1 preserves the input width and 2 exaggerates stereo differences. Width alone does not widen a mono source."),("delay","Delays the right channel in milliseconds for a Haas widening effect. Set 0 to disable the delay."),("balance","-1 keeps the left channel, 0 centers balance and +1 keeps the right channel.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
Tutorial { effect:"wahwah", overview:"Sweeps a resonant band-pass filter to create moving vowel-like tones.", steps:&["Start with the defaults and slowly vary Sweep rate. Reduce Resonance if the sound is too sharp."], controls:&[("rate","How quickly the filter sweeps back and forth."),("frequency","The lowest frequency reached by the sweep."),("depth","The sweep range in octaves. One octave doubles the frequency."),("q","Higher values make the resonant peak narrower and sharper."),("mix","0 is dry and 1 is fully filtered. Lower values preserve more of the original speech.")], listen:"Compare Enabled on and off at a similar volume. Reduce the effect amount if speech becomes hard to understand.", placement:"After voice cleanup. A limiter near the end of the chain can catch added peaks." },
];

impl PirateWaveApp {
    pub(super) fn effect_help_popup(&mut self, ctx: &egui::Context) {
        let Some(id) = self.effect_help.as_deref() else {
            return;
        };
        let Some(factory) = self.registry.get(id) else {
            self.effect_help = None;
            return;
        };
        let mut open = true;
        let mut close = false;
        egui::Window::new(format!("{} — Help", factory.name))
            .id(egui::Id::new("effect-tutorial"))
            .open(&mut open)
            .collapsible(false)
            .default_width(640.0)
            .default_height(700.0)
            .max_height((ctx.screen_rect().height() - 100.0).max(200.0))
            .vscroll(true)
            .default_pos(Pos2::new(300.0, 60.0))
            .show(ctx, |ui| {
                close = ui.button("Close help").clicked();
                ui.label("You can keep this guide open while adjusting the effect. Audio keeps running.");
                ui.separator();
                        if let Some(guide) = GUIDES.iter().find(|guide| guide.effect == id) {
                            ui.heading("What it does");
                            ui.label(guide.overview);
                            ui.add_space(12.0);
                            ui.heading("Try it step by step");
                            for (i, step) in guide.steps.iter().enumerate() {
                                ui.label(format!("{}. {step}", i + 1));
                                ui.add_space(5.0);
                            }
                            ui.add_space(8.0);
                            ui.heading("What the controls mean");
                            for parameter in factory.parameters {
                                ui.strong(parameter.label);
                                ui.label(guide.controls.iter().find(|(id, _)| *id == parameter.id).map_or("Adjust a little at a time and compare Enabled on and off.", |(_, text)| *text));
                                ui.add_space(7.0);
                            }
                            ui.heading("What to listen for");
                            ui.label(guide.listen);
                            ui.add_space(12.0);
                            ui.heading("Where to put it");
                            ui.label(guide.placement);
                        } else {
                            ui.heading("About this effect");
                            ui.label(factory.description);
                            ui.label("Begin with the default settings, change one control at a time, and compare Enabled on and off.");
                        }
                        ui.add_space(12.0);
                        ui.separator();
                        ui.heading("Audio words made simple");
                        ui.label("dB measures level: 0 dB on a gain control means no change; negative values turn it down. A signal level of 0 dB is digital maximum. Hz describes pitch or frequency: low numbers are bass, high numbers are treble. ms means milliseconds; 1,000 ms is one second.");
                        ui.label("Effects run from top to bottom. Compare Enabled on and off at a similar volume: louder can seem better even when the sound has not improved. If a processor node is set to Disable (passthrough), enable it in Mixer to hear its effects.");
            });
        if !open || close {
            self.effect_help = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_builtin_effect_and_control_has_a_tutorial() {
        let registry = EffectRegistry::default();
        assert_eq!(GUIDES.len(), registry.factories().len());
        for factory in registry.factories() {
            let guides: Vec<_> = GUIDES.iter().filter(|g| g.effect == factory.id).collect();
            assert_eq!(guides.len(), 1, "{} needs exactly one guide", factory.id);
            let guide = guides[0];
            assert!(!guide.steps.is_empty());
            for parameter in factory.parameters {
                assert_eq!(
                    guide
                        .controls
                        .iter()
                        .filter(|(id, text)| *id == parameter.id && !text.is_empty())
                        .count(),
                    1,
                    "Missing tutorial: {} / {}",
                    factory.id,
                    parameter.id
                );
            }
        }
    }
}
