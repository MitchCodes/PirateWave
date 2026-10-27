use super::*;
use piratewave_core::project::ApplicationTarget;
pub(super) fn smoke_applications() -> Vec<crate::application_capture::Application> {
    vec![crate::application_capture::Application {
        pid: 42,
        executable: "C:\\Program Files\\Example Player\\player.exe".into(),
        title: "Example Player - Now playing".into(),
    }]
}
pub(super) fn picker(
    ui: &mut egui::Ui,
    id: u64,
    target: &mut Option<ApplicationTarget>,
    applications: &[crate::application_capture::Application],
) -> bool {
    let before = target.clone();
    let text = target.as_ref().map_or("Choose application...".into(), |t| {
        let name = t
            .executable
            .rsplit(['\\', '/'])
            .next()
            .unwrap_or(&t.executable);
        format!("{name}: {}", t.window_title)
    });
    egui::ComboBox::from_id_salt((id, "application"))
        .width(160.0)
        .truncate()
        .selected_text(text)
        .show_ui(ui, |ui| {
            ui.selectable_value(target, None, "No application");
            for application in applications {
                let selected = target.as_ref().is_some_and(|t| {
                    t.executable.eq_ignore_ascii_case(&application.executable)
                        && t.window_title == application.title
                });
                let name = application
                    .executable
                    .rsplit(['\\', '/'])
                    .next()
                    .unwrap_or(&application.executable);
                if ui
                    .selectable_label(selected, format!("{name} — {}", application.title))
                    .on_hover_text(&application.executable)
                    .clicked()
                {
                    *target = Some(ApplicationTarget {
                        executable: application.executable.clone(),
                        window_title: application.title.clone(),
                        match_title: false,
                    });
                }
            }
            if applications.is_empty() {
                ui.label("Open the target app, then refresh.");
            }
        });
    if let Some(target) = target {
        ui.checkbox(&mut target.match_title, "Match exact window title");
        ui.label(
            RichText::new("Includes child processes")
                .small()
                .color(theme::palette().muted),
        );
    }
    before != *target
}
impl PirateWaveApp {
    pub(super) fn refresh_applications(&mut self) {
        match crate::application_capture::applications() {
            Ok(applications) => {
                self.applications = applications;
                self.applications_error.clear();
            }
            Err(error) => {
                self.applications_error = format!("{error:#}");
                self.notify(format!("Application list: {error:#}"));
            }
        }
    }
}
