//! One worker thread per device. Workers sleep until the next frame (animated effects)
//! or until settings change (static colors), so a solid color costs ~0% CPU.

use crate::color::Rgb;
use crate::config::Settings;
use crate::devices::{self, Device};
use crate::effects::Frame;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// Resend everything this often even if nothing changed, in case the hardware got reset.
const REFRESH: Duration = Duration::from_secs(5);
const RETRY: Duration = Duration::from_secs(5);

#[derive(Clone)]
struct State {
    settings: Settings,
    gen: u64,
    /// Bumped to make workers re-initialize hardware (after resume).
    epoch: u64,
    suspended: bool,
    quit: bool,
    /// Turn LEDs off when quitting.
    blackout_on_quit: bool,
}

struct Shared {
    state: Mutex<State>,
    cv: Condvar,
    devices: Mutex<Vec<String>>,
}

pub struct Engine {
    shared: Arc<Shared>,
    threads: Arc<Mutex<Vec<JoinHandle<()>>>>,
}

impl Engine {
    pub fn start(settings: Settings) -> Engine {
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                settings,
                gen: 0,
                epoch: 0,
                suspended: false,
                quit: false,
                blackout_on_quit: true,
            }),
            cv: Condvar::new(),
            devices: Mutex::new(Vec::new()),
        });
        let threads = Arc::new(Mutex::new(Vec::new()));
        // Detect off the UI thread so the tray icon shows up instantly.
        let (sh, th) = (shared.clone(), threads.clone());
        let detector = std::thread::spawn(move || {
            let devs = devices::detect(&settings);
            let mut names = Vec::new();
            for d in devs {
                names.push(d.name().to_string());
                crate::log(&format!("device: {}", d.name()));
                let sh = sh.clone();
                th.lock().unwrap().push(std::thread::spawn(move || worker(d, sh)));
            }
            *sh.devices.lock().unwrap() = names;
        });
        threads.lock().unwrap().push(detector);
        Engine { shared, threads }
    }

    pub fn devices(&self) -> Vec<String> {
        self.shared.devices.lock().unwrap().clone()
    }

    fn modify(&self, f: impl FnOnce(&mut State)) {
        let mut st = self.shared.state.lock().unwrap();
        f(&mut st);
        drop(st);
        self.shared.cv.notify_all();
    }

    pub fn set_settings(&self, s: Settings) {
        self.modify(|st| {
            if st.settings != s {
                st.settings = s;
                st.gen += 1;
            }
        });
    }

    pub fn set_suspended(&self, on: bool) {
        self.modify(|st| {
            if st.suspended && !on {
                st.epoch += 1;
            }
            st.suspended = on;
        });
    }

    /// Stop all workers. With `blackout`, LEDs are turned off first (if the setting allows).
    pub fn stop(self, blackout: bool) {
        self.modify(|st| {
            st.quit = true;
            st.blackout_on_quit = blackout;
        });
        // The detector pushes worker handles, so keep draining until nothing is left.
        loop {
            let Some(h) = self.threads.lock().unwrap().pop() else { break };
            let _ = h.join();
        }
    }
}

fn worker(mut dev: Box<dyn Device>, sh: Arc<Shared>) {
    let t0 = Instant::now();
    let mut seen_gen = u64::MAX;
    let mut seen_epoch = u64::MAX;
    let mut ready = false;
    let mut dark = false;
    let mut force = true;
    let mut last_force = Instant::now();
    let mut pos: Vec<(f32, f32)> = Vec::new();
    let mut colors: Vec<Rgb> = Vec::new();
    let mut stats = Stats::new();

    // Wait on the condvar until the state moves past what we've seen, or timeout.
    let wait = |timeout: Option<Duration>, gen: u64, epoch: u64, suspended: bool| {
        let guard = sh.state.lock().unwrap();
        let unchanged =
            |st: &mut State| !st.quit && st.gen == gen && st.epoch == epoch && st.suspended == suspended;
        let _ = match timeout {
            Some(t) => drop(sh.cv.wait_timeout_while(guard, t, unchanged).unwrap()),
            None => drop(sh.cv.wait_while(guard, unchanged).unwrap()),
        };
    };

    loop {
        let st = sh.state.lock().unwrap().clone();
        let s = st.settings;

        if st.quit || st.suspended {
            if ready && !dark && (st.suspended || (st.blackout_on_quit && s.off_on_exit)) {
                colors.clear();
                colors.resize(pos.len(), [0, 0, 0]);
                let _ = dev.write(&colors, true);
                dark = true;
            }
            if st.quit {
                return;
            }
            wait(None, st.gen, st.epoch, true);
            continue;
        }

        if !ready || st.epoch != seen_epoch {
            if let Err(e) = dev.init() {
                crate::log(&format!("{}: init failed: {e}", dev.name()));
                ready = false;
                wait(Some(RETRY), st.gen, st.epoch, false);
                continue;
            }
            ready = true;
            seen_epoch = st.epoch;
            force = true;
        }
        if st.gen != seen_gen {
            pos = dev.layout(&s);
            seen_gen = st.gen;
            force = true;
        }

        let frame_start = Instant::now();
        let frame = Frame::new(&s, (frame_start - t0).as_secs_f64());
        colors.clear();
        colors.extend(pos.iter().map(|&(x, y)| frame.color(x, y)));

        let refresh = force || dark || last_force.elapsed() >= REFRESH;
        if let Err(e) = dev.write(&colors, refresh) {
            crate::log(&format!("{}: write failed: {e}", dev.name()));
            ready = false;
            wait(Some(RETRY), st.gen, st.epoch, false);
            continue;
        }
        dark = false;
        if refresh {
            force = false;
            last_force = Instant::now();
        }
        stats.frame(frame_start.elapsed(), dev.name());

        if s.effect.animated() {
            // thread::sleep uses a high-resolution timer; changes are picked up next frame.
            let interval = Duration::from_secs_f64(1.0 / s.fps.max(1) as f64);
            if let Some(rest) = interval.checked_sub(frame_start.elapsed()) {
                std::thread::sleep(rest);
            }
        } else {
            wait(Some(REFRESH), st.gen, st.epoch, false);
        }
    }
}

/// Periodic per-device cost report in the log: frames, avg write time, thread CPU.
struct Stats {
    since: Instant,
    frames: u32,
    busy: Duration,
    cpu_start: u64,
}

const STATS_EVERY: Duration = Duration::from_secs(30);

fn thread_cpu_100ns() -> u64 {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, GetThreadTimes};
    let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
    let (mut c, mut e, mut k, mut u) = (z, z, z, z);
    unsafe { GetThreadTimes(GetCurrentThread(), &mut c, &mut e, &mut k, &mut u) };
    let ft = |f: FILETIME| (f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64;
    ft(k) + ft(u)
}

impl Stats {
    fn new() -> Self {
        Stats { since: Instant::now(), frames: 0, busy: Duration::ZERO, cpu_start: thread_cpu_100ns() }
    }

    fn frame(&mut self, took: Duration, name: &str) {
        self.frames += 1;
        self.busy += took;
        let wall = self.since.elapsed();
        if wall >= STATS_EVERY {
            let cpu = (thread_cpu_100ns() - self.cpu_start) as f64 / 1e7;
            crate::log(&format!(
                "{name}: {:.1} fps, {:.2} ms/frame, cpu {:.2}% of a core",
                self.frames as f64 / wall.as_secs_f64(),
                self.busy.as_secs_f64() * 1000.0 / self.frames.max(1) as f64,
                cpu / wall.as_secs_f64() * 100.0
            ));
            *self = Stats::new();
        }
    }
}
