# Research notes

To build our own configurator for the Bridge75 HE (FW internal name 75ALHE), we analyzed the official configurator, the firmware, and the upgrader, and verified part of the findings on the real keyboard. These notes collect the results.

## Documents

| File | Contents |
|---|---|
| [protocol.md](protocol.md) | HID protocol spec. The commands and packet formats our configurator sends and receives |
| [firmware.md](firmware.md) | Firmware internals. Hardware layout, sensor processing (filter, travel calculation, trigger decisions), storage area, bootloader, hidden features |
| [measurements.md](measurements.md) | Measurements on the real keyboard. Sensor values, calibration, filter delay |
| [official-app.md](official-app.md) | Structure of the official configurator, and the profiles it stores on the PC |

## What was examined

| Target | Version |
|---|---|
| Official configurator `Short Cut HE Config.exe` + `witmodSdk.dll` | V_2.03.04 (Qt5, 32bit) |
| Upgrader `Bridge75HE_Upgrader.exe` | MagneticKB Upgrade Tool V1.0.0.3 |
| Firmware (bundled with the upgrader) | `M484,01,KB,OW,75ALHE,V1.13.10` (built 2024-11-19 10:36:50) |
| The real keyboard | Shipped with V1_12_08 (2024-06-07). Updated to V1.13.10 with the official upgrader before verification |

The real keyboard has no lighting (LEDs). The FW and the official app contain lighting code shared with other models, but it has no effect on this keyboard.

## Evidence markers

Each document marks the evidence for its statements with these markers.

| Marker | Meaning |
|---|---|
| **[App]** | Code of the official configurator / SDK |
| **[FW]** | Firmware code |
| **[Up]** | Upgrader code |
| **[HW]** | Verified on the real keyboard |

Everything not marked "inferred" is verified in code or on the real keyboard. Addresses are offsets from the start of the firmware image (= flash addresses).

## Verified on the real keyboard (2026-11-02)

- Device detection (0416:7372, Interface 2, Usage Page 0xFF1B) and 64-byte send / receive
- 0x01 heartbeat, 0x0D device info, 0x07 lighting query, 0x14 sleep time query
- 0x21 (HE settings): sub4 max travel, sub5 key settings query, sub9 polling rate query, sub10 debounce query and set (kept after replugging USB), sub12 trigger settings, sub2/sub1 live travel reports
- 0x23 raw sensor value stream (1000 frames per second). It does not stop when heartbeats stop
- 0x24 (SOCD) query: b[2]=1 (pairs, 2 packets) and b[2]=2 (state)
- Writes and the resulting behavior ([measurements.md](measurements.md) section 9):
  - 0x18 keymap (base layer, Fn layer)
  - 0x19 macro (an empty slot with offset 8 = 0xFF outputs the fallback HID)
  - 0x22 advanced keys (DKS / MT / TGL)
  - 0x24 SOCD (pair write, global on/off, last input wins behavior)
  - 0x14 sleep time set
  - 0x21 sub0 / sub13 (trigger settings), sub6 (all keys to default), sub7 (single-key calibration), sub9 (polling rate set)
- 0x10 default key layout, 0x0E fixed-function key mask
- Updating the FW with the official upgrader resets all settings stored in the keyboard
- After calibrating in the official app, the calibrated flag in 0x01 becomes 1

## Notes

- This is unofficial research, not affiliated with the manufacturer. The purpose is interoperability (our own configurator).
- Some commands are dangerous to send. In particular **0x16 (serial number write)**, **0x14 with b[6]=1 (factory reset)**, **0x0B / 0x0C (FW update)**, **252 or more in sub10** ([protocol.md](protocol.md) section 13).
- Some commands make the FW write out of range in RAM without checking bounds: **0x22 slot 40 or higher**, **0x21 sub7 with an out-of-range row / col**.
- **Do not send 0x21 sub6 / 7 / 8 while a key is pressed**. The rest calibration value is learned at the pressed position, and the key may stop working ([protocol.md](protocol.md) section 6).
- This repository does not include firmware or official app binaries, or decompiled output.
