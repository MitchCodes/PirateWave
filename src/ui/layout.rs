use super::*;
pub(super) struct LayoutState {
    pub preferences_changed: Option<Instant>,
    pub detail_query: String,
    pub find_query: String,
    pub routing_origin: Vec2,
    pub routing_extent: Vec2,
    pub routing_offset: Vec2,
    pub routing_focus: Option<u64>,
    pub routing_drag: Option<(u64, Vec2)>,
    pub routing_viewport: Rect,
    pub source_viewport: Rect,
    pub bus_viewport: Rect,
    pub utility_viewport: Rect,
    pub mixer_focus: Option<u64>,
}
impl Default for LayoutState {
    fn default() -> Self {
        Self {
            preferences_changed: None,
            detail_query: String::new(),
            find_query: String::new(),
            routing_origin: Vec2::ZERO,
            routing_extent: Vec2::ZERO,
            routing_offset: Vec2::ZERO,
            routing_focus: None,
            routing_drag: None,
            routing_viewport: Rect::NOTHING,
            source_viewport: Rect::NOTHING,
            bus_viewport: Rect::NOTHING,
            utility_viewport: Rect::NOTHING,
            mixer_focus: None,
        }
    }
}
impl LayoutState {
    pub fn reset_navigation(&mut self) {
        let changed = self.preferences_changed;
        *self = Self::default();
        self.preferences_changed = changed;
    }
}
pub(super) fn node_size(kind: &NodeKind) -> Vec2 {
    if let NodeKind::Extension { definition } = kind {
        return Vec2::new(
            180.,
            82. + 28. * definition.inputs.len().max(definition.outputs.len()) as f32,
        );
    }
    Vec2::new(
        180.,
        if matches!(kind, NodeKind::Recording | NodeKind::AutoDucking) {
            138.
        } else {
            95.
        },
    )
}
pub(super) fn scene_bounds(nodes: &[Node], viewport: Vec2) -> (Vec2, Vec2) {
    let mut origin = Vec2::ZERO;
    let mut end = viewport;
    for node in nodes {
        let p = Vec2::from(node.position);
        origin = origin.min(p - Vec2::splat(32.));
        end = end.max(p + node_size(&node.kind) + Vec2::splat(96.));
    }
    (origin, (end - origin).max(viewport))
}
impl PirateWaveApp {
    pub(super) fn layout_preferences_changed(&mut self) {
        self.settings_dirty = true;
        self.layout.preferences_changed = Some(Instant::now());
    }
    pub(super) fn reveal_utility(&mut self, id: u64) {
        if self
            .project
            .nodes
            .iter()
            .any(|n| n.id == id && piratewave_core::shortcuts::UtilityFilter::All.includes(&n.kind))
        {
            self.app_settings.ui.utilities_expanded = true;
            self.app_settings.ui.utilities_filter = Default::default();
            self.layout.mixer_focus = Some(id);
            self.layout_preferences_changed();
        }
    }
    pub(super) fn node_control_view(&self, id: u64) -> View {
        match self
            .project
            .nodes
            .iter()
            .find(|n| n.id == id)
            .map(|n| &n.kind)
        {
            Some(NodeKind::Recording) => View::Recording(id),
            Some(NodeKind::AutoDucking) => View::Ducking(id),
            Some(NodeKind::VoiceChanger) => View::VoiceNode(id),
            Some(k) if k.is_plugin() => View::PluginNode(id),
            _ => View::Effects,
        }
    }
    /// A single node header shared by rack and dedicated editors. Returning true
    /// prevents the previous editor rendering after a navigation action.
    pub(super) fn node_header(&mut self, ui: &mut egui::Ui, id: u64) -> bool {
        self.selected = id;
        let before = self.selected;
        let mut keys = false;
        let mut back = false;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("node-controls-picker")
                .width(260.)
                .truncate()
                .selected_text(
                    self.project
                        .nodes
                        .iter()
                        .find(|n| n.id == id)
                        .map_or("Choose node", |n| n.name.as_str()),
                )
                .show_ui(ui, |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.layout.detail_query)
                            .hint_text("Find node…")
                            .desired_width(240.),
                    );
                    let query = self.layout.detail_query.to_lowercase();
                    let mut kinds = Vec::new();
                    for n in &self.project.nodes {
                        if !kinds.contains(&n.kind.label()) {
                            kinds.push(n.kind.label());
                        }
                    }
                    for kind in kinds {
                        let nodes: Vec<_> = self
                            .project
                            .nodes
                            .iter()
                            .filter(|n| {
                                n.kind.label() == kind
                                    && (query.is_empty()
                                        || n.name.to_lowercase().contains(&query)
                                        || kind.to_lowercase().contains(&query))
                            })
                            .collect();
                        if nodes.is_empty() {
                            continue;
                        }
                        ui.strong(kind);
                        for n in nodes {
                            ui.selectable_value(&mut self.selected, n.id, truncate(&n.name, 48))
                                .on_hover_text(&n.name);
                        }
                        ui.separator();
                    }
                });
            if let Some(node) = self.project.nodes.iter().find(|n| n.id == id) {
                ui.small(RichText::new(node.kind.label()).color(theme::palette().accent));
                if node.kind.has_effect_controls() || matches!(node.kind, NodeKind::AutoDucking) {
                    ui.small(if node.effects_disabled {
                        "Bypassed"
                    } else {
                        "Enabled"
                    });
                }
            }
            keys = ui
                .button(
                    if self
                        .project
                        .nodes
                        .iter()
                        .any(|n| n.id == id && matches!(n.kind, NodeKind::AutoDucking))
                    {
                        "Ducking keybinds"
                    } else {
                        "Keybinds"
                    },
                )
                .clicked();
            back = ui.button("< Back to mixer").clicked();
        });
        ui.add_space(12.);
        if back {
            self.view = View::Mixer;
            return true;
        }
        if keys {
            self.open_node_keybinds(id);
            return true;
        }
        if before != self.selected {
            if self.view != View::Effects {
                self.view = self.node_control_view(self.selected);
            }
            return true;
        }
        false
    }
}
