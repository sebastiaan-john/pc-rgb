//! Gigabyte RGB Fusion 2 USB controller, ITE IT5711 (048d:5711), as on the X870 AORUS ELITE WIFI7.
//! Protocol taken from SignalRGB's `Gigabyte_Motherboard_Controller_0x5711.js`.

use super::Device;
use crate::color::{Order, Rgb};
use crate::config::{Header, Settings};
use crate::hid::{self, HidDevice};
use std::io;

const VID: u16 = 0x048D;
const PID: u16 = 0x5711;
const CMD: u8 = 0xCC;
const CMD_COMMIT: u8 = 0x28;
const CMD_SOFTWARE_MODE: u8 = 0x32;
const CMD_ARGB_LED_COUNTS: u8 = 0x34;

/// Onboard zones (report id, color order). Order matches SignalRGB's "Auto" config:
/// Led 1, Led 2, Led 3, 12V header, Led 4, Led 5, Led 6, Led 7, Led 8.
const ZONES: [(u8, Order); 9] = [
    (0x20, Order::Bgr),
    (0x22, Order::Bgr),
    (0x23, Order::Bgr),
    (0x24, Order::Rgb),
    (0x25, Order::Bgr),
    (0x26, Order::Bgr),
    (0x27, Order::Bgr),
    (0x91, Order::Bgr),
    (0x92, Order::Bgr),
];
const LED8: usize = 8;

/// 5V ARGB headers: direct-mode channel ids.
const HEADERS: [u8; 3] = [0x58, 0x59, 0x62];
const ARGB_ORDER: Order = Order::Grb;
const LEDS_PER_PACKET: usize = 19;

/// Corsair ML fan LED positions on its 4x4 grid, in hardware order (a clockwise ring).
const ML_FAN: [(f32, f32); 8] =
    [(3., 1.), (3., 2.), (2., 3.), (1., 3.), (0., 2.), (0., 1.), (1., 0.), (2., 0.)];

pub struct Gigabyte {
    dev: HidDevice,
    headers: [Header; 3],
    counts_sent: Option<[Header; 3]>,
    last: Vec<Rgb>,
}

fn led_count_enum(n: usize) -> u8 {
    match n {
        0..=32 => 0,
        33..=64 => 1,
        65..=256 => 2,
        257..=512 => 3,
        _ => 4,
    }
}

impl Gigabyte {
    pub fn open() -> io::Result<Option<Self>> {
        let Some(info) = hid::enumerate(VID, PID).into_iter().find(|i| i.usage == 0xCC) else {
            return Ok(None);
        };
        let dev = HidDevice::open(&info)?;
        Ok(Some(Gigabyte { dev, headers: [Header::None; 3], counts_sent: None, last: Vec::new() }))
    }

    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.dev.set_feature(data)
    }

    fn commit(&mut self) -> io::Result<()> {
        self.send(&[CMD, CMD_COMMIT, 0xFF, 0x07])
    }

    fn send_led_counts(&mut self) -> io::Result<()> {
        let e: Vec<u8> = self.headers.iter().map(|h| led_count_enum(h.led_count())).collect();
        self.send(&[CMD, CMD_ARGB_LED_COUNTS, e[0] | (e[1] << 4), e[2]])?;
        self.counts_sent = Some(self.headers);
        Ok(())
    }

    fn zone_packet(zone: u8, c: Rgb) -> [u8; 17] {
        let mut p = [0u8; 17];
        p[0] = CMD;
        p[1] = zone;
        // 16-bit zone mask, little endian.
        match zone {
            0x20..=0x27 => p[2] = 1 << (zone - 0x20),
            0x91 => p[3] = 0x02,
            0x92 => p[3] = 0x04,
            _ => {}
        }
        p[0x0B] = 1; // static mode
        p[0x0C] = 0x5A; // max brightness
        p[0x0E..0x11].copy_from_slice(&c);
        p
    }

    fn stream_header(&mut self, ch: u8, colors: &[Rgb]) -> io::Result<()> {
        let mut pkt = [0u8; 64];
        for (i, chunk) in colors.chunks(LEDS_PER_PACKET).enumerate() {
            let start = i * LEDS_PER_PACKET * 3;
            pkt[0] = CMD;
            pkt[1] = ch;
            pkt[2] = start as u8;
            pkt[3] = (start >> 8) as u8;
            pkt[4] = (chunk.len() * 3) as u8;
            for (j, &c) in chunk.iter().enumerate() {
                pkt[5 + j * 3..8 + j * 3].copy_from_slice(&ARGB_ORDER.apply(c));
            }
            let n = 5 + chunk.len() * 3;
            self.send(&pkt[..n])?;
        }
        Ok(())
    }
}

