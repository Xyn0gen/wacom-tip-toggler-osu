//! Pressure & buttons switch of Wacom tablets running shavit's 1000 Hz firmware, over hidraw.
//!
//! The firmware keeps its settings in a HID feature report: ID 0x60 (64 bytes) on PTK-x70 and
//! CTL-4100/6100, ID 0x24 (32 bytes) on CTL-4xx/6xx/490. Reading it returns `T V 1`, two bytes
//! that are handed back unchanged, the pressure & buttons flag (byte 6), capability flags
//! (byte 7, 0x04 = motion sync) and the motion sync value (byte 8). Writing sends `T V 1`, the
//! two bytes, the new flag and the motion sync value, starting at a model-specific offset. Only
//! the tablet's RAM changes, so unplugging it restores the defaults.
//! Protocol as used by github.com/lukecupr/osu-wacom-pen-toggle (GPL-3.0).

use std::fs::{self, File, OpenOptions};
use std::io;
use std::os::fd::AsRawFd;
use std::path::PathBuf;

const VID_WACOM: u32 = 0x056a;
const SIGNATURE: [u8; 3] = *b"TV\x01";
/// (report id, report length including the id byte)
const REPORTS: [(u8, usize); 2] = [(0x60, 64), (0x24, 32)];

pub struct Tablet {
    pub name: String,
    pub node: PathBuf,
    pub report_id: u8,
    sysfs: PathBuf,
    /// Resolved HID device path at detection; it changes when the tablet is replugged.
    hid_dev: PathBuf,
    len: usize,
    write_offset: usize,
}

/// `_IOC(_IOC_READ | _IOC_WRITE, 'H', nr, len)` from linux/hidraw.h.
fn hid_ioc(nr: u64, len: usize) -> u64 {
    (3 << 30) | ((len as u64) << 16) | ((b'H' as u64) << 8) | nr
}

fn get_feature(file: &File, report_id: u8, len: usize) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; len];
    buf[0] = report_id;
    // HIDIOCGFEATURE
    let r = unsafe { libc::ioctl(file.as_raw_fd(), hid_ioc(0x07, len) as _, buf.as_mut_ptr()) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(buf)
}

fn set_feature(file: &File, buf: &[u8]) -> io::Result<()> {
    // HIDIOCSFEATURE
    let r = unsafe { libc::ioctl(file.as_raw_fd(), hid_ioc(0x06, buf.len()) as _, buf.as_ptr()) };
    if r < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn write_offset(pid: u32, report_id: u8) -> usize {
    match pid {
        // CTL-4100 / CTL-6100 (and WL)
        0x0374..=0x0377 | 0x03c5 => 3,
        // CTL/CTH-480/680, CTL-472/672, CTL/CTH-490/690, PTK-470/670/870
        0x0302 | 0x0303 | 0x030e | 0x0323 | 0x037a | 0x037b | 0x033b..=0x033e | 0x03f5 | 0x03f7
        | 0x03f9 => 1,
        _ if report_id == 0x60 => 3,
        _ => 1,
    }
}

fn open(node: &PathBuf) -> io::Result<File> {
    OpenOptions::new().read(true).write(true).open(node)
}

impl Tablet {
    /// Finds the first Wacom HID interface that answers with the firmware's signature.
    pub fn find() -> Option<Tablet> {
        let mut entries: Vec<_> = fs::read_dir("/sys/class/hidraw").ok()?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let sysfs = entry.path();
            let Ok(uevent) = fs::read_to_string(sysfs.join("device/uevent")) else {
                continue;
            };
            let field = |key: &str| uevent.lines().find_map(|l| l.strip_prefix(key)).map(str::to_owned);
            // HID_ID=0003:0000056A:000003F5
            let Some(hid_id) = field("HID_ID=") else { continue };
            let mut ids = hid_id.split(':').skip(1).map(|s| u32::from_str_radix(s, 16));
            let (Some(Ok(vid)), Some(Ok(pid))) = (ids.next(), ids.next()) else {
                continue;
            };
            if vid != VID_WACOM {
                continue;
            }
            let node = PathBuf::from("/dev").join(entry.file_name());
            let Ok(file) = open(&node) else { continue };
            for (report_id, len) in REPORTS {
                let Ok(buf) = get_feature(&file, report_id, len) else { continue };
                if buf[1..4] != SIGNATURE {
                    continue;
                }
                let Ok(hid_dev) = fs::canonicalize(sysfs.join("device")) else { continue };
                return Some(Tablet {
                    name: field("HID_NAME=").unwrap_or_else(|| format!("Wacom {pid:04x}")),
                    node,
                    report_id,
                    sysfs,
                    hid_dev,
                    len,
                    write_offset: write_offset(pid, report_id),
                });
            }
        }
        None
    }

    /// Whether this is still the same plugged-in device (sysfs only, no USB traffic).
    pub fn still_present(&self) -> bool {
        fs::canonicalize(self.sysfs.join("device")).ok().as_ref() == Some(&self.hid_dev)
    }

    fn read(&self, file: &File) -> io::Result<Vec<u8>> {
        let buf = get_feature(file, self.report_id, self.len)?;
        if buf[1..4] != SIGNATURE {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "firmware signature missing"));
        }
        Ok(buf)
    }

    /// Raw settings report, for `--status`.
    pub fn settings(&self) -> io::Result<Vec<u8>> {
        self.read(&open(&self.node)?)
    }

    /// Switches pressure & buttons. Returns false if it was already in that state.
    pub fn set_pressure(&self, on: bool) -> io::Result<bool> {
        let file = open(&self.node)?;
        let cur = self.read(&file)?;
        if (cur[6] != 0) == on {
            return Ok(false);
        }
        let mut out = vec![0u8; self.len];
        out[0] = self.report_id;
        let o = self.write_offset;
        out[o..o + 3].copy_from_slice(&SIGNATURE);
        out[o + 3] = cur[4];
        out[o + 4] = cur[5];
        out[o + 5] = on as u8;
        out[o + 6] = if cur[7] & 0x04 != 0 { cur[8] } else { 0 };
        set_feature(&file, &out)?;
        if (self.read(&file)?[6] != 0) != on {
            return Err(io::Error::other("tablet ignored the setting"));
        }
        Ok(true)
    }
}
