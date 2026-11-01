//! pc-rgb: a tiny tray app that drives this PC's RGB directly. No browser engine, no
//! services, no plugins, no telemetry.
#![windows_subsystem = "windows"]

mod autostart;
mod color;
mod config;
mod devices;
mod effects;
mod engine;
mod hid;
mod nvapi;
mod pawnio;
mod smbios;
mod tray;

use std::io::Write;
use std::sync::Mutex;
use windows_sys::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE, WAIT_ABANDONED, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::Threading::{CreateMutexW, GetCurrentProcess, OpenProcessToken, WaitForSingleObject};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

pub fn log(msg: &str) {
    if let Some(f) = LOG.lock().unwrap().as_mut() {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let _ = writeln!(f, "[{}] {msg}", t.as_secs());
    }
}

fn open_log(name: &str) {
    let _ = std::fs::create_dir_all(config::dir());
    let path = config::dir().join(name);
    // Keep one short session's worth of log.
    let f = std::fs::File::create(path).ok();
    *LOG.lock().unwrap() = f;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn is_elevated() -> bool {
    unsafe {
        let mut token: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
            return false;
        }
        let mut el = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut len = 0;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            &mut el as *mut _ as _,
            std::mem::size_of::<TOKEN_ELEVATION>() as u32,
            &mut len,
        );
        CloseHandle(token);
        ok != 0 && el.TokenIsElevated != 0
    }
}

/// Relaunch elevated (RAM lighting needs the SMBus driver). Returns true if the new process started.
pub fn relaunch_elevated() -> bool {
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe = wide(&exe.to_string_lossy());
    let r = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            wide("runas").as_ptr(),
            exe.as_ptr(),
            wide("--elevated").as_ptr(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    r as isize > 32
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |a: &str| args.iter().any(|x| x == a);

    // Detection only, no writes to hardware. Results go to probe.log.
    if has("--probe") {
        open_log("probe.log");
        log(&format!("probe, elevated={}", is_elevated()));
        log(&format!("dimms: {:?}", smbios::dimm_part_numbers()));
        for d in devices::detect(&config::Settings::load()) {
            log(&format!("found: {}", d.name()));
        }
        log("probe done");
        return;
    }

    // Ask for elevation once; if declined, run without RAM control.
    if !is_elevated() && !has("--no-elevate") && !has("--elevated") && relaunch_elevated() {
        return;
    }

    unsafe {
        let m = CreateMutexW(std::ptr::null(), 1, wide(r"Local\pc-rgb-single-instance").as_ptr());
        if m.is_null() {
            return;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // An elevated relaunch waits for the instance that spawned it to exit.
            let w = if has("--elevated") { WaitForSingleObject(m, 5000) } else { WAIT_TIMEOUT };
            if w != WAIT_OBJECT_0 && w != WAIT_ABANDONED {
                return;
            }
        }
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    open_log("pc-rgb.log");
    log(&format!("pc-rgb {} starting, elevated={}", env!("CARGO_PKG_VERSION"), is_elevated()));

    if !config::path().exists() {
        let _ = config::Settings::default().save();
    }
    let settings = config::Settings::load();
    tray::run(settings);
}
