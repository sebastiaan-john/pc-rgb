//! MSI RTX 50-series GPUs over NVIDIA I2C, address 0x68.
//! Protocol taken from SignalRGB's `MSI_GPU_Blackwell.js` (per-LED mode).

use super::Device;
use crate::color::Rgb;
use crate::config::Settings;
use crate::nvapi::{self, NvGpu};
use std::io;

const ADDR: u8 = 0x68;
const NV_VENDOR: u32 = 0x10DE;
const MSI_SUBVENDOR: u32 = 0x1462;

struct Model {
    name: &'static str,
    /// (hardware LED index, grid x, grid y)
    leds: &'static [(u8, f32, f32)],
}

const GAMING_TRIO: Model = Model {
    name: "MSI GAMING TRIO",
    leds: &[(0, 0., 0.), (1, 1., 0.), (2, 2., 0.), (3, 8., 0.), (4, 9., 0.), (5, 7., 1.)],
};

const SUPRIM: Model = Model {
    name: "MSI SUPRIM",
    leds: &[
        (0, 0., 0.), (1, 1., 0.), (2, 2., 0.), (3, 3., 0.),
        (4, 1., 2.), (5, 2., 3.), (6, 3., 4.), (7, 4., 5.), (8, 5., 6.), (9, 6., 7.),
        (10, 5., 8.), (11, 4., 9.), (12, 3., 10.), (13, 2., 11.), (14, 1., 12.),
    ],
};

const VANGUARD: Model = Model {
    name: "MSI VANGUARD",
    leds: &[
        (0, 10., 0.), (1, 11., 0.), (2, 12., 0.), (3, 13., 0.), (4, 14., 0.), (5, 15., 0.), (6, 16., 0.),
        (7, 17., 0.), (11, 18., 0.), (8, 14., 1.), (9, 15., 1.), (10, 16., 1.), (12, 18., 1.),
        (18, 5., 4.), (21, 10., 4.), (22, 11., 4.), (15, 2., 5.), (16, 3., 5.), (17, 4., 5.),
        (19, 8., 5.), (20, 9., 5.), (13, 0., 6.), (14, 1., 6.),
    ],
};

/// (PCI device id, MSI subsystem id) -> model.
fn model_for(device: u32, sub: u32) -> Option<&'static Model> {
    Some(match (device, sub) {
        (0x2D05, 0x5372) => &GAMING_TRIO,                       // 5060
        (0x2D04, 0x5353 | 0x5360 | 0x5361 | 0x5354) => &GAMING_TRIO, // 5060 Ti
        (0x2D04, 0x5352) => &VANGUARD,
        (0x2F04, 0x5322) => &GAMING_TRIO,                       // 5070
        (0x2F04, 0x5321) => &VANGUARD,
        (0x2C05, 0x5315 | 0x5318) => &GAMING_TRIO,              // 5070 Ti
        (0x2C05, 0x5314) => &VANGUARD,
        (0x2C02, 0x5315) => &GAMING_TRIO,                       // 5080
        (0x2C02, 0x5314) => &VANGUARD,
        (0x2C02, 0x5313) => &SUPRIM,
        (0x2B85, 0x5303) => &GAMING_TRIO,                       // 5090
        (0x2B85, 0x5301) => &SUPRIM,
        (0x2B85, 0x5302) => &VANGUARD,
        _ => return None,
    })
}

pub struct MsiGpu {
    gpu: NvGpu,
    model: &'static Model,
    buf: Vec<u8>,
    last: Vec<Rgb>,
}

impl MsiGpu {
    pub fn detect() -> Vec<MsiGpu> {
        nvapi::gpus()
            .into_iter()
            .filter_map(|gpu| {
                let (vendor, device) = (gpu.device_id & 0xFFFF, gpu.device_id >> 16);
                let (subvendor, sub) = (gpu.subsys_id & 0xFFFF, gpu.subsys_id >> 16);
                if vendor != NV_VENDOR || subvendor != MSI_SUBVENDOR {
                    return None;
                }
                let model = model_for(device, sub)?;
                Some(MsiGpu { gpu, model, buf: Vec::new(), last: Vec::new() })
            })
            .collect()
    }
}

impl Device for MsiGpu {
    fn name(&self) -> &str {
        self.model.name
    }

    fn init(&mut self) -> io::Result<()> {
        // Per-LED direct mode.
        self.last.clear();
        self.gpu.write_byte_data(ADDR, 0x46, 0x01)?;
        self.gpu.write_byte_data(ADDR, 0x06, 0x00)?;
        self.gpu.write_byte_data(ADDR, 0x20, 0x00)
    }

    fn layout(&mut self, s: &Settings) -> Vec<(f32, f32)> {
        self.model.leds.iter().map(|&(_, x, y)| s.layout.gpu.at(x, y)).collect()
    }

    fn write(&mut self, colors: &[Rgb], force: bool) -> io::Result<()> {
        if !force && self.last == colors {
            return Ok(());
        }
        // [0x01] then 5 bytes per LED slot: index, r, g, b, 0x01.
        let slots = self.model.leds.iter().map(|l| l.0 as usize + 1).max().unwrap_or(0);
        self.buf.clear();
        self.buf.resize(1 + slots * 5, 0);
        self.buf[0] = 0x01;
        for (&(idx, _, _), c) in self.model.leds.iter().zip(colors) {
            let o = idx as usize * 5 + 1;
            self.buf[o] = idx;
            self.buf[o + 1..o + 4].copy_from_slice(c);
            self.buf[o + 4] = 0x01;
        }
        self.gpu.write_i2c_block(ADDR, 0x04, &mut self.buf)?;
        self.last.clear();
        self.last.extend_from_slice(colors);
        Ok(())
    }
}
