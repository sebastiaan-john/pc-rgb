//! Effects are pure functions of (settings, time, canvas position). No canvas is
//! rendered: each LED evaluates its own color directly, which costs nothing.

use crate::color::{hsl, scale, to_hsl, Rgb};
use crate::config::{Effect, Settings, WaveDir};

pub const CANVAS_W: f32 = 320.0;
pub const CANVAS_H: f32 = 200.0;

/// Per-frame precomputed effect state.
pub struct Frame {
    kind: Kind,
    bright: f32,
}

enum Kind {
    Flat(Rgb),
    /// Hue gradient across the canvas: hue = h0 + 360 * pos, at fixed s/l.
    Wave { h0: f32, s: f32, l: f32, dir: WaveDir },
}

impl Frame {
    pub fn new(s: &Settings, t: f64) -> Self {
        // SignalRGB effects advance `speed / 50` degrees per frame at 60 fps.
        let deg_per_sec = s.speed as f64 / 50.0 * 60.0;
        let front = (-(t * deg_per_sec)).rem_euclid(360.0) as f32;
        let kind = match s.effect {
            Effect::Off => Kind::Flat([0, 0, 0]),
            Effect::Solid => Kind::Flat(s.color),
            Effect::Breathing => {
                // 4 s cycle at speed 50.
                let hz = (s.speed.max(1) as f64 / 50.0) / 4.0;
                let k = 0.5 - 0.5 * (t * hz * std::f64::consts::TAU).cos();
                Kind::Flat(scale(s.color, k as f32))
            }
            Effect::Pastel => {
                let (h, sat, l) = to_hsl(s.pastel_color);
                Kind::Wave { h0: h + front, s: sat, l, dir: s.wave_dir }
            }
            Effect::Rainbow => Kind::Wave { h0: front, s: 100.0, l: 50.0, dir: s.wave_dir },
        };
        Frame { kind, bright: s.brightness as f32 / 100.0 }
    }

    #[inline]
    pub fn color(&self, x: f32, y: f32) -> Rgb {
        let c = match self.kind {
            Kind::Flat(c) => c,
            Kind::Wave { h0, s, l, dir } => {
                let p = match dir {
                    WaveDir::Horizontal => x / CANVAS_W,
                    WaveDir::Vertical => y / CANVAS_H,
                };
                hsl(h0 + 360.0 * p.clamp(0.0, 1.0), s, l)
            }
        };
        if self.bright >= 1.0 {
            c
        } else {
            scale(c, self.bright)
        }
    }
}
