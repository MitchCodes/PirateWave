//! Original geometric controller artwork. Raster previews and exports share the
//! same renderer and bundled UI font; no third-party logos or web assets.
use ab_glyph::{point, Font, FontRef, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Mute,
    Play,
    Stop,
    Record,
    Volume,
    Effects,
    Route,
    Headphones,
    Capture,
    Ducking,
}
impl Glyph {
    pub const ALL: [Self; 10] = [
        Self::Mute,
        Self::Play,
        Self::Stop,
        Self::Record,
        Self::Volume,
        Self::Effects,
        Self::Route,
        Self::Headphones,
        Self::Capture,
        Self::Ducking,
    ];
    pub fn name(self) -> &'static str {
        match self {
            Self::Mute => "Mute",
            Self::Play => "Play",
            Self::Stop => "Stop",
            Self::Record => "Record",
            Self::Volume => "Volume",
            Self::Effects => "Effects",
            Self::Route => "Route",
            Self::Headphones => "Listen",
            Self::Capture => "Capture",
            Self::Ducking => "Ducking",
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Design {
    pub glyph: Glyph,
    pub label: String,
    pub size: u32,
    pub background: [u8; 4],
    pub foreground: [u8; 4],
    pub label_color: [u8; 4],
    pub gradient: bool,
    pub transparent: bool,
    pub rounded: bool,
}
impl Default for Design {
    fn default() -> Self {
        Self {
            glyph: Glyph::Mute,
            label: "MUTE".into(),
            size: 288,
            background: [32, 29, 44, 255],
            foreground: [190, 167, 250, 255],
            label_color: [239, 236, 247, 255],
            gradient: true,
            transparent: false,
            rounded: true,
        }
    }
}
fn segment(p: [f32; 2], a: [f32; 2], b: [f32; 2], width: f32) -> bool {
    let d = [b[0] - a[0], b[1] - a[1]];
    let t = (((p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1]) / (d[0] * d[0] + d[1] * d[1]).max(1e-8))
        .clamp(0., 1.);
    ((p[0] - a[0] - t * d[0]).powi(2) + (p[1] - a[1] - t * d[1]).powi(2)).sqrt() < width
}
fn circle(p: [f32; 2], c: [f32; 2], r: f32) -> bool {
    (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) < r * r
}
fn mark(g: Glyph, p: [f32; 2]) -> bool {
    let line = |a, b| segment(p, a, b, 0.022);
    let speaker = || {
        (p[0] > 0.27 && p[0] < 0.4 && p[1] > 0.34 && p[1] < 0.49)
            || (p[0] > 0.38 && p[0] < 0.55 && (p[1] - 0.415).abs() < (p[0] - 0.28) * 0.55)
    };
    match g {
        Glyph::Mute => {
            speaker() || line([0.62, 0.33], [0.78, 0.5]) || line([0.78, 0.33], [0.62, 0.5])
        }
        Glyph::Volume => {
            speaker() || line([0.68, 0.32], [0.68, 0.51]) || line([0.59, 0.415], [0.77, 0.415])
        }
        Glyph::Play => p[0] > 0.35 && p[0] < 0.7 && (p[1] - 0.415).abs() < (0.7 - p[0]) * 0.65,
        Glyph::Stop => p[0] > 0.34 && p[0] < 0.67 && p[1] > 0.25 && p[1] < 0.58,
        Glyph::Record => circle(p, [0.5, 0.415], 0.19),
        Glyph::Effects => {
            line([0.58, 0.2], [0.36, 0.44])
                || line([0.36, 0.44], [0.61, 0.44])
                || line([0.61, 0.44], [0.43, 0.64])
        }
        Glyph::Route => {
            line([0.25, 0.42], [0.75, 0.42])
                || line([0.6, 0.27], [0.75, 0.42])
                || line([0.75, 0.42], [0.6, 0.57])
                || circle(p, [0.25, 0.42], 0.055)
        }
        Glyph::Headphones => {
            let r = ((p[0] - 0.5).powi(2) + (p[1] - 0.42).powi(2)).sqrt();
            (r > 0.2 && r < 0.245 && p[1] < 0.43)
                || (p[1] > 0.4
                    && p[1] < 0.61
                    && ((p[0] > 0.255 && p[0] < 0.35) || (p[0] > 0.65 && p[0] < 0.745)))
        }
        Glyph::Capture => {
            (circle(p, [0.5, 0.415], 0.13) && !circle(p, [0.5, 0.415], 0.09))
                || line([0.25, 0.24], [0.38, 0.24])
                || line([0.25, 0.24], [0.25, 0.36])
                || line([0.75, 0.24], [0.62, 0.24])
                || line([0.75, 0.24], [0.75, 0.36])
                || line([0.25, 0.59], [0.38, 0.59])
                || line([0.25, 0.59], [0.25, 0.47])
                || line([0.75, 0.59], [0.62, 0.59])
                || line([0.75, 0.59], [0.75, 0.47])
        }
        Glyph::Ducking => {
            line([0.28, 0.28], [0.28, 0.55])
                || line([0.28, 0.55], [0.43, 0.55])
                || line([0.43, 0.55], [0.53, 0.35])
                || line([0.53, 0.35], [0.74, 0.35])
                || line([0.64, 0.26], [0.74, 0.35])
                || line([0.74, 0.35], [0.64, 0.44])
        }
    }
}
pub fn render(d: &Design) -> Result<RgbaImage, String> {
    if ![144, 288, 576].contains(&d.size) {
        return Err("Icon size must be 144, 288 or 576 pixels".into());
    }
    let mut image = RgbaImage::new(d.size, d.size);
    // Four subsamples retain clean edges even at small key resolutions.
    for y in 0..d.size {
        for x in 0..d.size {
            let p = [
                (x as f32 + 0.5) / d.size as f32,
                (y as f32 + 0.5) / d.size as f32,
            ];
            let corner = d.rounded && {
                let cx = p[0].clamp(0.13, 0.87);
                let cy = p[1].clamp(0.13, 0.87);
                (p[0] - cx).powi(2) + (p[1] - cy).powi(2) > 0.13_f32.powi(2)
            };
            if corner {
                continue;
            }
            let mut bg = if d.transparent {
                [0, 0, 0, 0]
            } else {
                d.background
            };
            if d.gradient && !d.transparent {
                let factor = 1.2 - 0.4 * p[1];
                for value in &mut bg[..3] {
                    *value = (*value as f32 * factor).clamp(0., 255.) as u8;
                }
            }
            let coverage = [[-0.25, -0.25], [0.25, -0.25], [-0.25, 0.25], [0.25, 0.25]]
                .iter()
                .filter(|offset| {
                    mark(
                        d.glyph,
                        [
                            p[0] + offset[0] / d.size as f32,
                            p[1] + offset[1] / d.size as f32,
                        ],
                    )
                })
                .count() as f32
                / 4.;
            let alpha = coverage * d.foreground[3] as f32 / 255.;
            let ba = bg[3] as f32 / 255.;
            let a = alpha + ba * (1. - alpha);
            let mut value = [0; 4];
            if a > 0. {
                for c in 0..3 {
                    value[c] = ((d.foreground[c] as f32 * alpha + bg[c] as f32 * ba * (1. - alpha))
                        / a) as u8;
                }
                value[3] = (a * 255.) as u8;
            }
            image.put_pixel(x, y, Rgba(value));
        }
    }
    if !d.label.is_empty() {
        let definitions = eframe::egui::FontDefinitions::default();
        let data = definitions
            .font_data
            .get("Ubuntu-Light")
            .ok_or("Bundled UI font missing")?;
        let font = FontRef::try_from_slice(data.font.as_ref()).map_err(|e| e.to_string())?;
        let text: String = d.label.chars().take(24).collect();
        let mut scale = d.size as f32 * 0.115;
        let width = |scale: f32| {
            text.chars()
                .map(|c| {
                    font.as_scaled(PxScale::from(scale))
                        .h_advance(font.glyph_id(c))
                })
                .sum::<f32>()
        };
        if width(scale) > d.size as f32 * 0.85 {
            scale *= d.size as f32 * 0.85 / width(scale);
        }
        let scaled = font.as_scaled(PxScale::from(scale));
        let mut x = (d.size as f32 - width(scale)) * 0.5;
        for ch in text.chars() {
            let id = font.glyph_id(ch);
            let glyph =
                id.with_scale_and_position(PxScale::from(scale), point(x, d.size as f32 * 0.84));
            if let Some(outlined) = font.outline_glyph(glyph) {
                let bounds = outlined.px_bounds();
                outlined.draw(|gx, gy, coverage| {
                    let px = gx as i32 + bounds.min.x as i32;
                    let py = gy as i32 + bounds.min.y as i32;
                    if px < 0 || py < 0 || px >= d.size as i32 || py >= d.size as i32 {
                        return;
                    }
                    let pixel = image.get_pixel_mut(px as u32, py as u32);
                    let a = coverage * d.label_color[3] as f32 / 255.;
                    let ba = pixel[3] as f32 / 255.;
                    let out = a + ba * (1. - a);
                    if out > 0. {
                        for c in 0..3 {
                            pixel[c] = ((d.label_color[c] as f32 * a
                                + pixel[c] as f32 * ba * (1. - a))
                                / out)
                                .round() as u8;
                        }
                        pixel[3] = (out * 255.).round() as u8;
                    }
                });
            }
            x += scaled.h_advance(id);
        }
    }
    Ok(image)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_original_icons_render_export_sizes_labels_and_transparency() {
        for glyph in Glyph::ALL {
            let d = Design {
                glyph,
                size: 144,
                ..Design::default()
            };
            let image = render(&d).unwrap();
            assert_eq!(image.dimensions(), (144, 144));
            assert!(image.pixels().any(|p| p[3] > 0));
        }
        let d = Design {
            size: 576,
            transparent: true,
            label: "Mic α".into(),
            ..Design::default()
        };
        let image = render(&d).unwrap();
        assert_eq!(image.get_pixel(0, 0)[3], 0);
        assert!(image.pixels().any(|p| p[3] > 0));
        assert!(render(&Design {
            size: 8000,
            ..Design::default()
        })
        .is_err());
    }
}
