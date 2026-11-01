//! Settings + a tiny `key = value` config file at %APPDATA%\pc-rgb\config.ini.

use crate::color::{parse_hex, to_hex, Rgb};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    Solid,
    Pastel,
    Rainbow,
    Breathing,
    Off,
}

impl Effect {
    pub const ALL: [Effect; 5] = [Effect::Solid, Effect::Pastel, Effect::Rainbow, Effect::Breathing, Effect::Off];

    pub fn key(self) -> &'static str {
        match self {
            Effect::Solid => "solid",
            Effect::Pastel => "pastel",
            Effect::Rainbow => "rainbow",
            Effect::Breathing => "breathing",
            Effect::Off => "off",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Effect::Solid => "Solid color",
            Effect::Pastel => "Pastel",
            Effect::Rainbow => "Rainbow",
            Effect::Breathing => "Breathing",
            Effect::Off => "Off",
        }
    }

    fn from_key(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.key() == s)
    }

    pub fn animated(self) -> bool {
        matches!(self, Effect::Pastel | Effect::Rainbow | Effect::Breathing)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WaveDir {
    Horizontal,
    Vertical,
}

/// Where a device sits on the 320x200 effect canvas, SignalRGB-style:
/// top-left corner plus the canvas size of one LED grid cell.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
    pub x: f32,
    pub y: f32,
    pub sx: f32,
    pub sy: f32,
}

impl Place {
    const fn new(x: f32, y: f32, sx: f32, sy: f32) -> Self {
        Place { x, y, sx, sy }
    }

    /// Canvas position of the centre of grid cell (gx, gy).
    #[inline]
    pub fn at(&self, gx: f32, gy: f32) -> (f32, f32) {
        (self.x + (gx + 0.5) * self.sx, self.y + (gy + 0.5) * self.sy)
    }
}

/// What's plugged into a motherboard 5V ARGB header.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Header {
    None,
    /// Straight strip with N LEDs.
    Strip(u16),
    /// Corsair ML fan, 8 LEDs on a 4x4 grid.
    MlFan,
}

impl Header {
    pub fn led_count(self) -> usize {
        match self {
            Header::None => 0,
            Header::Strip(n) => n as usize,
            Header::MlFan => 8,
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "none" | "0" => Some(Header::None),
            "ml_fan" => Some(Header::MlFan),
            n => n.parse::<u16>().ok().filter(|&n| n <= 120).map(Header::Strip),
        }
    }

