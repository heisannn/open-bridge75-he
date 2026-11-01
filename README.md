# open-bridge75-he

An unofficial configurator for the Bridge75 HE magnetic keyboard (FW internal name 75ALHE). It provides a command-line tool now; a desktop app (GPUI Kit) is planned.

The CLI is tested on the real keyboard (FW V1.13.10, Windows 11). The desktop app is still a placeholder.

## Crates

| Crate | Contents |
|---|---|
| `bridge75-core` | Common types and the HID protocol: packets, key matrix, settings, TOML profiles. No I/O |
| `bridge75-device` | Raw HID communication through hidapi |
| `bridge75-cli` | The `bridge75` command |
| `bridge75-app` | Desktop app (GPUI Kit). Not built by default |

## Build

Requires Rust 1.85 or later.

```sh
cargo build --release          # target/release/bridge75
cargo run -p bridge75-app      # desktop app
```

## Usage

```sh
bridge75 info                                      # FW version, calibration status, ...
bridge75 trigger show                              # trigger settings of all keys
bridge75 trigger set all --mode rapid -a 1.6 --rt 0.8
bridge75 monitor w a s d                           # live key travel
bridge75 profile dump -o backup.toml               # back up the readable settings
bridge75 profile apply my.toml                     # write a profile
```

Run `bridge75 --help` for all commands, `bridge75 keys` for key names and `bridge75 actions` for the key assignment syntax.

The keyboard cannot report its keymap, macros or advanced keys, so keep them in a TOML profile and write them with `profile apply`. Sections left out of a profile are not written.

## Notes

- Updating the FW resets every setting on the keyboard. Back up with `bridge75 profile dump` first.
- Keep your hands off the keys during `calibrate` and `trigger reset`. Both count down 3 s before sending.
- The CLI never sends the dangerous commands (serial number write, factory reset, FW update).

## Documentation

[docs/](docs/README.md) describes the HID protocol, the firmware internals and measurements on the real keyboard.

## Disclaimer

This is an unofficial project, not affiliated with the manufacturer. Use it at your own risk. The repository does not include firmware or official app binaries.
