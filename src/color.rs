//! Color helpers. Colors are plain `[u8; 3]` RGB triples on the wire.

pub type Rgb = [u8; 3];

/// HSL (h in degrees, s/l in 0..=100) -> RGB, matching CSS `hsl()` semantics.
pub fn hsl(h: f32, s: f32, l: f32) -> Rgb {
    let h = h.rem_euclid(360.0) / 360.0;
    let s = (s / 100.0).clamp(0.0, 1.0);
    let l = (l / 100.0).clamp(0.0, 1.0);
    if s == 0.0 {
        let v = to_u8(l);
        return [v, v, v];
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    [
        to_u8(hue_to_rgb(p, q, h + 1.0 / 3.0)),
        to_u8(hue_to_rgb(p, q, h)),
        to_u8(hue_to_rgb(p, q, h - 1.0 / 3.0)),
    ]
}

fn hue_to_rgb(p: f32, q: f32, mut t: f32) -> f32 {
    if t < 0.0 {
        t += 1.0;
    }
    if t > 1.0 {
        t -= 1.0;
    }
    if t < 1.0 / 6.0 {
        p + (q - p) * 6.0 * t
    } else if t < 0.5 {
        q
    } else if t < 2.0 / 3.0 {
        p + (q - p) * (2.0 / 3.0 - t) * 6.0
    } else {
        p
    }
}

#[inline]
fn to_u8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// RGB -> HSL rounded to whole numbers, same as the SignalRGB effect's `hexToHSL`.
pub fn to_hsl(c: Rgb) -> (f32, f32, f32) {
    let [r, g, b] = c.map(|v| v as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    if max == min {
        return (0.0, 0.0, (l * 100.0).round());
    }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r {
        (g - b) / d + if g < b { 6.0 } else { 0.0 }
    } else if max == g {
        (b - r) / d + 2.0
    } else {
        (r - g) / d + 4.0
    };
    ((h * 60.0).round(), (s * 100.0).round(), (l * 100.0).round())
}

pub fn scale(c: Rgb, k: f32) -> Rgb {
    c.map(|v| (v as f32 * k).round().clamp(0.0, 255.0) as u8)
}

pub fn parse_hex(s: &str) -> Option<Rgb> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let v = u32::from_str_radix(s, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

pub fn to_hex(c: Rgb) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Reorder an RGB triple for hardware that expects a different channel order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Order {
    Rgb,
    Bgr,
    Grb,
}

impl Order {
    #[inline]
    pub fn apply(self, [r, g, b]: Rgb) -> Rgb {
        match self {
            Order::Rgb => [r, g, b],
            Order::Bgr => [b, g, r],
            Order::Grb => [g, r, b],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hsl_roundtrip_pastel_default() {
        // SignalRGB's Pastel default color.
        let c = parse_hex("#5050AF").unwrap();
        assert_eq!(to_hsl(c), (240.0, 37.0, 50.0));
        assert_eq!(hsl(0.0, 100.0, 50.0), [255, 0, 0]);
        assert_eq!(hsl(120.0, 100.0, 50.0), [0, 255, 0]);
        assert_eq!(hsl(-120.0, 100.0, 50.0), [0, 0, 255]);
    }
}
