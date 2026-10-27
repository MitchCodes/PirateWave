use super::*;
use crate::effects::{EffectRegistry, EffectSpec};
fn effect(id: &str) -> Box<dyn AudioEffect> {
    let registry = EffectRegistry::default();
    registry
        .create(&EffectSpec::new(registry.get(id).unwrap()), 48000.)
        .unwrap()
}
fn tone(hz: f32, amplitude: f32, n: usize) -> Vec<Frame> {
    (0..n)
        .map(|i| {
            let x = amplitude * (std::f32::consts::TAU * hz * i as f32 / 48000.).sin();
            [x, -x]
        })
        .collect()
}
fn rms(frames: &[Frame]) -> f32 {
    (frames.iter().map(|f| f[0] * f[0]).sum::<f32>() / frames.len() as f32).sqrt()
}
#[test]
fn expansion_and_upward_compression_follow_gain_curves() {
    for (id, mode, input, expected) in [
        ("expander", 0., -40., -50.),
        ("expander", 1., -20., -10.),
        ("upward_compressor", 0., -40., -35.),
    ] {
        let mut e = effect(id);
        if id == "expander" {
            e.set_parameter("mode", mode).unwrap();
        }
        e.set_parameter("threshold", -30.).unwrap();
        e.set_parameter("detection", 1.).unwrap();
        e.set_parameter("ratio", 2.).unwrap();
        let mut frames = vec![[amplitude(input), amplitude(input) * 0.25]; 48000];
        e.process(&mut frames);
        assert!(
            (db(frames.last().unwrap()[0]) - expected).abs() < 0.05,
            "{id}:{:?}",
            frames.last()
        );
        assert!((frames.last().unwrap()[1] / frames.last().unwrap()[0] - 0.25).abs() < 1e-6);
    }
    let mut e = effect("upward_compressor");
    e.set_parameter("range", 6.).unwrap();
    let mut quiet = vec![[1e-5; 2]; 48000];
    e.process(&mut quiet);
    assert!(db(quiet[47999][0] / 1e-5) <= 6.01);
}
#[test]
fn limiter_stops_transients_and_recovers_without_stereo_shift() {
    let mut e = effect("limiter");
    e.set_parameter("ceiling", -6.).unwrap();
    let ceiling = amplitude(-6.);
    let mut frames = vec![[3., -0.75]; 8192];
    e.process(&mut frames);
    assert!(frames.iter().flatten().all(|s| s.abs() <= ceiling));
    assert!((frames[0][1] / frames[0][0] + 0.25).abs() < 1e-6);
    let mut quiet = vec![[0.1; 2]; 48000];
    e.process(&mut quiet);
    assert!((quiet[47999][0] - 0.1).abs() < 1e-5);
}
#[test]
fn polarity_is_exact_and_channel_selective() {
    for (choice, wanted) in [(0., [-0.25, 0.5]), (1., [-0.25, -0.5]), (2., [0.25, 0.5])] {
        let mut e = effect("invert_polarity");
        e.set_parameter("channels", choice).unwrap();
        let mut frame = [[0.25, -0.5]];
        e.process(&mut frame);
        assert_eq!(frame[0], wanted);
    }
}
#[test]
fn equalizer_is_flat_by_default_and_bands_shape_their_regions() {
    for (band, hz) in [("low", 50.), ("mid", 1000.), ("high", 15000.)] {
        let source = tone(hz, 0.1, 24000);
        let mut audio = source.clone();
        let mut e = effect("three_band_equalizer");
        e.process(&mut audio);
        assert_eq!(audio, source);
        e.set_parameter(band, 6.).unwrap();
        let mut audio = source.clone();
        e.process(&mut audio);
        let gain = db(rms(&audio[12000..]) / rms(&source[12000..]));
        assert!((gain - 6.).abs() < 0.25, "{band}: {gain}");
    }
}
#[test]
fn deesser_targets_sibilance_and_leaves_low_voice_intact() {
    for hz in [200., 8000.] {
        let mut e = effect("de_esser");
        e.set_parameter("threshold", -40.).unwrap();
        let original = tone(hz, 0.25, 24000);
        let mut audio = original.clone();
        e.process(&mut audio);
        let change = db(rms(&audio[12000..]) / rms(&original[12000..]));
        if hz < 1000. {
            assert!(change.abs() < 0.2, "low voice {change}");
        } else {
            assert!(change < -3., "sibilance {change}");
        }
    }
}
#[test]
fn suppression_reduces_stationary_noise_and_has_aligned_dry_mix() {
    let mut e = effect("noise_suppression");
    let mut seed = 1234u32;
    let input: Vec<_> = (0..48000)
        .map(|_| {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            [seed as i32 as f32 / i32::MAX as f32 * 0.03; 2]
        })
        .collect();
    let mut audio = input.clone();
    for block in audio.chunks_mut(128) {
        e.process(block);
    }
    let reduction = db(rms(&audio[24000..]) / rms(&input[24000..]));
    assert!(reduction < -6., "noise reduction {reduction} dB");
    e.reset();
    e.set_parameter("strength", 0.).unwrap();
    let mut impulse = vec![[0.; 2]; 2048];
    impulse[0] = [0.5, -0.5];
    e.process(&mut impulse);
    assert_eq!(impulse[960], [0.5, -0.5]);
    assert_eq!(impulse.iter().filter(|f| f[0] != 0.).count(), 1);
}
#[test]
fn all_processors_roundtrip_reset_and_preserve_block_state() {
    let registry = EffectRegistry::default();
    for factory in factories() {
        let spec = EffectSpec::new(&factory);
        let saved = serde_json::to_string(&spec).unwrap();
        let spec: EffectSpec = serde_json::from_str(&saved).unwrap();
        let mut a = registry.create(&spec, 48000.).unwrap();
        let mut b = registry.create(&spec, 48000.).unwrap();
        let mut x = tone(700., 0.1, 4096);
        let mut y = x.clone();
        a.process(&mut x);
        for block in y.chunks_mut(127) {
            registry.configure(b.as_mut(), &spec).unwrap();
            b.process(block);
        }
        assert_eq!(x, y, "{} block/state", factory.id);
        a.reset();
        let mut silence = vec![[0.; 2]; 2048];
        a.process(&mut silence);
        assert!(
            silence
                .iter()
                .flatten()
                .all(|s| s.is_finite() && s.abs() < 1e-8),
            "{} reset",
            factory.id
        );
        for p in factory.parameters {
            assert!(a.set_parameter(p.id, f32::NAN).is_err());
        }
    }
    assert_eq!(
        registry
            .factories()
            .iter()
            .filter(|f| f.id == "gain")
            .count(),
        1
    );
}
#[test]
fn alternate_rates_remain_finite() {
    for rate in [8000., 44100., 96000., 384000.] {
        for factory in factories() {
            let mut e = (factory.create)(rate);
            let mut audio = vec![[0.03, -0.02]; 2048];
            e.process(&mut audio);
            assert!(
                audio.iter().flatten().all(|v| v.is_finite()),
                "{} at {rate}",
                factory.id
            );
        }
    }
}