    fn fmt(self) -> String {
        match self {
            Header::None => "none".into(),
            Header::Strip(n) => n.to_string(),
            Header::MlFan => "ml_fan".into(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub gpu: Place,
    /// RAM sticks in ascending SMBus address order.
    pub ram: [Place; 2],
    /// Motherboard zones Led 1-7 + 12V header.
    pub mb_zones: Place,
    pub mb_led8: Place,
    pub header: [Place; 3],
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    pub effect: Effect,
    pub color: Rgb,
    pub pastel_color: Rgb,
    pub wave_dir: WaveDir,
    /// 0..=100, 50 = SignalRGB "normal".
    pub speed: u8,
    /// 0..=100 percent.
    pub brightness: u8,
    pub fps: u8,
    pub off_on_exit: bool,
    /// LEDs per RAM stick; 0 = detect from SMBIOS part number.
    pub ram_leds: u8,
    pub headers: [Header; 3],
    pub layout: Layout,
}

impl Default for Settings {
    /// Defaults mirror the SignalRGB setup this replaces (Pastel, speed 20, same canvas layout).
    fn default() -> Self {
        Settings {
            effect: Effect::Pastel,
            color: [0x00, 0x9b, 0xde],
            pastel_color: [0x50, 0x50, 0xaf],
            wave_dir: WaveDir::Horizontal,
            speed: 20,
            brightness: 100,
            fps: 20,
            off_on_exit: true,
            ram_leds: 0,
            headers: [Header::None, Header::MlFan, Header::Strip(60)],
            layout: Layout {
                gpu: Place::new(29.0, 112.0, 25.9, 8.2),
                ram: [Place::new(197.0, 15.0, 17.0, 17.0), Place::new(177.0, 15.0, 17.0, 17.0)],
                mb_zones: Place::new(100.0, 40.0, 40.0, 40.0),
                mb_led8: Place::new(56.0, 32.0, 40.0, 40.0),
                header: [
                    Place::new(24.0, 98.0, 4.533, 4.533),
                    Place::new(53.0, 17.0, 40.0, 40.0),
                    Place::new(24.0, 98.0, 4.533, 4.533),
                ],
            },
        }
    }
}

pub fn dir() -> PathBuf {
    let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."));
    base.join("pc-rgb")
}

pub fn path() -> PathBuf {
    dir().join("config.ini")
}

fn parse_place(v: &str) -> Option<Place> {
    let mut it = v.split(',').map(|p| p.trim().parse::<f32>());
    let p = Place::new(it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?, it.next()?.ok()?);
    it.next().is_none().then_some(p)
}

fn fmt_place(p: &Place) -> String {
    format!("{}, {}, {}, {}", p.x, p.y, p.sx, p.sy)
}

impl Settings {
    pub fn load() -> Self {
        let mut s = Settings::default();
        let Ok(text) = std::fs::read_to_string(path()) else {
            return s;
        };
        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            let num = |max: u8| v.parse::<u8>().ok().map(|n| n.min(max));
            match k {
                "effect" => s.effect = Effect::from_key(v).unwrap_or(s.effect),
                "color" => s.color = parse_hex(v).unwrap_or(s.color),
                "pastel_color" => s.pastel_color = parse_hex(v).unwrap_or(s.pastel_color),
                "wave" => {
                    s.wave_dir = if v == "vertical" { WaveDir::Vertical } else { WaveDir::Horizontal }
                }
                "speed" => s.speed = num(100).unwrap_or(s.speed),
                "brightness" => s.brightness = num(100).unwrap_or(s.brightness),
                "fps" => s.fps = num(60).unwrap_or(s.fps).max(1),
                "off_on_exit" => s.off_on_exit = v == "true" || v == "1",
                "ram_leds" => s.ram_leds = num(12).unwrap_or(s.ram_leds),
                "header1" | "header2" | "header3" => {
                    let i = (k.as_bytes()[6] - b'1') as usize;
                    s.headers[i] = Header::parse(v).unwrap_or(s.headers[i]);
                }
                _ => {
                    let Some(p) = parse_place(v) else { continue };
                    let l = &mut s.layout;
                    match k {
                        "pos.gpu" => l.gpu = p,
                        "pos.ram1" => l.ram[0] = p,
                        "pos.ram2" => l.ram[1] = p,
                        "pos.mb_zones" => l.mb_zones = p,
                        "pos.mb_led8" => l.mb_led8 = p,
                        "pos.header1" => l.header[0] = p,
                        "pos.header2" => l.header[1] = p,
                        "pos.header3" => l.header[2] = p,
                        _ => {}
                    }
                }
            }
        }
        s
    }

    pub fn save(&self) -> std::io::Result<()> {
        let l = &self.layout;
        let text = format!(
            "# pc-rgb config. Edit while running, then pick \"Reload config\" from the tray menu.\n\
             \n\
             # solid | pastel | rainbow | breathing | off\n\
             effect = {}\n\
             color = {}\n\
             pastel_color = {}\n\
             # horizontal | vertical\n\
             wave = {}\n\
             # 0-100 (50 = normal)\n\
             speed = {}\n\
             brightness = {}\n\
             fps = {}\n\
             off_on_exit = {}\n\
             # LEDs per RAM stick, 0 = auto-detect from part number\n\
             ram_leds = {}\n\
             \n\
             # 5V ARGB headers: none | ml_fan | <led count for a strip>\n\
             header1 = {}\n\
             header2 = {}\n\
             header3 = {}\n\
             \n\
             # Canvas placement (320x200 canvas, like SignalRGB): x, y, cell width, cell height\n\
             pos.gpu = {}\n\
             pos.ram1 = {}\n\
             pos.ram2 = {}\n\
             pos.mb_zones = {}\n\
             pos.mb_led8 = {}\n\
             pos.header1 = {}\n\
             pos.header2 = {}\n\
             pos.header3 = {}\n",
            self.effect.key(),
            to_hex(self.color),
            to_hex(self.pastel_color),
            if self.wave_dir == WaveDir::Vertical { "vertical" } else { "horizontal" },
            self.speed,
            self.brightness,
            self.fps,
            self.off_on_exit,
            self.ram_leds,
            self.headers[0].fmt(),
            self.headers[1].fmt(),
            self.headers[2].fmt(),
            fmt_place(&l.gpu),
            fmt_place(&l.ram[0]),
            fmt_place(&l.ram[1]),
            fmt_place(&l.mb_zones),
            fmt_place(&l.mb_led8),
            fmt_place(&l.header[0]),
            fmt_place(&l.header[1]),
            fmt_place(&l.header[2]),
        );
        std::fs::create_dir_all(dir())?;
        let tmp = path().with_extension("ini.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(tmp, path())
    }
}
