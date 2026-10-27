//! Typed control requests shared by UI, shortcuts, controllers and guided rules.
use crate::{
    automation::Origin,
    effects::EffectRegistry,
    project::Project,
    shortcuts::ShortcutAction,
    workflows::{Control, Scene, SceneSetting},
};
#[derive(Clone, Debug)]
pub enum Action {
    Invoke(ShortcutAction),
    Set { node: u64, control: Control },
    Batch(Vec<Action>),
}
pub fn absolute(
    project: &Project,
    registry: &EffectRegistry,
    node: u64,
    control: Control,
) -> Result<Project, String> {
    Scene {
        id: 0,
        name: String::new(),
        duration_ms: 0,
        effect_kinds: vec![],
        settings: vec![SceneSetting {
            node,
            included: true,
            value: control,
        }],
        keybind: String::new(),
    }
    .apply(project, registry)
}
/// Validate all leaf targets before dispatch; batches cannot allocate audio resources.
pub fn validate(
    action: &Action,
    project: &Project,
    registry: &EffectRegistry,
    origin: Origin,
) -> Result<(), String> {
    fn visit(
        action: &Action,
        p: &Project,
        r: &EffectRegistry,
        origin: Origin,
        count: &mut usize,
        depth: usize,
    ) -> Result<(), String> {
        if depth > 8 {
            return Err("Action batch nesting exceeds eight levels".into());
        }
        match action {
            Action::Batch(actions) => {
                if actions.is_empty() || actions.len() > 32 {
                    return Err("An action batch requires 1-32 actions".into());
                }
                for action in actions {
                    visit(action, p, r, origin, count, depth + 1)?;
                }
            }
            Action::Invoke(action) => {
                *count += 1;
                if !action.exists(p) {
                    return Err("Action target is missing".into());
                }
                if origin == Origin::Automation
                    && matches!(
                        action,
                        ShortcutAction::RecordingSlot {
                            action: crate::recording::SlotAction::Delete,
                            ..
                        }
                    )
                {
                    return Err("Automation cannot delete recordings".into());
                }
                if let ShortcutAction::Scene(id) = action {
                    p.workflows
                        .scenes
                        .iter()
                        .find(|s| s.id == *id)
                        .unwrap()
                        .apply(p, r)?;
                }
            }
            Action::Set { node, control } => {
                *count += 1;
                absolute(p, r, *node, control.clone())?;
            }
        }
        if *count > 32 {
            return Err("An action batch supports at most 32 commands".into());
        }
        Ok(())
    }
    visit(action, project, registry, origin, &mut 0, 0)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_late_batch_target_cannot_modify_earlier_controls() {
        let p = Project::default();
        let r = EffectRegistry::default();
        let action = Action::Batch(vec![
            Action::Set {
                node: p.nodes[0].id,
                control: Control::Gain { value: -12. },
            },
            Action::Invoke(ShortcutAction::Sound(u64::MAX)),
        ]);
        assert!(validate(&action, &p, &r, Origin::Ui).is_err());
        assert_eq!(p.nodes[0].gain_db, 0.);
        assert!(validate(
            &Action::Batch(vec![
                Action::Invoke(ShortcutAction::App(
                    crate::shortcuts::AppAction::StopSounds
                ));
                33
            ]),
            &p,
            &r,
            Origin::Ui
        )
        .is_err());
    }
}
