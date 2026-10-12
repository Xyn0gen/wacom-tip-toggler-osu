# osu-tip-toggle

Turns your pen's **Pressure & Buttons** off while you're playing a map in osu!lazer on Linux, and
back on everywhere else. For Wacom tablets running
[shavit's 1000 Hz custom firmware](https://files.shav.it/osu/tablet/).

A Linux take on [lukecupr/osu-wacom-pen-toggle](https://github.com/lukecupr/osu-wacom-pen-toggle)
(Windows).

## Why

On shavit's firmware, reading pen pressure makes reports arrive late or smoothed. Turning
"Pressure & Buttons" off in the firmware gives clean 1000 Hz tracking, but then the pen can't
click, which makes menus a pain. This tool switches it for you:

| osu! state | Pressure & buttons |
| --- | --- |
| Playing a map | off |
| Menus, song select, results | on |
| Pause screen, fail screen, "click to resume" | on |
| osu! closed, or the tool stopped | on |

## How it works

- **Game state:** follows osu!lazer's runtime log, which osu! writes every 50 ms. It watches for
  the player screen opening and closing, and for the gameplay clock stopping (pause, fail) and
  starting (play, resume). No memory reading.
- **Tablet:** reads and writes the firmware's settings report (HID feature report 0x60 on PTK-x70
  and CTL-4100/6100, 0x24 on CTL-4xx/6xx/490) through `/dev/hidraw*`. This is the same setting
  shavit's website changes. Only the tablet's RAM is touched, so replugging restores the default.
- Changes are spaced at least 1 s apart (faster toggling can desync the firmware), and pressure
  is switched back on when the tool exits.
- Runs alongside osu!'s built-in tablet support. No root and no OpenTabletDriver daemon needed.

## Requirements

- Linux and osu!lazer (not osu!stable)
- A Wacom tablet with shavit's firmware. Tested on a PTK-470. The CTL-4xx/6xx/490 and
  CTL-4100/6100 report layouts come from osu-wacom-pen-toggle and are untested here.
- Read/write access to the tablet's hidraw node (OpenTabletDriver's udev rules grant it)
- Rust, to build

## Install

```sh
cargo build --release
install -Dm755 target/release/osu-tip-toggle ~/.local/bin/osu-tip-toggle
install -Dm644 osu-tip-toggle.service ~/.config/systemd/user/osu-tip-toggle.service
systemctl --user enable --now osu-tip-toggle
```

Watch it switch while you play:

```sh
journalctl --user -u osu-tip-toggle -f
```

## Usage

| Command | What it does |
| --- | --- |
| `osu-tip-toggle` | Run it (the service does this) |
| `osu-tip-toggle --status` | Show the tablet's current firmware settings |
| `osu-tip-toggle --set on\|off` | Switch pressure & buttons by hand |
| `osu-tip-toggle --dry-run` | Follow osu! and print decisions without touching the tablet |

osu!'s data folder is found through `~/.local/share/osu/storage.ini`, so a relocated data folder
works.

## Notes

- The pen tip and buttons are off from the moment a map starts, so skip intros with the keyboard.
- Flashing custom firmware can brick your tablet. Follow shavit's instructions carefully.

## Credits

- [shavit](https://github.com/shavitush) for the custom firmware
- [lukecupr/osu-wacom-pen-toggle](https://github.com/lukecupr/osu-wacom-pen-toggle) for the
  firmware's report format

## License

GPL-3.0-or-later, like the project it builds on. See [LICENSE](LICENSE).
