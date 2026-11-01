//! Tray icon + popup menu. Plain Win32, a hidden window, nothing else.

use crate::color::{hsl, to_hsl, Rgb};
use crate::config::{Effect, Settings, WaveDir};
use crate::engine::Engine;
use crate::{autostart, config, log, wide};
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::Dialogs::*;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const WM_TRAY: u32 = WM_APP + 1;
const WM_CONFIG_CHANGED: u32 = WM_APP + 2;
const PBT_APMSUSPEND: usize = 0x4;
const PBT_APMRESUMESUSPEND: usize = 0x7;
const PBT_APMRESUMEAUTOMATIC: usize = 0x12;

const ID_EFFECT: usize = 100;
const ID_PICK_COLOR: usize = 200;
const ID_PRESET: usize = 300;
const ID_SPEED: usize = 400;
const ID_BRIGHT: usize = 500;
const ID_WAVE_H: usize = 600;
const ID_WAVE_V: usize = 601;
const ID_AUTOSTART: usize = 700;
const ID_EDIT_CONFIG: usize = 701;
const ID_RELOAD: usize = 702;
const ID_REDETECT: usize = 703;
const ID_ELEVATE: usize = 704;
const ID_EXIT: usize = 799;

const PRESETS: [(&str, Rgb); 10] = [
    ("White", [255, 255, 255]),
    ("Warm white", [255, 170, 90]),
    ("Red", [255, 0, 0]),
    ("Orange", [255, 80, 0]),
    ("Yellow", [255, 200, 0]),
    ("Green", [0, 255, 0]),
    ("Cyan", [0, 255, 255]),
    ("Blue", [0, 0, 255]),
    ("Purple", [140, 0, 255]),
    ("Pink", [255, 0, 140]),
];
const SPEEDS: [u8; 7] = [5, 10, 20, 35, 50, 75, 100];
const BRIGHTNESS: [u8; 6] = [100, 80, 60, 40, 20, 10];

struct App {
    hwnd: HWND,
    engine: Option<Engine>,
    settings: Settings,
    icon: HICON,
    autostart: bool,
}

thread_local! {
    static APP: RefCell<Option<App>> = const { RefCell::new(None) };
    static CUSTOM_COLORS: RefCell<[u32; 16]> = const { RefCell::new([0x00FF_FFFF; 16]) };
}

fn with_app<R>(f: impl FnOnce(&mut App) -> R) -> Option<R> {
    APP.with(|a| a.borrow_mut().as_mut().map(f))
}

pub fn run(settings: Settings) {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let class = wide("pc-rgb-tray");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            lpszClassName: class.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassW(&wc);
        // A real (hidden) top-level window: message-only windows don't get WM_POWERBROADCAST.
        let hwnd = CreateWindowExW(0, class.as_ptr(), wide("pc-rgb").as_ptr(), WS_OVERLAPPED, 0, 0, 0, 0, null_mut(), null_mut(), hinst, null());
        if hwnd.is_null() {
            log("CreateWindowExW failed");
            return;
        }

        let engine = Engine::start(settings);
        let autostart = autostart::enabled();
        APP.with(|a| {
            *a.borrow_mut() = Some(App { hwnd, engine: Some(engine), settings, icon: null_mut(), autostart })
        });
        with_app(|app| app.refresh_icon(NIM_ADD));
        watch_config(hwnd);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn taskbar_created() -> u32 {
    use std::sync::OnceLock;
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| unsafe { RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()) })
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY => {
            let ev = lp as u32 & 0xFFFF;
            if ev == WM_RBUTTONUP || ev == WM_LBUTTONUP {
                show_menu(hwnd);
            }
            0
        }
        WM_CONFIG_CHANGED => {
            reload_config(false);
            0
        }
        WM_POWERBROADCAST => {
            match wp {
                PBT_APMSUSPEND => {
                    log("suspend");
                    with_app(|a| a.engine.as_ref().map(|e| e.set_suspended(true)));
                    // Give workers a moment to blank the LEDs before the system sleeps.
                    std::thread::sleep(std::time::Duration::from_millis(300));
                }
                PBT_APMRESUMESUSPEND | PBT_APMRESUMEAUTOMATIC => {
                    log("resume");
                    with_app(|a| a.engine.as_ref().map(|e| e.set_suspended(false)));
                }
                _ => {}
            }
            1
        }
        WM_QUERYENDSESSION => 1,
        WM_ENDSESSION => {
            if wp != 0 {
                shutdown();
            }
            0
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            0
        }
        m if m == taskbar_created() && m != 0 => {
            with_app(|a| a.refresh_icon(NIM_ADD));
            0
        }
        _ => DefWindowProcW(hwnd, msg, wp, lp),
    }
}