impl Device for Gigabyte {
    fn name(&self) -> &str {
        "Gigabyte motherboard"
    }

    fn init(&mut self) -> io::Result<()> {
        // Reopen: after sleep/resume or a USB reset the old handle is dead.
        if let Some(info) = hid::enumerate(VID, PID).into_iter().find(|i| i.usage == 0xCC) {
            self.dev = HidDevice::open(&info)?;
        }
        // Config table: firmware version + description string ("IT5711-GIGABYTE V1.0.19.5").
        self.send(&[CMD, 0x60])?;
        if let Ok(cfg) = self.dev.get_feature(CMD) {
            let desc: String = cfg.get(12..40).unwrap_or(&[]).iter().take_while(|&&b| b != 0).map(|&b| b as char).collect();
            crate::log(&format!("gigabyte: fw {}.{}.{}.{} '{}'", cfg[4], cfg[5], cfg[6], cfg[7], desc.trim()));
        }
        // Reset all onboard zones, then enable direct mode on the ARGB headers.
        for z in (0x20..=0x27).chain(0x90..=0x92) {
            self.send(&[CMD, z])?;
        }
        self.commit()?;
        self.send(&[CMD, CMD_SOFTWARE_MODE, 0x0B])?;
        self.send_led_counts()?;
        self.last.clear();
        Ok(())
    }

    fn layout(&mut self, s: &Settings) -> Vec<(f32, f32)> {
        self.headers = s.headers;
        let l = &s.layout;
        let mut v: Vec<(f32, f32)> =
            (0..ZONES.len()).map(|i| if i == LED8 { l.mb_led8.at(1., 1.) } else { l.mb_zones.at(1., 1.) }).collect();
        for (h, p) in s.headers.iter().zip(&l.header) {
            match *h {
                Header::None => {}
                Header::Strip(n) => v.extend((0..n).map(|i| p.at(i as f32, 0.))),
                Header::MlFan => v.extend(ML_FAN.iter().map(|&(x, y)| p.at(x, y))),
            }
        }
        v
    }

    fn write(&mut self, colors: &[Rgb], force: bool) -> io::Result<()> {
        if self.counts_sent != Some(self.headers) {
            self.send_led_counts()?;
        }
        let fresh = force || self.last.len() != colors.len();

        let mut dirty = false;
        for (i, &(zone, order)) in ZONES.iter().enumerate() {
            if fresh || self.last[i] != colors[i] {
                self.send(&Self::zone_packet(zone, order.apply(colors[i])))?;
                dirty = true;
            }
        }
        if dirty {
            self.commit()?;
        }

        let mut off = ZONES.len();
        let headers = self.headers;
        for (h, &ch) in headers.iter().zip(&HEADERS) {
            let n = h.led_count();
            let seg = &colors[off..off + n];
            if n > 0 && (fresh || self.last[off..off + n] != *seg) {
                self.stream_header(ch, seg)?;
            }
            off += n;
        }

        self.last.clear();
        self.last.extend_from_slice(colors);
        Ok(())
    }
}
