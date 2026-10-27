use super::*;
impl PirateWaveApp {
    pub(super) fn workspace_menu(&mut self, ui: &mut egui::Ui) {
        let mut switch = None;
        let mut create = false;
        let mut clone = false;
        let selector_width = ui.available_width();
        egui::menu::menu_custom_button(
            ui,
            egui::Button::new(RichText::new(format!("{}  v", self.project.name)).strong())
                .min_size(Vec2::new(selector_width, 42.))
                .wrap_mode(egui::TextWrapMode::Truncate),
            |ui| {
                ui.set_min_width(280.);
                ui.set_max_width(360.);
                if let Some(store) = &self.workspaces {
                    egui::ScrollArea::vertical()
                        .max_height(250.)
                        .show(ui, |ui| {
                            for entry in store.entries() {
                                if ui
                                    .selectable_label(entry.id == store.active(), &entry.name)
                                    .clicked()
                                {
                                    switch = Some(entry.id);
                                    ui.close_menu();
                                }
                            }
                        });
                    ui.separator();
                    if ui.button("New workspace").clicked() {
                        create = true;
                        ui.close_menu();
                    }
                    if ui.button("Clone current workspace").clicked() {
                        clone = true;
                        ui.close_menu();
                    }
                    if ui.button("Delete current workspace").clicked() {
                        self.workspace_delete = Some((store.active(), self.project.name.clone()));
                        ui.close_menu();
                    }
                    if ui.button("Rename workspace...").clicked() {
                        self.workspace_rename = Some(self.project.name.clone());
                        ui.close_menu();
                    }
                } else {
                    ui.label("Workspace catalog unavailable; see status message.");
                }
            },
        );
        if clone {
            self.add_workspace(self.project.clone());
        } else if create {
            let mut number = 1;
            while self.workspaces.as_ref().is_some_and(|s| {
                s.entries()
                    .iter()
                    .any(|e| e.name.eq_ignore_ascii_case(&format!("Workspace {number}")))
            }) {
                number += 1;
            }
            let project = Project {
                name: format!("Workspace {number}"),
                ..Project::default()
            };
            self.add_workspace(project);
        } else if let Some(id) = switch {
            self.switch_workspace(id);
        }
        if let Some((id, name)) = self.workspace_delete.clone() {
            let mut confirm = false;
            let mut cancel = false;
            egui::Window::new("Delete workspace?")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label(format!("Delete workspace \"{name}\"?"));
                    ui.label("This removes its saved configuration. Shared audio files are kept.");
                    ui.horizontal(|ui| {
                        confirm = ui.button("Delete workspace").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            if confirm {
                self.workspace_delete = None;
                self.delete_workspace(id);
            } else if cancel {
                self.workspace_delete = None;
            }
        }
        if let Some(mut name) = self.workspace_rename.clone() {
            let mut apply = false;
            let mut cancel = false;
            egui::Window::new("Rename workspace")
                .collapsible(false)
                .resizable(false)
                .show(ui.ctx(), |ui| {
                    ui.label("Workspace names must be unique.");
                    ui.text_edit_singleline(&mut name);
                    ui.horizontal(|ui| {
                        apply = ui.button("Rename").clicked();
                        cancel = ui.button("Cancel").clicked();
                    });
                });
            self.workspace_rename = if cancel { None } else { Some(name.clone()) };
            if apply {
                if let Some(store) = &mut self.workspaces {
                    match store.rename(store.active(), &name) {
                        Ok(()) => {
                            self.project.name = name.trim().into();
                            self.workspace_rename = None;
                            self.dirty = true;
                            self.save();
                        }
                        Err(e) => self.notify_error(format!("Cannot rename workspace: {e:#}")),
                    }
                }
            }
        }
    }
    pub(super) fn switch_workspace(&mut self, id: u64) {
        if self.workspaces.as_ref().is_none_or(|s| s.active() == id) {
            return;
        }
        if self.dirty {
            self.save();
            if self.dirty {
                return;
            }
        }
        let result = (|| -> anyhow::Result<_> {
            let store = self.workspaces.as_mut().unwrap();
            let project = store.load(id)?;
            store.select(id)?;
            Ok((project, store.path(id)))
        })();
        match result {
            Ok((project, path)) => {
                // Start replaces the complete session (including clips/history), rather
                // than reconfiguring and carrying recordings across workspace IDs.
                let running = self.status.running;
                if let Some(h) = &mut self.hotkeys {
                    h.clear();
                }
                self.controller.generation = piratewave_core::recording::RecordSlot::new("").id;
                self.controller.recordings.clear();
                self.layout.reset_navigation();
                self.recording_ui = Default::default();
                self.effect_tuning = Default::default();
                self.project = project;
                self.path = path;
                self.dirty = false;
                self.needs_apply = false;
                self.needs_restart = false;
                self.selected = self.project.nodes.first().map_or(0, |n| n.id);
                self.view = View::Mixer;
                self.clips.clear();
                self.loading.clear();
                self.pending_port = None;
                self.selected_edge = None;
                self.recorder = None;
                self.choose_effect = false;
                self.add_node = false;
                self.effect_help = None;
                self.workspace_rename = None;
                self.workspace_delete = None;
                self.refresh_shortcuts();
                self.load_clips();
                if running {
                    self.command(Command::Start(self.project.clone()));
                }
                self.notify(if running {
                    "Workspace switched; audio restarting with this workspace's devices."
                } else {
                    "Workspace switched"
                });
            }
            Err(e) => self.notify_error(format!("Cannot switch workspace: {e:#}")),
        }
    }
    fn delete_workspace(&mut self, id: u64) {
        let Some(store) = &self.workspaces else {
            return;
        };
        if store.active() != id {
            return;
        }
        let replacement = store.entries().iter().find(|e| e.id != id).map(|e| e.id);
        let replacement = match replacement {
            Some(id) => id,
            None => {
                let project = Project {
                    name: "Workspace 1".into(),
                    ..Project::default()
                };
                match self.workspaces.as_mut().unwrap().add(project) {
                    Ok(id) => id,
                    Err(error) => {
                        self.notify_error(format!(
                            "Cannot create replacement workspace: {error:#}"
                        ));
                        return;
                    }
                }
            }
        };
        self.switch_workspace(replacement);
        if self.workspaces.as_ref().unwrap().active() == id {
            return;
        }
        match self.workspaces.as_mut().unwrap().delete(id) {
            Ok(()) => self.notify("Workspace deleted"),
            Err(error) => self.notify_error(format!("Cannot delete workspace: {error:#}")),
        }
    }
    pub(super) fn add_workspace(&mut self, project: Project) {
        if let Some(store) = &mut self.workspaces {
            match store.add(project) {
                Ok(id) => self.switch_workspace(id),
                Err(e) => self.notify_error(format!("Cannot create workspace: {e:#}")),
            }
        }
    }
    pub(super) fn import_workspace(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Workspace JSON", &["json"])
            .pick_file()
        {
            match storage::load(&path) {
                Ok(project) => self.add_workspace(project),
                Err(e) => self.notify_error(format!("Cannot import workspace: {e:#}")),
            }
        }
    }
    pub(super) fn export_workspace(&mut self) {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Workspace JSON", &["json"])
            .set_file_name("workspace.json")
            .save_file()
        {
            // Export does not change autosave's managed destination.
            match storage::save(&path, &self.project) {
                Ok(()) => self.notify("Workspace exported"),
                Err(e) => self.notify_error(format!("Cannot export workspace: {e:#}")),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deleting_last_workspace_creates_and_selects_replacement() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-delete-ui-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let store =
            crate::workspaces::Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        let original = store.active();
        let original_path = store.path(original);
        let mut app = super::super::tests::app();
        app.project = store.load(original).unwrap();
        app.path = original_path.clone();
        app.workspaces = Some(store);
        app.project.nodes[0].gain_db = -3.;
        app.dirty = true;
        app.delete_workspace(original);
        assert_ne!(app.workspaces.as_ref().unwrap().active(), original);
        assert_eq!(app.workspaces.as_ref().unwrap().entries().len(), 1);
        assert!(!original_path.exists());
        assert!(!app.dirty);
        assert_ne!(app.project.nodes[0].gain_db, -3.);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn switching_saves_previous_and_restarts_running_audio_without_carrying_state() {
        let root = std::env::temp_dir().join(format!(
            "piratewave-workspace-ui-{}",
            piratewave_core::recording::RecordSlot::new("").id
        ));
        let mut store =
            crate::workspaces::Workspaces::open(root.clone(), &root.join("absent.json")).unwrap();
        let first = store.active();
        let next = store
            .add(Project {
                name: "VR".into(),
                nodes: vec![],
                connections: vec![],
                sounds: vec![],
                ..Project::default()
            })
            .unwrap();
        let mut app = super::super::tests::app();
        app.path = store.path(first);
        app.project = store.load(first).unwrap();
        app.workspaces = Some(store);
        app.project.nodes[0].gain_db = -9.;
        app.dirty = true;
        app.status.running = true;
        app.needs_apply = true;
        app.needs_restart = true;
        let generation = app.controller.generation;
        app.switch_workspace(next);
        assert_eq!(app.project.name, "VR");
        assert!(app.engine.snapshot().unwrap().running);
        assert_ne!(app.controller.generation, generation);
        assert!(!app.needs_apply && !app.needs_restart);
        assert_eq!(
            app.workspaces.as_ref().unwrap().load(first).unwrap().nodes[0].gain_db,
            -9.
        );
        app.command(Command::Stop);
        app.status.running = false;
        app.switch_workspace(first);
        assert!(!app.engine.snapshot().unwrap().running);
        drop(app);
        std::fs::remove_dir_all(root).unwrap();
    }
}