fn shutdown() {
    let engine = with_app(|a| {
        a.remove_icon();
        a.engine.take()
    })
    .flatten();
    if let Some(e) = engine {
        e.stop(true);
    }
    log("exit");
}

impl App {
    fn apply(&mut self, f: impl FnOnce(&mut Settings)) {
        f(&mut self.settings);
        if let Some(e) = &self.engine {
            e.set_settings(self.settings);
        }
        if let Err(e) = self.settings.save() {
            log(&format!("saving config failed: {e}"));
        }
        self.refresh_icon(NIM_MODIFY);
    }

    fn remove_icon(&mut self) {
        unsafe {
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            Shell_NotifyIconW(NIM_DELETE, &nid);
        }
    }

    fn refresh_icon(&mut self, op: NOTIFY_ICON_MESSAGE) {
        unsafe {
            let old = self.icon;
            self.icon = make_icon(&self.settings);
            let mut nid: NOTIFYICONDATAW = std::mem::zeroed();
            nid.cbSize = std::mem::size_of::<NOTIFYICONDATAW>() as u32;
            nid.hWnd = self.hwnd;
            nid.uID = 1;
            nid.uFlags = NIF_MESSAGE | NIF_ICON | NIF_TIP;
            nid.uCallbackMessage = WM_TRAY;
            nid.hIcon = self.icon;
            let tip = format!("pc-rgb: {}", self.settings.effect.label());
            for (d, s) in nid.szTip.iter_mut().zip(tip.encode_utf16().take(127)) {
                *d = s;
            }
            if Shell_NotifyIconW(op, &nid) == 0 && op == NIM_MODIFY {
                Shell_NotifyIconW(NIM_ADD, &nid);
            }
            if !old.is_null() {
                DestroyIcon(old);
            }
        }
    }
}

/// Draw a round tray icon reflecting the current effect.
unsafe fn make_icon(s: &Settings) -> HICON {
    let size = GetSystemMetrics(SM_CXSMICON).clamp(16, 64);
    let mut bmi: BITMAPINFO = std::mem::zeroed();
    bmi.bmiHeader.biSize = std::mem::size_of::<BITMAPINFOHEADER>() as u32;
    bmi.bmiHeader.biWidth = size;
    bmi.bmiHeader.biHeight = -size;
    bmi.bmiHeader.biPlanes = 1;
    bmi.bmiHeader.biBitCount = 32;
    bmi.bmiHeader.biCompression = BI_RGB;
    let mut bits: *mut core::ffi::c_void = null_mut();
    let hdc = GetDC(null_mut());
    let color = CreateDIBSection(hdc, &bmi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    ReleaseDC(null_mut(), hdc);
    if color.is_null() || bits.is_null() {
        return LoadIconW(null_mut(), IDI_APPLICATION);
    }
    let px = std::slice::from_raw_parts_mut(bits as *mut u32, (size * size) as usize);
    let c = size as f32 / 2.0;
    let r_out = c - 0.5;
    let r_in = if s.effect == Effect::Off { r_out - size as f32 * 0.14 } else { -1.0 };
    let (ph, psat, pl) = to_hsl(s.pastel_color);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 + 0.5 - c, y as f32 + 0.5 - c);
            let d = (dx * dx + dy * dy).sqrt();
            let a = ((r_out - d + 0.5).clamp(0.0, 1.0) * (d - r_in + 0.5).clamp(0.0, 1.0) * 255.0) as u32;
            let ang = dy.atan2(dx).to_degrees();
            let rgb = match s.effect {
                Effect::Solid | Effect::Breathing => s.color,
                Effect::Pastel => hsl(ph + ang, psat, pl),
                Effect::Rainbow => hsl(ang, 100.0, 50.0),
                Effect::Off => [120, 120, 120],
            };
            px[(y * size + x) as usize] = (a << 24) | (rgb[0] as u32) << 16 | (rgb[1] as u32) << 8 | rgb[2] as u32;
        }
    }
    let mask_bits = vec![0u8; ((size as usize + 15) / 16 * 2) * size as usize];
    let mask = CreateBitmap(size, size, 1, 1, mask_bits.as_ptr() as _);
    let ii = ICONINFO { fIcon: TRUE, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
    let icon = CreateIconIndirect(&ii);
    DeleteObject(color);
    DeleteObject(mask);
    icon
}

unsafe fn add(menu: HMENU, id: usize, text: &str, checked: bool, enabled: bool) {
    let mut flags = MF_STRING;
    if checked {
        flags |= MF_CHECKED;
    }
    if !enabled {
        flags |= MF_GRAYED;
    }
    AppendMenuW(menu, flags, id, wide(text).as_ptr());
}

unsafe fn sep(menu: HMENU) {
    AppendMenuW(menu, MF_SEPARATOR, 0, null());
}

