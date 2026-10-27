//! Shared, clipped plots. All coordinates and analysis belong to the UI thread.
use super::{egui, theme, Align2, Color32, FontId, Pos2, Rect, Stroke, Vec2};

#[derive(Clone, Copy)]
pub enum Axis {
    Linear(f32, f32),
    Log(f32, f32),
}
impl Axis {
    pub fn value(self, fraction: f32) -> f32 {
        match self {
            Self::Linear(a, b) => a + (b - a) * fraction,
            Self::Log(a, b) => a * (b / a).powf(fraction),
        }
    }
    fn fraction(self, value: f32) -> f32 {
        match self {
            Self::Linear(a, b) => (value - a) / (b - a),
            Self::Log(a, b) => (value / a).ln() / (b / a).ln(),
        }
    }
}

pub struct Curve<'a> {
    pub points: &'a [[f32; 2]],
    pub color: Color32,
}

/// Shared response, transfer, spectrum, waveform and modulation plot.
/// A cursor reports axis values; zero lines and quarters provide visual scale.
pub fn plot(ui: &mut egui::Ui, x: Axis, y: Axis, curves: &[Curve<'_>], caption: &str) {
    let (outer, response) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(80.), 150.),
        egui::Sense::hover(),
    );
    if !ui.is_rect_visible(outer) {
        return;
    }
    let rect = Rect::from_min_max(
        outer.min + Vec2::new(38., 12.),
        outer.max - Vec2::new(12., 24.),
    );
    let p = ui.painter_at(outer);
    p.rect_filled(outer, 5., theme::palette().panel);
    for i in 0..=4 {
        let t = i as f32 / 4.;
        let px = rect.left() + rect.width() * t;
        let py = rect.bottom() - rect.height() * t;
        p.line_segment(
            [Pos2::new(px, rect.top()), Pos2::new(px, rect.bottom())],
            Stroke::new(1_f32, theme::palette().line),
        );
        p.line_segment(
            [Pos2::new(rect.left(), py), Pos2::new(rect.right(), py)],
            Stroke::new(1_f32, theme::palette().line),
        );
        p.text(
            Pos2::new(px, rect.bottom() + 5.),
            if i == 0 {
                Align2::LEFT_TOP
            } else if i == 4 {
                Align2::RIGHT_TOP
            } else {
                Align2::CENTER_TOP
            },
            if (x.value(1.) - x.value(0.)).abs() < 10. {
                format!("{:.1}", x.value(t))
            } else {
                format!("{:.0}", x.value(t))
            },
            FontId::proportional(10.),
            theme::palette().muted,
        );
        p.text(
            Pos2::new(rect.left() - 4., py),
            Align2::RIGHT_CENTER,
            if (y.value(1.) - y.value(0.)).abs() < 10. {
                format!("{:.1}", y.value(t))
            } else {
                format!("{:.0}", y.value(t))
            },
            FontId::proportional(10.),
            theme::palette().muted,
        );
    }
    for curve in curves {
        let points = curve
            .points
            .iter()
            .filter(|v| v.iter().all(|a| a.is_finite()))
            .map(|v| {
                Pos2::new(
                    rect.left() + x.fraction(v[0]).clamp(0., 1.) * rect.width(),
                    rect.bottom() - y.fraction(v[1]).clamp(0., 1.) * rect.height(),
                )
            })
            .collect();
        p.with_clip_rect(rect.intersect(ui.clip_rect()))
            .add(egui::Shape::line(points, Stroke::new(1.7_f32, curve.color)));
    }
    if let Some(pos) = response.hover_pos() {
        if rect.contains(pos) {
            p.line_segment(
                [
                    Pos2::new(pos.x, rect.top()),
                    Pos2::new(pos.x, rect.bottom()),
                ],
                Stroke::new(1_f32, theme::palette().muted),
            );
            response.on_hover_text(format!(
                "{caption}\nx: {:.2}   y: {:.2}",
                x.value((pos.x - rect.left()) / rect.width()),
                y.value((rect.bottom() - pos.y) / rect.height())
            ));
        }
    }
}

pub(super) fn bell_response(hz: f32, center: f32, gain: f32, q: f32) -> f32 {
    let a = 10_f64.powf(gain as f64 / 40.);
    let w = std::f64::consts::TAU * center as f64 / 48000.;
    let alpha = w.sin() / (2. * q as f64);
    let b = [1. + alpha * a, -2. * w.cos(), 1. - alpha * a];
    let a = [1. + alpha / a, -2. * w.cos(), 1. - alpha / a];
    let f = std::f64::consts::TAU * hz as f64 / 48000.;
    let power = |c: [f64; 3]| {
        (c[0] + c[1] * f.cos() + c[2] * (2. * f).cos()).powi(2)
            + (c[1] * f.sin() + c[2] * (2. * f).sin()).powi(2)
    };
    (10. * (power(b) / power(a).max(1e-24)).max(1e-24).log10()) as f32
}
