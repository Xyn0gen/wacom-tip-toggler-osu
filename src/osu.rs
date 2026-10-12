//! Follows osu!lazer's runtime log to know when gameplay is running.
//!
//! osu! writes its log every 50 ms. Screen changes ("OsuScreenStack#N(depth:D) entered
//! SoloPlayer#M") mark entering and leaving gameplay, and the gameplay clock lines mark pauses,
//! fail screens and resumes: the clock stops before the pause or fail overlay shows, and starts
//! again only after the resume click.

use std::fs::{self, File};
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

#[derive(Default, Clone, Copy, PartialEq)]
pub struct Game {
    in_player: bool,
    clock_running: bool,
}

impl Game {
    pub fn playing(&self) -> bool {
        self.in_player && self.clock_running
    }

    pub fn feed(&mut self, line: &str) {
        if let Some((_, rest)) = line.split_once("OsuScreenStack#") {
            // "568(depth:6) entered SoloPlayer#567"
            let Some((_, action)) = rest.split_once(") ") else { return };
            for (verb, active) in [("entered ", true), ("resume to ", true), ("suspended ", false), ("exit from ", false)] {
                if let Some(screen) = action.strip_prefix(verb) {
                    // Solo/Replay/Multiplayer/Spectator/EditorPlayer; not PlayerLoader
                    if screen.split('#').next().unwrap_or_default().ends_with("Player") {
                        self.in_player = active;
                        self.clock_running = false;
                    }
                    return;
                }
            }
        } else if line.contains("GameplayClockContainer started via call to") {
            self.clock_running = true;
        } else if line.contains("GameplayClockContainer stopped via call to") {
            self.clock_running = false;
        } else if line.contains("Host execution state changed to Stopp") {
            *self = Game::default();
        }
    }
}

/// osu!'s logs folder: `<data>/logs`, where `<data>` may be relocated by storage.ini.
pub fn logs_dir() -> PathBuf {
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_default();
    let base = data_home.join("osu");
    let custom = fs::read_to_string(base.join("storage.ini")).ok().and_then(|ini| {
        ini.lines().find_map(|l| {
            let (key, value) = l.split_once('=')?;
            (key.trim() == "FullPath").then(|| PathBuf::from(value.trim()))
        })
    });
    custom.unwrap_or(base).join("logs")
}

pub fn osu_running() -> bool {
    let Ok(procs) = fs::read_dir("/proc") else { return false };
    procs.flatten().any(|p| {
        p.file_name().to_str().is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
            && fs::read_to_string(p.path().join("comm")).is_ok_and(|c| c.trim_end() == "osu!")
    })
}

pub enum Event {
    Session(PathBuf),
    Line(String),
}

/// Reads the newest `<unix time>.runtime.log`, switching when osu! starts a new session.
pub struct LogFollower {
    dir: PathBuf,
    file: Option<(PathBuf, File, u64)>,
    partial: Vec<u8>,
}

impl LogFollower {
    pub fn new(dir: PathBuf) -> Self {
        LogFollower { dir, file: None, partial: Vec::new() }
    }

    fn newest(&self) -> Option<PathBuf> {
        fs::read_dir(&self.dir)
            .ok()?
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stamp: u64 = name.strip_suffix(".runtime.log")?.parse().ok()?;
                Some((stamp, e.path()))
            })
            .max_by_key(|(stamp, _)| *stamp)
            .map(|(_, path)| path)
    }

    /// Switches to a newer session log (read from its start) if one appeared.
    pub fn check_session(&mut self, events: &mut Vec<Event>) {
        let Some(newest) = self.newest() else { return };
        if self.file.as_ref().is_some_and(|(path, ..)| *path == newest) {
            return;
        }
        if let Ok(file) = File::open(&newest) {
            self.file = Some((newest.clone(), file, 0));
            self.partial.clear();
            events.push(Event::Session(newest));
        }
    }

    /// Complete lines appended since the last call.
    pub fn read_lines(&mut self, events: &mut Vec<Event>) {
        let Some((_, file, offset)) = self.file.as_mut() else { return };
        let mut buf = Vec::new();
        if file.seek(SeekFrom::Start(*offset)).is_err() || file.read_to_end(&mut buf).is_err() {
            return;
        }
        *offset += buf.len() as u64;
        self.partial.extend_from_slice(&buf);
        while let Some(pos) = self.partial.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=pos).collect();
            events.push(Event::Line(String::from_utf8_lossy(&line).trim_end().to_owned()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Game;

    fn run(lines: &[&str]) -> Game {
        let mut g = Game::default();
        for l in lines {
            g.feed(l);
        }
        g
    }

    const ENTER: &str = "2026-10-12 05:59:01 [verbose]: \u{1f4fa} OsuScreenStack#568(depth:6) entered SoloPlayer#567";
    const START: &str = "2026-10-12 05:59:01 [verbose]: GameplayClockContainer started via call to StartGameplayClock";
    const STOP: &str = "2026-10-12 05:59:30 [verbose]: GameplayClockContainer stopped via call to StopGameplayClock";

    #[test]
    fn gameplay_pause_resume_results() {
        let loader = "2026-10-12 05:59:01 [verbose]: \u{1f4fa} OsuScreenStack#568(depth:5) suspended SoloSongSelect+PlayerLoader#284 (waiting on SoloPlayer#567)";
        assert!(!run(&[loader]).playing());
        assert!(!run(&[loader, ENTER]).playing());
        assert!(run(&[loader, ENTER, START]).playing());
        assert!(!run(&[ENTER, START, STOP]).playing());
        assert!(run(&[ENTER, START, STOP, START]).playing());
        let results = "2026-10-12 06:03:25 [verbose]: \u{1f4fa} OsuScreenStack#568(depth:6) suspended SoloPlayer#567 (waiting on SoloResultsScreen#561)";
        assert!(!run(&[ENTER, START, results]).playing());
        let resume = "2026-10-12 06:03:25 [verbose]: \u{1f4fa} OsuScreenStack#568(depth:6) resume to SoloPlayer#567";
        assert!(!run(&[ENTER, START, results, resume]).playing());
    }

    #[test]
    fn quit_and_shutdown() {
        let exit = "2026-10-12 06:03:25 [verbose]: \u{1f4fa} OsuScreenStack#568(depth:5) exit from SoloPlayer#567";
        assert!(!run(&[ENTER, START, exit]).playing());
        let stopped = "2026-10-12 06:10:00 [verbose]: Host execution state changed to Stopping";
        assert!(!run(&[ENTER, START, stopped]).playing());
    }

    #[test]
    fn background_stack_is_ignored() {
        let bg = "2026-10-12 06:03:25 [verbose]: \u{1f4fa} BackgroundScreenStack#480(depth:2) entered BackgroundScreenBeatmap#180";
        assert!(run(&[ENTER, START, bg]).playing());
    }
}
