//! UI-only palettes; never consulted by the engine or project serializer.
use super::*;
use piratewave_core::shortcuts::Theme;
use std::cell::Cell;
#[derive(Clone, Copy)]
pub(super) struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub line: Color32,
    pub accent: Color32,
    pub muted: Color32,
    pub green: Color32,
    pub peach: Color32,
    pub text: Color32,
}
thread_local! { static CURRENT: Cell<Theme> = const { Cell::new(Theme::Dark) }; }
fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
fn colors(theme: Theme) -> Palette {
    let (bg, panel, line, accent, muted, green, peach, text) = match theme {
        Theme::Dark => (
            0x101114, 0x191a1f, 0x2f2f39, 0xbea7fa, 0x8f91a0, 0x97d3ae, 0xdfad94, 0xe7e5f0,
        ),
        Theme::Light => (
            0xf3f5f9, 0xffffff, 0xc5cbd6, 0x6341a5, 0x596273, 0x217445, 0x974524, 0x202633,
        ),
        Theme::Bumblebee => (
            0x11110d, 0x202018, 0x53513a, 0xffd447, 0xb9b79f, 0xb4df8a, 0xffaa60, 0xfff6cd,
        ),
        Theme::MintGreen => (
            0x0e1c19, 0x182c25, 0x355348, 0x8ce4bc, 0xa0b9ad, 0xb7e895, 0xf2bd96, 0xe6f7ec,
        ),
        Theme::Halloween => (
            0x19121e, 0x291c30, 0x513750, 0xffae54, 0xbba2c2, 0x9fe278, 0xf19ba8, 0xfaeafa,
        ),
        Theme::Christmas => (
            0x101e19, 0x1d3026, 0x3d5944, 0xf2cb79, 0xacc0ad, 0x9bdfad, 0xffa5a5, 0xf9f3e4,
        ),
        Theme::Ocean => (
            0x0c1a28, 0x132a3c, 0x31516a, 0x79d3f2, 0x9eb7c9, 0x91debf, 0xf2c38e, 0xe1f3fc,
        ),
        Theme::Rose => (
            0x21151c, 0x34222d, 0x624454, 0xf0a5c5, 0xc2a4b4, 0xa5dcb9, 0xf6c09f, 0xfcebf3,
        ),
        Theme::Lavender => (
            0xf0edf8, 0xfcfaff, 0xc9bfdc, 0x69439f, 0x665a77, 0x276b48, 0x97462c, 0x30263e,
        ),
        Theme::Ember => (
            0x211713, 0x34251e, 0x62483a, 0xf5b17c, 0xc3aa99, 0xb6d998, 0xf49588, 0xf9eee2,
        ),
        Theme::Midnight => (
            0x0b1020, 0x151e35, 0x354365, 0x9bb6ff, 0xa1acc8, 0x88d5b2, 0xf3b69c, 0xe7ecff,
        ),
    };
    Palette {
        bg: rgb(bg),
        panel: rgb(panel),
        line: rgb(line),
        accent: rgb(accent),
        muted: rgb(muted),
        green: rgb(green),
        peach: rgb(peach),
        text: rgb(text),
    }
}
pub(super) fn palette() -> Palette {
    CURRENT.with(|t| colors(t.get()))
}
pub(super) fn surface(dark: Color32) -> Color32 {
    CURRENT.with(|t| {
        if t.get() == Theme::Dark {
            return dark;
        }
        let p = colors(t.get());
        if dark == rgb(0x2b2339) || dark == rgb(0x483b58) {
            blend(p.panel, p.accent, 0.20)
        } else if dark == rgb(0x352724) {
            blend(p.panel, p.peach, 0.18)
        } else if dark == rgb(0x253333) {
            blend(p.panel, p.green, 0.15)
        } else {
            p.panel
        }
    })
}
pub(super) fn foreground(dark: Color32) -> Color32 {
    CURRENT.with(|t| {
        if t.get() == Theme::Dark {
            dark
        } else {
            colors(t.get()).text
        }
    })
}
pub(super) fn accent(dark: Color32) -> Color32 {
    CURRENT.with(|t| {
        if t.get() == Theme::Dark {
            dark
        } else {
            colors(t.get()).accent
        }
    })
}
fn blend(a: Color32, b: Color32, amount: f32) -> Color32 {
    let channel = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * amount).round() as u8;
    Color32::from_rgb(
        channel(a.r(), b.r()),
        channel(a.g(), b.g()),
        channel(a.b(), b.b()),
    )
}
pub(super) fn apply(ctx: &egui::Context, theme: Theme) {
    CURRENT.with(|t| t.set(theme));
    let key = egui::Id::new("piratewave-applied-theme");
    if ctx.data(|d| d.get_temp::<Theme>(key)) == Some(theme) {
        return;
    }
    let p = colors(theme);
    let light = matches!(theme, Theme::Light | Theme::Lavender);
    ctx.set_theme(if light {
        egui::Theme::Light
    } else {
        egui::Theme::Dark
    });
    let mut v = if light {
        egui::Visuals::light()
    } else {
        egui::Visuals::dark()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.panel;
    v.extreme_bg_color = if theme == Theme::Dark {
        rgb(0x0d0e11)
    } else {
        p.bg
    };
    v.faint_bg_color = p.panel;
    v.override_text_color = Some(p.text);
    v.selection.bg_fill = if theme == Theme::Dark {
        rgb(0x42315b)
    } else {
        blend(p.panel, p.accent, 0.25)
    };
    v.selection.stroke = Stroke::new(1_f32, p.accent);
    if theme != Theme::Dark {
        v.widgets.noninteractive.bg_stroke = Stroke::new(1_f32, p.line);
        v.widgets.noninteractive.fg_stroke.color = p.text;
        v.widgets.inactive.fg_stroke.color = p.text;
        v.widgets.hovered.fg_stroke.color = p.text;
        v.widgets.active.fg_stroke.color = p.text;
        v.widgets.inactive.bg_stroke.color = p.line;
        v.widgets.hovered.bg_stroke.color = p.accent;
        v.widgets.active.bg_stroke.color = p.accent;
        v.hyperlink_color = p.accent;
        v.warn_fg_color = p.peach;
        v.error_fg_color = if light { rgb(0xb12636) } else { rgb(0xff9292) };
    }
    for (widget, dark_color, weight) in [
        (&mut v.widgets.inactive, 0x25252d, 0.06),
        (&mut v.widgets.hovered, 0x362f44, 0.16),
        (&mut v.widgets.active, 0x413555, 0.24),
    ] {
        let color = if theme == Theme::Dark {
            rgb(dark_color)
        } else {
            blend(p.panel, p.accent, weight)
        };
        widget.bg_fill = color;
        if theme != Theme::Dark {
            widget.weak_bg_fill = color;
        }
    }
    ctx.set_visuals(v);
    ctx.data_mut(|d| d.insert_temp(key, theme));
    ctx.request_repaint();
}
#[cfg(test)]
mod tests {
    use super::*;
    fn contrast(a: Color32, b: Color32) -> f32 {
        let lum = |c: Color32| {
            let linear = |v: u8| {
                let v = v as f32 / 255.;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * linear(c.r()) + 0.7152 * linear(c.g()) + 0.0722 * linear(c.b())
        };
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }
    #[test]
    fn every_theme_renders_main_views_without_audio_or_project_updates() {
        let ctx = egui::Context::default();
        let mut app = super::super::tests::app();
        let project = serde_json::to_value(&app.project).unwrap();
        for theme in Theme::ALL {
            apply(&ctx, theme);
            app.app_settings.ui.theme = theme;
            for view in [
                View::Mixer,
                View::Routing,
                View::Effects,
                View::Settings,
                View::Soundboard,
            ] {
                app.view = view;
                let output = super::super::tests::render(&ctx, &mut app, vec![]);
                assert!(!output.shapes.is_empty());
                assert!(!app.dirty && !app.needs_apply && !app.needs_restart);
                assert_eq!(serde_json::to_value(&app.project).unwrap(), project);
            }
        }
        apply(&ctx, Theme::Dark);
    }
    #[test]
    fn palettes_are_readable_persistent_and_restore_dark() {
        let ctx = egui::Context::default();
        for theme in Theme::ALL {
            apply(&ctx, theme);
            let p = palette();
            for foreground in [p.text, p.muted, p.accent, p.green, p.peach] {
                for background in [p.bg, p.panel] {
                    assert!(
                        contrast(foreground, background) >= 4.5,
                        "{} contrast {}",
                        theme.label(),
                        contrast(foreground, background)
                    );
                }
            }
            assert_eq!(ctx.style().visuals.panel_fill, p.bg);
            assert_eq!(
                serde_json::from_str::<Theme>(&serde_json::to_string(&theme).unwrap()).unwrap(),
                theme
            );
        }
        apply(&ctx, Theme::Dark);
        assert_eq!(palette().bg, rgb(0x101114));
        assert_eq!(surface(rgb(0x202020)), rgb(0x202020));
        assert_eq!(
            serde_json::from_str::<piratewave_core::shortcuts::UiPreferences>("{}")
                .unwrap()
                .theme,
            Theme::Dark
        );
    }
}
