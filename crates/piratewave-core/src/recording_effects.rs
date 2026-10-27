//! Slot-local effect history. Construct/configure on the control plane; process
//! preallocated blocks on playback, or on a file worker for an offline export.
use crate::{
    effects::{AudioEffect, EffectRegistry, EffectSpec},
    Frame, SAMPLE_RATE,
};
pub struct RecordingEffects {
    effects: Vec<(EffectSpec, Box<dyn AudioEffect>)>,
    bypass: bool,
}

impl RecordingEffects {
    pub fn new(
        specs: &[EffectSpec],
        bypass: bool,
        registry: &EffectRegistry,
    ) -> Result<Self, String> {
        let effects = specs
            .iter()
            .map(|spec| Ok((spec.clone(), registry.create(spec, SAMPLE_RATE as f32)?)))
            .collect::<Result<_, String>>()?;
        Ok(Self { effects, bypass })
    }
    pub fn sync(
        &mut self,
        specs: &[EffectSpec],
        bypass: bool,
        registry: &EffectRegistry,
    ) -> Result<(), String> {
        if self.bypass == bypass && self.effects.iter().map(|(s, _)| s).eq(specs) {
            return Ok(());
        }
        for spec in specs {
            registry.validate(spec)?;
        }
        let mut next = Vec::with_capacity(specs.len());
        for spec in specs {
            let effect = if let Some(index) = self.effects.iter().position(|(old, _)| {
                old.id == spec.id
                    && old.kind == spec.kind
                    && old.extension == spec.extension
                    && old.extension_descriptor == spec.extension_descriptor
            }) {
                let (old, mut effect) = self.effects.remove(index);
                registry.configure(effect.as_mut(), spec)?;
                if old.enabled != spec.enabled || self.bypass != bypass {
                    effect.reset();
                }
                effect
            } else {
                registry.create(spec, SAMPLE_RATE as f32)?
            };
            next.push((spec.clone(), effect));
        }
        self.effects = next;
        self.bypass = bypass;
        Ok(())
    }
    pub fn process(&mut self, frames: &mut [Frame]) {
        if !self.bypass {
            for (spec, effect) in &mut self.effects {
                if spec.enabled {
                    effect.process(frames);
                }
            }
        }
    }
    pub fn faulted(&self) -> bool {
        !self.bypass && self.effects.iter().any(|(s, e)| s.enabled && e.faulted())
    }
    pub fn recover(&mut self) {
        for (_, e) in &mut self.effects {
            if e.faulted() {
                e.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BLOCK_SIZE;
    #[test]
    fn fresh_slot_histories_are_isolated_and_bypass_resets_delay_history() {
        let registry = EffectRegistry::default();
        let mut delay = EffectSpec::new(registry.get("delay").unwrap());
        delay.parameters.insert("time".into(), 1.);
        delay.parameters.insert("feedback".into(), 0.5);
        delay.parameters.insert("mix".into(), 1.);
        let mut first = RecordingEffects::new(&[delay.clone()], false, &registry).unwrap();
        let mut second = RecordingEffects::new(&[delay.clone()], false, &registry).unwrap();
        let mut impulse = [[0.; 2]; BLOCK_SIZE];
        impulse[0] = [1.; 2];
        let mut replay = impulse;
        first.process(&mut impulse);
        second.process(&mut replay);
        assert_eq!(impulse, replay);
        assert!(impulse.iter().any(|f| f[0] > 0.1));
        let mut other = RecordingEffects::new(&[delay.clone()], false, &registry).unwrap();
        let mut silence = [[0.; 2]; BLOCK_SIZE];
        other.process(&mut silence);
        assert_eq!(silence, [[0.; 2]; BLOCK_SIZE]);
        first.sync(&[delay.clone()], true, &registry).unwrap();
        first.sync(&[delay], false, &registry).unwrap();
        first.process(&mut silence);
        assert_eq!(silence, [[0.; 2]; BLOCK_SIZE]);
    }
    #[test]
    fn chain_order_and_individual_toggles_apply_without_modifying_specs() {
        let registry = EffectRegistry::default();
        let mut gain = EffectSpec::new(registry.get("gain").unwrap());
        gain.parameters.insert("gain".into(), 6.0206);
        let mut invert = EffectSpec::new(registry.get("invert_polarity").unwrap());
        let mut rack =
            RecordingEffects::new(&[gain.clone(), invert.clone()], false, &registry).unwrap();
        let mut block = [[0.2; 2]; BLOCK_SIZE];
        rack.process(&mut block);
        assert!((block[0][0] + 0.4).abs() < 0.00001);
        invert.enabled = false;
        rack.sync(&[invert, gain], false, &registry).unwrap();
        block.fill([0.2; 2]);
        rack.process(&mut block);
        assert!((block[0][0] - 0.4).abs() < 0.00001);
    }
}
