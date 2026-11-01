//! Corsair DDR5 (Dominator / Vengeance) RGB over the AMD PIIX4 SMBus via PawnIO.
//! Protocol taken from SignalRGB's `Corsair_Dominator_Ram.js`.

use super::Device;
use crate::color::Rgb;
use crate::config::Settings;
use crate::pawnio::Piix4;
use std::io;

const CANDIDATES: [u8; 12] = [0x18, 0x19, 0x1A, 0x1B, 0x58, 0x59, 0x5A, 0x5B, 0x5C, 0x5D, 0x5E, 0x5F];
const REG_VENDOR: u8 = 0x43;
const REG_MODEL: u8 = 0x44;
const VENDOR_CORSAIR: u8 = 0x1B;
const MODELS: [u8; 2] = [0x03, 0x04];
const REG_DIRECT: u8 = 0x31;
const CMD_DIRECT: u8 = 0x0C;

pub struct CorsairRam {
    bus: Piix4,
    addrs: Vec<u8>,
    leds: usize,
    last: Vec<Option<Vec<Rgb>>>,
}

/// LEDs per stick by Corsair part number prefix (same table as SignalRGB).
fn leds_from_part_numbers() -> usize {
    for p in crate::smbios::dimm_part_numbers() {
        match p.get(..3) {
            Some("CMT") => return 12,
            Some("CMH") | Some("CMN") => return 10,
            Some("CMG") => return 6,
            _ => {}
        }
    }
    10
}

/// CRC-8, polynomial 0x07 (SMBus PEC).
fn crc8(data: &[u8]) -> u8 {
    data.iter().fold(0u8, |mut crc, &b| {
        crc ^= b;
        for _ in 0..8 {
            crc = if crc & 0x80 != 0 { (crc << 1) ^ 0x07 } else { crc << 1 };
        }
        crc
    })
}

impl CorsairRam {
    pub fn detect(s: &Settings) -> io::Result<Option<Self>> {
        if !crate::is_elevated() {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "not elevated, RAM lighting disabled"));
        }
        let mut bus = Piix4::open()?;
        let addrs = bus
            .locked(|b| {
                let mut found = Vec::new();
                for &a in &CANDIDATES {
                    if b.quick(a).is_err() {
                        continue;
                    }
                    let vendor_ok = (0..5).any(|_| b.read_byte_data(a, REG_VENDOR).ok() == Some(VENDOR_CORSAIR));
                    let model_ok =
                        vendor_ok && (0..5).any(|_| b.read_byte_data(a, REG_MODEL).is_ok_and(|m| MODELS.contains(&m)));
                    if model_ok {
                        found.push(a);
                    }
                }
                Ok(found)
            })?
            .unwrap_or_default();
        if addrs.is_empty() {
            return Ok(None);
        }
        let leds = if s.ram_leds > 0 { s.ram_leds as usize } else { leds_from_part_numbers() };
        crate::log(&format!("ram: Corsair DDR5 at {addrs:02x?}, {leds} LEDs each"));
        let last = vec![None; addrs.len()];
        Ok(Some(CorsairRam { bus, addrs, leds, last }))
    }
}

impl Device for CorsairRam {
    fn name(&self) -> &str {
        "Corsair DDR5"
    }

    fn init(&mut self) -> io::Result<()> {
        self.last.iter_mut().for_each(|l| *l = None);
        Ok(())
    }

    fn layout(&mut self, s: &Settings) -> Vec<(f32, f32)> {
        let mut v = Vec::with_capacity(self.addrs.len() * self.leds);
        for i in 0..self.addrs.len() {
            let p = s.layout.ram[i.min(1)];
            v.extend((0..self.leds).map(|y| p.at(0., y as f32)));
        }
        v
    }

    fn write(&mut self, colors: &[Rgb], force: bool) -> io::Result<()> {
        let leds = self.leds;
        let addrs = self.addrs.clone();
        let last = &mut self.last;
        self.bus
            .locked(|bus| {
                for (i, &addr) in addrs.iter().enumerate() {
                    let seg = &colors[i * leds..(i + 1) * leds];
                    if !force && last[i].as_deref() == Some(seg) {
                        continue;
                    }
                    let mut pkt = [0u8; 38];
                    pkt[0] = CMD_DIRECT;
                    for (j, c) in seg.iter().enumerate().take(12) {
                        pkt[1 + j * 3..4 + j * 3].copy_from_slice(c);
                    }
                    pkt[37] = crc8(&pkt[..37]);
                    bus.write_block_data(addr, REG_DIRECT, &pkt[..32])?;
                    bus.write_block_data(addr, REG_DIRECT + 1, &pkt[32..])?;
                    last[i] = Some(seg.to_vec());
                }
                Ok(())
            })
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn crc_matches_signalrgb_table() {
        // First entries of the PEC table in the plugin: 0, 7, 14, 9, 28 ...
        for (i, want) in [0u8, 7, 14, 9, 28, 27, 18, 21].into_iter().enumerate() {
            assert_eq!(super::crc8(&[i as u8]), want);
        }
    }
}
