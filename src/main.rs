//! Turns pen pressure & buttons off while osu!lazer gameplay runs (not paused or failed) on Wacom
//! tablets with shavit's 1000 Hz firmware, and back on everywhere else. Reading pressure makes that
//! firmware delay or smooth reports; without it the pen tracks at a steady 1000 Hz.
//!
//! usage: osu-tip-toggle [--dry-run | --status | --set on|off]

mod osu;
mod tablet;

use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::sleep;
use std::time::{Duration, Instant};

use tablet::Tablet;

/// Flipping the setting faster than this can desync the tablet's MCU and digitizer at 1000 Hz
/// (per osu-wacom-pen-toggle), so changes are spaced at least this far apart.
const MIN_TOGGLE_INTERVAL: Duration = Duration::from_millis(1000);
const TICK: Duration = Duration::from_millis(20);
const FIND_RETRY: Duration = Duration::from_secs(10);

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::Relaxed);
}

fn on_off(on: bool) -> &'static str {
    if on {
        "ON"
    } else {
        "OFF"
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.iter().map(String::as_str).collect::<Vec<_>>().as_slice() {
        [] => run(false),
        ["--dry-run"] => run(true),
        ["--status"] => status(),
        ["--set", value @ ("on" | "off")] => set(*value == "on"),
        _ => {
            eprintln!("usage: osu-tip-toggle [--dry-run | --status | --set on|off]");
            std::process::exit(2);
        }
    }
}

fn status() {
    let Some(t) = Tablet::find() else {
        eprintln!("no tablet with shavit's firmware found");
        std::process::exit(1);
    };
    match t.settings() {
        Ok(s) => println!(
            "{} on {} (report 0x{:02X}): pressure & buttons {}, report rate {} Hz, motion sync {}",
            t.name,
            t.node.display(),
            t.report_id,
            on_off(s[6] != 0),
            u16::from_le_bytes([s[4], s[5]]),
            if s[7] & 0x04 != 0 { on_off(s[8] != 0) } else { "unsupported" },
        ),
        Err(e) => {
            eprintln!("{}: {e}", t.node.display());
            std::process::exit(1);
        }
    }
}

fn set(on: bool) {
    let Some(t) = Tablet::find() else {
        eprintln!("no tablet with shavit's firmware found");
        std::process::exit(1);
    };
    match t.set_pressure(on) {
        Ok(changed) => println!("pressure & buttons {}{}", on_off(on), if changed { "" } else { " (already)" }),
        Err(e) => {
            eprintln!("{}: {e}", t.node.display());
            std::process::exit(1);
        }
    }
}

fn run(dry_run: bool) {
    for sig in [libc::SIGTERM, libc::SIGINT, libc::SIGHUP] {
        let handler = on_signal as extern "C" fn(libc::c_int);
        unsafe { libc::signal(sig, handler as libc::sighandler_t) };
    }
    let dir = osu::logs_dir();
    eprintln!("osu! logs: {}{}", dir.display(), if dry_run { " (dry run, tablet untouched)" } else { "" });
    let mut log = osu::LogFollower::new(dir);
    let mut game = osu::Game::default();
    let mut events = Vec::new();
    let mut tablet: Option<Tablet> = None;
    let mut next_find = Instant::now();
    // Pressure state last written to (or found on) the tablet.
    let mut applied: Option<bool> = None;
    let mut last_write: Option<Instant> = None;
    let mut osu_running = false;
    let mut wanted: Option<bool> = None;
    let mut next_slow = Instant::now();

    while !STOP.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now >= next_slow {
            next_slow = now + Duration::from_secs(1);
            log.check_session(&mut events);
            osu_running = osu::osu_running();
            if tablet.as_ref().is_some_and(|t| !t.still_present()) {
                eprintln!("tablet disconnected");
                tablet = None;
            }
            if tablet.is_none() && now >= next_find {
                next_find = now + FIND_RETRY;
                tablet = Tablet::find();
                applied = None;
                if let Some(t) = &tablet {
                    eprintln!("tablet: {} on {} (report 0x{:02X})", t.name, t.node.display(), t.report_id);
                }
            }
        }

        log.read_lines(&mut events);
        for event in events.drain(..) {
            match event {
                osu::Event::Session(path) => {
                    game = osu::Game::default();
                    eprintln!("following {}", path.display());
                }
                osu::Event::Line(line) => game.feed(&line),
            }
        }

        let want = !(osu_running && game.playing());
        if wanted != Some(want) {
            wanted = Some(want);
            eprintln!("{} -> pressure & buttons {}", if want { "menus / paused" } else { "gameplay" }, on_off(want));
        }
        if let Some(t) = &tablet {
            let spaced = last_write.is_none_or(|w| w.elapsed() >= MIN_TOGGLE_INTERVAL);
            if !dry_run && applied != Some(want) && spaced {
                match t.set_pressure(want) {
                    Ok(changed) => {
                        applied = Some(want);
                        if changed {
                            last_write = Some(Instant::now());
                        }
                    }
                    Err(e) => {
                        eprintln!("tablet write failed: {e}");
                        tablet = None;
                        next_find = Instant::now() + Duration::from_secs(1);
                    }
                }
            }
        }
        sleep(TICK);
    }

    // Leave the pen usable after we exit.
    if dry_run {
        return;
    }
    if let Some(t) = tablet.or_else(Tablet::find) {
        if let Some(rest) = last_write.and_then(|w| MIN_TOGGLE_INTERVAL.checked_sub(w.elapsed())) {
            sleep(rest);
        }
        match t.set_pressure(true) {
            Ok(_) => eprintln!("exit -> pressure & buttons ON"),
            Err(e) => eprintln!("could not turn pressure back on: {e}"),
        }
    }
}