unsafe fn sub(menu: HMENU, text: &str) -> HMENU {
    let m = CreatePopupMenu();
    AppendMenuW(menu, MF_POPUP, m as usize, wide(text).as_ptr());
    m
}

fn show_menu(hwnd: HWND) {
    let Some((s, devices, autostart)) =
        with_app(|a| (a.settings, a.engine.as_ref().map(|e| e.devices()).unwrap_or_default(), a.autostart))
    else {
        return;
    };
    let cmd = unsafe {
        let m = CreatePopupMenu();
        let title = match devices.len() {
            0 => "pc-rgb: no devices found".to_string(),
            n => format!("pc-rgb: {n} device{}", if n == 1 { "" } else { "s" }),
        };
        add(m, 0, &title, false, false);
        for d in &devices {
            add(m, 0, &format!("    {d}"), false, false);
        }
        if !crate::is_elevated() {
            add(m, ID_ELEVATE, "Restart as admin (enables RAM)", false, true);
        }
        sep(m);
        for (i, e) in Effect::ALL.iter().enumerate() {
            add(m, ID_EFFECT + i, e.label(), s.effect == *e, true);
        }
        sep(m);
        let what = if s.effect == Effect::Pastel { "Pastel base color..." } else { "Pick color..." };
        add(m, ID_PICK_COLOR, what, false, true);
        let pm = sub(m, "Quick colors");
        for (i, (name, c)) in PRESETS.iter().enumerate() {
            add(pm, ID_PRESET + i, name, s.effect == Effect::Solid && s.color == *c, true);
        }
        let sm = sub(m, "Speed");
        for (i, v) in SPEEDS.iter().enumerate() {
            add(sm, ID_SPEED + i, &v.to_string(), s.speed == *v, true);
        }
        let bm = sub(m, "Brightness");
        for (i, v) in BRIGHTNESS.iter().enumerate() {
            add(bm, ID_BRIGHT + i, &format!("{v}%"), s.brightness == *v, true);
        }
        let wm = sub(m, "Wave direction");
        add(wm, ID_WAVE_H, "Horizontal", s.wave_dir == WaveDir::Horizontal, true);
        add(wm, ID_WAVE_V, "Vertical", s.wave_dir == WaveDir::Vertical, true);
        sep(m);
        add(m, ID_AUTOSTART, "Start with Windows", autostart, true);
        add(m, ID_EDIT_CONFIG, "Edit config file", false, true);
        add(m, ID_RELOAD, "Reload config", false, true);
        add(m, ID_REDETECT, "Re-detect devices", false, true);
        sep(m);
        add(m, ID_EXIT, "Exit", false, true);

        let mut pt = POINT { x: 0, y: 0 };
        GetCursorPos(&mut pt);
        SetForegroundWindow(hwnd);
        let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON | TPM_NONOTIFY, pt.x, pt.y, 0, hwnd, null());
        PostMessageW(hwnd, WM_NULL, 0, 0);
        DestroyMenu(m);
        cmd as usize
    };
    if cmd != 0 {
        command(hwnd, cmd);
    }
}

fn command(hwnd: HWND, cmd: usize) {
    match cmd {
        c if (ID_EFFECT..ID_EFFECT + Effect::ALL.len()).contains(&c) => {
            with_app(|a| a.apply(|s| s.effect = Effect::ALL[c - ID_EFFECT]));
        }
        c if (ID_PRESET..ID_PRESET + PRESETS.len()).contains(&c) => {
            with_app(|a| {
                a.apply(|s| {
                    s.color = PRESETS[c - ID_PRESET].1;
                    s.effect = Effect::Solid;
                })
            });
        }
        c if (ID_SPEED..ID_SPEED + SPEEDS.len()).contains(&c) => {
            with_app(|a| a.apply(|s| s.speed = SPEEDS[c - ID_SPEED]));
        }
        c if (ID_BRIGHT..ID_BRIGHT + BRIGHTNESS.len()).contains(&c) => {
            with_app(|a| a.apply(|s| s.brightness = BRIGHTNESS[c - ID_BRIGHT]));
        }
        ID_WAVE_H => {
            with_app(|a| a.apply(|s| s.wave_dir = WaveDir::Horizontal));
        }
        ID_WAVE_V => {
            with_app(|a| a.apply(|s| s.wave_dir = WaveDir::Vertical));
        }
        ID_PICK_COLOR => pick_color(hwnd),
        ID_AUTOSTART => {
            let on = !with_app(|a| a.autostart).unwrap_or(false);
            let ok = autostart::set(on);
            let now = autostart::enabled();
            with_app(|a| a.autostart = now);
            if !ok {
                message(hwnd, "Could not change the startup task. pc-rgb must run as administrator for this.");
            }
        }
        ID_EDIT_CONFIG => unsafe {
            let p = wide(&config::path().to_string_lossy());
            ShellExecuteW(hwnd, wide("open").as_ptr(), wide("notepad.exe").as_ptr(), p.as_ptr(), null(), SW_SHOWNORMAL);
        },
        ID_RELOAD => reload_config(true),
        ID_REDETECT => {
            if let Some(e) = with_app(|a| a.engine.take()).flatten() {
                e.stop(false);
            }
            with_app(|a| a.engine = Some(Engine::start(a.settings)));
        }
        ID_ELEVATE => {
            // Release our engine first so the new instance can own the hardware.
            if crate::relaunch_elevated() {
                shutdown_quietly();
                unsafe { DestroyWindow(hwnd) };
            }
        }
        ID_EXIT => {
            shutdown();
            unsafe { DestroyWindow(hwnd) };
        }
        _ => {}
    }
}

