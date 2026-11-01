//! AMD PIIX4 SMBus access through the PawnIO driver (signed, already installed on this box).
//! Requires administrator rights.

use std::ffi::c_void;
use std::io;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0};
use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

/// Signed PawnIO module (LGPL-2.1, https://github.com/namazso/PawnIO.Modules).
static PIIX4_MODULE: &[u8] = include_bytes!("../vendor/pawnio/SmbusPIIX4.bin");

type FnOpen = unsafe extern "system" fn(*mut HANDLE) -> i32;
type FnLoad = unsafe extern "system" fn(HANDLE, *const u8, usize) -> i32;
type FnExec = unsafe extern "system" fn(HANDLE, *const u8, *const u64, usize, *mut u64, usize, *mut usize) -> i32;
type FnClose = unsafe extern "system" fn(HANDLE) -> i32;

const SMBUS_WRITE: u64 = 0;
const SMBUS_READ: u64 = 1;
const PROTO_QUICK: u64 = 0;
const PROTO_BYTE_DATA: u64 = 2;
const PROTO_BLOCK_DATA: u64 = 5;
const SLEEP_MODE_SHORT_BUSY: u64 = 1;

pub struct Piix4 {
    handle: HANDLE,
    exec: FnExec,
    close: FnClose,
    /// Cross-vendor SMBus mutex honoured by HWiNFO, OpenRGB, LibreHardwareMonitor, etc.
    mutex: HANDLE,
}

unsafe impl Send for Piix4 {}

impl Drop for Piix4 {
    fn drop(&mut self) {
        unsafe {
            (self.close)(self.handle);
            CloseHandle(self.mutex);
        }
    }
}

fn hr(what: &str, code: i32) -> io::Result<()> {
    if code >= 0 {
        Ok(())
    } else {
        Err(io::Error::new(io::ErrorKind::Other, format!("PawnIO {what} failed (0x{:08x})", code as u32)))
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe fn load_lib() -> Option<*mut c_void> {
    let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
    for p in [format!(r"{pf}\PawnIO\PawnIOLib.dll"), "PawnIOLib.dll".into()] {
        let h = LoadLibraryW(wide(&p).as_ptr());
        if !h.is_null() {
            return Some(h);
        }
    }
    None
}

impl Piix4 {
    pub fn open() -> io::Result<Self> {
        unsafe {
            let lib = load_lib().ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "PawnIO is not installed"))?;
            let sym = |n: &std::ffi::CStr| GetProcAddress(lib, n.as_ptr() as _);
            let (Some(o), Some(l), Some(e), Some(c)) =
                (sym(c"pawnio_open"), sym(c"pawnio_load"), sym(c"pawnio_execute"), sym(c"pawnio_close"))
            else {
                return Err(io::Error::new(io::ErrorKind::NotFound, "PawnIOLib.dll is missing exports"));
            };
            let open: FnOpen = std::mem::transmute(o);
            let load: FnLoad = std::mem::transmute(l);
            let exec: FnExec = std::mem::transmute(e);
            let close: FnClose = std::mem::transmute(c);

            let mut handle: HANDLE = std::ptr::null_mut();
            hr("open (run as administrator?)", open(&mut handle))?;
            if let Err(e) = hr("load SmbusPIIX4", load(handle, PIIX4_MODULE.as_ptr(), PIIX4_MODULE.len())) {
                close(handle);
                return Err(e);
            }
            let mutex = CreateMutexW(std::ptr::null(), 0, wide(r"Global\Access_SMBUS.HTP.Method").as_ptr());
            let bus = Piix4 { handle, exec, close, mutex };
            // The module busy-waits for the whole transfer by default (~4 ms of 100% CPU per
            // DIMM update). ShortBusy sleeps for the bulk of it and only spins on status polls.
            if let Err(e) = bus.exec(c"ioctl_set_sleep_mode", &[SLEEP_MODE_SHORT_BUSY], &mut []) {
                crate::log(&format!("pawnio: could not set sleep mode: {e}"));
            }
            Ok(bus)
        }
    }

    fn exec(&self, name: &std::ffi::CStr, input: &[u64], out: &mut [u64]) -> io::Result<()> {
        let mut ret = 0usize;
        let code = unsafe {
            (self.exec)(
                self.handle,
                name.as_ptr() as *const u8,
                input.as_ptr(),
                input.len(),
                out.as_mut_ptr(),
                out.len(),
                &mut ret,
            )
        };
        hr("execute", code)
    }

    /// Run `f` while holding the shared SMBus mutex. Returns `Ok(None)` if the bus stayed
    /// busy (another tool is mid-transaction); the caller simply retries next frame.
    pub fn locked<R>(&mut self, f: impl FnOnce(&mut Self) -> io::Result<R>) -> io::Result<Option<R>> {
        if self.mutex.is_null() {
            return f(self).map(Some);
        }
        let w = unsafe { WaitForSingleObject(self.mutex, 100) };
        if w != WAIT_OBJECT_0 && w != WAIT_ABANDONED {
            return Ok(None);
        }
        // Make sure the primary bus is on port 0 (where DIMMs live) in case another tool moved it.
        let r = self.exec(c"ioctl_piix4_port_sel", &[0], &mut [0]).and_then(|_| f(self));
        unsafe { ReleaseMutex(self.mutex) };
        r.map(Some)
    }

    pub fn quick(&self, addr: u8) -> io::Result<()> {
        self.exec(c"ioctl_smbus_xfer", &[addr as u64, SMBUS_WRITE, 0, PROTO_QUICK], &mut [])
    }

    pub fn read_byte_data(&self, addr: u8, cmd: u8) -> io::Result<u8> {
        let mut out = [0u64; 1];
        self.exec(c"ioctl_smbus_xfer", &[addr as u64, SMBUS_READ, cmd as u64, PROTO_BYTE_DATA], &mut out)?;
        Ok(out[0] as u8)
    }

    /// SMBus block write, up to 32 bytes.
    pub fn write_block_data(&self, addr: u8, cmd: u8, data: &[u8]) -> io::Result<()> {
        if data.is_empty() || data.len() > 32 {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "block must be 1..=32 bytes"));
        }
        let mut bytes = [0u8; 40];
        bytes[0] = data.len() as u8;
        bytes[1..=data.len()].copy_from_slice(data);
        let mut input = [addr as u64, SMBUS_WRITE, cmd as u64, PROTO_BLOCK_DATA, 0, 0, 0, 0, 0];
        for (i, chunk) in bytes.chunks_exact(8).enumerate() {
            input[4 + i] = u64::from_le_bytes(chunk.try_into().unwrap());
        }
        self.exec(c"ioctl_smbus_xfer", &input, &mut [])
    }
}
