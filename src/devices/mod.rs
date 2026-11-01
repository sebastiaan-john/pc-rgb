pub mod corsair_ram;
pub mod gigabyte;
pub mod msi_gpu;

use crate::color::Rgb;
use crate::config::Settings;
use std::io;

pub trait Device: Send {
    fn name(&self) -> &str;
    /// Put the hardware into direct/software control. Called at start and after resume.
    fn init(&mut self) -> io::Result<()>;
    /// Canvas positions of every LED, in the order `write` expects colors.
    /// May also pick up hardware config from settings (e.g. ARGB header LED counts).
    fn layout(&mut self, s: &Settings) -> Vec<(f32, f32)>;
    /// Push colors. `force` resends everything, otherwise unchanged parts may be skipped.
    fn write(&mut self, colors: &[Rgb], force: bool) -> io::Result<()>;
}

/// Find everything we know how to drive. Each returned device gets its own worker thread.
pub fn detect(s: &Settings) -> Vec<Box<dyn Device>> {
    let mut out: Vec<Box<dyn Device>> = Vec::new();
    match gigabyte::Gigabyte::open() {
        Ok(Some(d)) => out.push(Box::new(d)),
        Ok(None) => {}
        Err(e) => crate::log(&format!("gigabyte: {e}")),
    }
    for d in msi_gpu::MsiGpu::detect() {
        out.push(Box::new(d));
    }
    match corsair_ram::CorsairRam::detect(s) {
        Ok(Some(d)) => out.push(Box::new(d)),
        Ok(None) => {}
        Err(e) => crate::log(&format!("ram: {e}")),
    }
    out
}
