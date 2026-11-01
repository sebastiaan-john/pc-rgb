//! NVIDIA GPU I2C via nvapi64.dll (user mode, no driver needed).

use std::ffi::c_void;
use std::io;
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};

type QueryInterface = unsafe extern "C" fn(u32) -> *mut c_void;
type FnInit = unsafe extern "C" fn() -> i32;
type FnEnum = unsafe extern "C" fn(*mut [*mut c_void; 64], *mut u32) -> i32;
type FnPciIds = unsafe extern "C" fn(*mut c_void, *mut u32, *mut u32, *mut u32, *mut u32) -> i32;
type FnI2c = unsafe extern "C" fn(*mut c_void, *mut I2cInfoV3, *mut u32) -> i32;

const ID_INITIALIZE: u32 = 0x0150_E828;
const ID_ENUM_PHYSICAL_GPUS: u32 = 0xE5AC_921F;
const ID_GPU_GET_PCI_IDENTIFIERS: u32 = 0x2DDF_B66E;
const ID_I2C_WRITE_EX: u32 = 0x283A_C65A;
const ID_I2C_READ_EX: u32 = 0x4D7B_0709;

#[repr(C)]
struct I2cInfoV3 {
    version: u32,
    display_mask: u32,
    is_ddc_port: u8,
    dev_address: u8,
    reg_address: *const u8,
    reg_addr_size: u32,
    data: *mut u8,
    size: u32,
    speed: u32,
    speed_khz: u32,
    port_id: u8,
    is_port_id_set: u32,
}

const I2C_INFO_VER3: u32 = std::mem::size_of::<I2cInfoV3>() as u32 | (3 << 16);

pub struct NvGpu {
    handle: *mut c_void,
    write_ex: FnI2c,
    read_ex: FnI2c,
    /// PCI ids: (device << 16) | vendor, (subdevice << 16) | subvendor.
    pub device_id: u32,
    pub subsys_id: u32,
}

unsafe impl Send for NvGpu {}

fn err(what: &str, status: i32) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("{what} failed (NvAPI status {status})"))
}

/// Enumerate NVIDIA GPUs. Returns an empty list if no NVIDIA driver is installed.
pub fn gpus() -> Vec<NvGpu> {
    unsafe {
        let name: Vec<u16> = "nvapi64.dll\0".encode_utf16().collect();
        let lib = LoadLibraryW(name.as_ptr());
        if lib.is_null() {
            return Vec::new();
        }
        let Some(qi) = GetProcAddress(lib, c"nvapi_QueryInterface".as_ptr() as _) else {
            return Vec::new();
        };
        let qi: QueryInterface = std::mem::transmute(qi);
        let get = |id: u32| {
            let p = qi(id);
            (!p.is_null()).then_some(p)
        };
        let (Some(init), Some(enum_gpus), Some(pci), Some(w), Some(r)) = (
            get(ID_INITIALIZE),
            get(ID_ENUM_PHYSICAL_GPUS),
            get(ID_GPU_GET_PCI_IDENTIFIERS),
            get(ID_I2C_WRITE_EX),
            get(ID_I2C_READ_EX),
        ) else {
            return Vec::new();
        };
        let init: FnInit = std::mem::transmute(init);
        let enum_gpus: FnEnum = std::mem::transmute(enum_gpus);
        let pci: FnPciIds = std::mem::transmute(pci);
        let write_ex: FnI2c = std::mem::transmute(w);
        let read_ex: FnI2c = std::mem::transmute(r);

        if init() != 0 {
            return Vec::new();
        }
        let mut handles = [std::ptr::null_mut(); 64];
        let mut count = 0u32;
        if enum_gpus(&mut handles, &mut count) != 0 {
            return Vec::new();
        }
        handles[..count.min(64) as usize]
            .iter()
            .filter_map(|&h| {
                let (mut dev, mut sub, mut rev, mut ext) = (0, 0, 0, 0);
                (pci(h, &mut dev, &mut sub, &mut rev, &mut ext) == 0).then(|| NvGpu {
                    handle: h,
                    write_ex,
                    read_ex,
                    device_id: dev,
                    subsys_id: sub,
                })
            })
            .collect()
    }
}

impl NvGpu {
    fn xfer(&self, write: bool, addr: u8, reg: u8, data: &mut [u8]) -> io::Result<()> {
        let reg = [reg];
        let mut info = I2cInfoV3 {
            version: I2C_INFO_VER3,
            display_mask: 0,
            is_ddc_port: 0,
            dev_address: (addr << 1) | (!write as u8),
            reg_address: reg.as_ptr(),
            reg_addr_size: 1,
            data: data.as_mut_ptr(),
            size: data.len() as u32,
            speed: 0xFFFF,  // NVAPI_I2C_SPEED_DEPRECATED
            speed_khz: 0,   // NVAPI_I2C_SPEED_DEFAULT
            port_id: 1,     // RGB controllers live on port 1
            is_port_id_set: 1,
        };
        let mut unknown = 0u32;
        let f = if write { self.write_ex } else { self.read_ex };
        let st = unsafe { f(self.handle, &mut info, &mut unknown) };
        if st == 0 {
            Ok(())
        } else {
            Err(err(if write { "I2CWriteEx" } else { "I2CReadEx" }, st))
        }
    }

    pub fn write_byte_data(&self, addr: u8, reg: u8, value: u8) -> io::Result<()> {
        self.xfer(true, addr, reg, &mut [value])
    }

    /// Raw I2C block write (register, then payload; no SMBus length byte).
    pub fn write_i2c_block(&self, addr: u8, reg: u8, data: &mut [u8]) -> io::Result<()> {
        self.xfer(true, addr, reg, data)
    }
}