/// Exit without blanking the LEDs (used when handing over to an elevated instance).
fn shutdown_quietly() {
    if let Some(e) = with_app(|a| {
        a.remove_icon();
        a.engine.take()
    })
    .flatten()
    {
        e.stop(false);
    }
}

fn message(hwnd: HWND, text: &str) {
    unsafe {
        MessageBoxW(hwnd, wide(text).as_ptr(), wide("pc-rgb").as_ptr(), MB_OK | MB_ICONWARNING);
    }
}

fn pick_color(hwnd: HWND) {
    let Some(s) = with_app(|a| a.settings) else { return };
    let pastel = s.effect == Effect::Pastel;
    let cur = if pastel { s.pastel_color } else { s.color };
    let picked = CUSTOM_COLORS.with(|cc| unsafe {
        let mut custom = *cc.borrow();
        let mut ch: CHOOSECOLORW = std::mem::zeroed();
        ch.lStructSize = std::mem::size_of::<CHOOSECOLORW>() as u32;
        ch.hwndOwner = hwnd;
        ch.rgbResult = cur[0] as u32 | (cur[1] as u32) << 8 | (cur[2] as u32) << 16;
        ch.lpCustColors = custom.as_mut_ptr();
        ch.Flags = CC_FULLOPEN | CC_RGBINIT | CC_ANYCOLOR;
        SetForegroundWindow(hwnd);
        let ok = ChooseColorW(&mut ch) != 0;
        *cc.borrow_mut() = custom;
        ok.then(|| {
            let c = ch.rgbResult;
            [c as u8, (c >> 8) as u8, (c >> 16) as u8]
        })
    });
    if let Some(c) = picked {
        with_app(|a| {
            a.apply(|s| {
                if pastel {
                    s.pastel_color = c;
                } else {
                    s.color = c;
                    if !matches!(s.effect, Effect::Solid | Effect::Breathing) {
                        s.effect = Effect::Solid;
                    }
                }
            })
        });
    }
}

/// Watch the config directory (kernel change notification, zero CPU while idle) and
/// apply edits to config.ini live.
fn watch_config(hwnd: HWND) {
    use windows_sys::Win32::Storage::FileSystem::{
        FindFirstChangeNotificationW, FindNextChangeNotification, FILE_NOTIFY_CHANGE_FILE_NAME,
        FILE_NOTIFY_CHANGE_LAST_WRITE,
    };
    use windows_sys::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    let hwnd = hwnd as usize;
    std::thread::spawn(move || unsafe {
        let dir = wide(&config::dir().to_string_lossy());
        let h = FindFirstChangeNotificationW(dir.as_ptr(), 0, FILE_NOTIFY_CHANGE_LAST_WRITE | FILE_NOTIFY_CHANGE_FILE_NAME);
        if h == INVALID_HANDLE_VALUE {
            return;
        }
        while WaitForSingleObject(h, INFINITE) == WAIT_OBJECT_0 {
            PostMessageW(hwnd as HWND, WM_CONFIG_CHANGED, 0, 0);
            if FindNextChangeNotification(h) == 0 {
                break;
            }
        }
    });
}

fn config_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(config::path()).and_then(|m| m.modified()).ok()
}

/// Reload config.ini if it changed on disk (or always, with `force`). Does not re-save.
fn reload_config(force: bool) {
    thread_local!(static SEEN: std::cell::Cell<Option<std::time::SystemTime>> = const { std::cell::Cell::new(None) });
    let m = config_mtime();
    if !force && SEEN.with(|s| s.replace(m)) == m {
        return;
    }
    let s = Settings::load();
    with_app(|a| {
        if a.settings != s {
            log("config reloaded");
            a.settings = s;
            if let Some(e) = &a.engine {
                e.set_settings(s);
            }
            a.refresh_icon(NIM_MODIFY);
        }
    });
}
