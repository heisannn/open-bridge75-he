# Measurements on the real keyboard

Measured on 2026-11-02. Windows 11, hidapi (Python), keyboard plugged directly into a USB port on the PC itself.

| Item | Details |
|---|---|
| FW | Section 1 uses the shipped V1_12_08. Section 2 onward uses V1.13.10, updated with the official upgrader |
| Debounce | 0 (official default is 3) |
| Trigger settings | 77 keys use Rapid Trigger (actuation point 1.6mm, RT sensitivity 0.8mm). 4 keys (A, X, '", .>) use 1.2mm / 0.4mm |
| Value units | "raw value" is the 12bit ADC value delivered by 0x23. "÷8" is the ADC>>3 value the FW uses in its calculations |

---

## 1. Communication check

- Interface 2 of 0416:7372 appears on Windows as Usage Page 0xFF1B / Usage 0x91.
- 64-byte (including Report ID) writes and reads work.
- Commands checked: 0x01, 0x0D, 0x07 query, 0x14 sleep query, 0x21 sub2 / sub3 / sub4 / sub5 / sub9 / sub10 / sub12, sub1 reports, 0x23.
- The sub10 (debounce) setting was kept after replugging USB.
- The second token of 0x0D was `22`, as the FW analysis describes. The old FW writes its version as `V1_12_08` (underscore separated).

## 2. Scan rate

Counted frames in the 0x23 stream.

| Recording | Frames | Per second | Missing |
|---|---|---|---|
| 3 s | 2999 | about 1000 | — |
| 5 s | 5000 | 1000.1 | 1 incomplete (at the end) |
| 120 s | 119,995 | 999.97 | 1 incomplete |

So we verified on the real keyboard that **each key is read at 1kHz** (matches the FW analysis: 8kHz ÷ 8 groups).

## 3. Sensor values at rest (3 s, 81 keys)

- Raw value: 1891 to 2032 (236 to 254 after ÷8). The differences between keys are probably due to mounting position and magnet variation.
- Noise: the max-min difference over 3000 frames is 11 to 15 in raw value (average 11.3). That is 1 to 2 steps after ÷8.

## 4. Calibration

After calibrating all keys in the official app, we pressed every key to the bottom one at a time and measured the deepest value. The FW has no way to read calibration values back, so this deepest value is used as an approximation of the calibration value (the FW also records a stable value at bottom-out).

### Bottom-out values (÷8)
- 146 to 170 across 81 keys, average 154. The FW default is 150.
- The span from rest to bottom-out is about 90 steps (720 to 785 in raw value). That means 3.6mm is measured in about 90 steps, or about 0.04mm per step on average.

| Difference from default 150 | Keys |
|---|---|
| +20 | Del |
| +10 to +12 | E, 2@, Up |
| +7 to +9 | J, M, Y, F4, F11, [{, L, R_Shift, 6^, ]}, A, H, L_Shift, R_Alt |
| within ±6 | most of the rest |
| −1 to −4 | Down, Right, F2, F7, PgUp, L_Alt |

### Estimate for the uncalibrated case (bottom-out 150)
We plugged the measured values and the default into the FW's travel formula and compared the results.
- Some keys did not reach 3.6mm even when pressed to the bottom. Readings: 3.2mm for Del, 3.3mm for E, 3.4mm for 2, Up, J, M, Y.
- An actuation point set to 1.6mm corresponded to an actual press depth that varied by key from 1.5 to 1.9mm (Del 1.9mm, about 30 keys 1.7mm, Down, Right, F7, PgUp 1.5mm).
- After calibration, every key reads 3.6mm at bottom-out, and actuation points line up.

## 5. Reproducing the FW processing (simulator)

We reproduced the filter and travel calculation from [firmware.md](firmware.md) section 3 on the PC and applied them to the 120 s recording (raw values from 0x23). We compared the result with the sub1 travel reports the FW itself sent (4 keys W, A, S, D, 6923 reports total).

- The rest and bottom-out calibration values (T, B) cannot be read, so we searched for the values that best match the reports. The results are almost the same as the measurements in section 4 (e.g. D: T=250, B=153).
- Metrics
  - **Recall**: the share of FW reports for which the simulation has the same value change within ± a few frames
  - **Precision**: the share of simulated changes for which the FW sent a report

| Key | Debounce 0 (actual setting) | Debounce 1 | Debounce 3 | No filter |
|---|---|---|---|---|
| W | 100.0% / 79.7% | 96.9% / 78.9% | 91.4% / 78.7% | 100.0% / 78.0% |
| A | 100.0% / 93.1% | 93.0% / 91.2% | 79.9% / 87.8% | 99.6% / 80.1% |
| S | 99.8% / 84.1% | 96.1% / 82.7% | 88.7% / 81.1% | 99.8% / 77.9% |
| D | 100.0% / 99.9% | 95.1% / 98.6% | 86.9% / 94.9% | 99.6% / 90.5% |

(Each cell is recall / precision)

- The actual setting (debounce 0) gives the highest recall and precision for every key. We concluded that the simulator reproduces the FW travel calculation.
- Precision is lower for W and S because there is only one report slot, and the FW did not send reports for monitored keys that moved at the same time ([protocol.md](protocol.md) section 6).
- The press / release decision ([firmware.md](firmware.md) section 3.3) was written from the code and has not yet been checked against actual key input.

## 6. Decision delay caused by the filter

Using the same 120 s recording, we compared trigger decisions with the FW filter against decisions with no filter, where each frame's value is taken immediately. We used each key's actual trigger settings and covered the 14 keys that were used, with 833 presses. 1 frame = 1ms.

| Debounce | Press delay (average / median / 90th percentile) | Release delay (average) |
|---|---|---|
| **0 (actual setting)** | **0.50ms / 0 / 1ms** | 0.01ms |
| 3 (default) | 2.80ms / 3ms / 4ms | 0.56ms |

- Changing debounce from 3 to 0 makes press decisions 2.3ms faster on average.
- The filter's delay of "waiting a few frames at the start of movement and at reversals" is mostly hidden by the time the finger takes to travel the actuation point or RT sensitivity distance.
- With no filter, D produced 6 ON/OFF events that the FW did not. Removing the filter completely is not safe.

## 7. Expected gains from rewriting the FW

| Item | Average reduction | Evidence |
|---|---|---|
| Improve the filter (e.g. decide immediately in the dead zone) | about 0.5ms | Measured in section 6 (difference between debounce 0 and no filter) |
| Process each group right away | about 0.4ms | Calculation (values up to 0.9ms old are used now) |
| Scan at 2kHz | about 0.25ms | Calculation. Depends on sensor settling time |
| **Total** | **about 1ms** | |

USB already sends every 125µs at High-Speed, so there is no room for improvement there.

## 8. Reset by FW update

- Updating V1_12_08 → V1.13.10 with the official upgrader returned all settings in the keyboard to defaults (trigger settings mode 0 / 1.8mm on all keys, debounce 3, calibrated flag in 0x01 = 0).
- Before the update we read 0x21 sub5 (all keys), debounce, and so on. After the update we wrote them back and verified that all 96 cells matched the originals.
- The official app's profiles on the PC did not contain trigger settings ([official-app.md](official-app.md)).

## 9. Writing settings and resulting behavior

We wrote settings with our own CLI (`crates/bridge75-cli`) and checked them by reading back and by actual input.

- Before testing, we backed up the readable settings.
- Afterwards, we reset the keymap, macros, advanced keys, and SOCD to defaults (empty). Everything else was written back from the backup.
- After writing back, we read the settings and verified they matched the backup.

| Target | What was written | Result |
|---|---|---|
| 0x21 sub10 / sub9 | Debounce 0 → 1 → 0, polling rate 8 → 4 → 8 | Matched on read back |
| 0x21 sub0 / sub13 | Esc, F1 to normal 2.0mm. Then F1 to rapid-split (release point 0.8mm, RT 0.3mm) | Matched on read back |
| 0x14 b[2]=1 | Keyboard sleep 1800 → 1500 → 1800 s | Matched on read back |
| 0x14 b[2]=1 | Sent with lighting set to 0 | Query returned 60 |
| 0x18 | Changed F7 to F12 on the base layer, and H on the Fn layer | F7 (combo Shift+A) gave `A`. Fn+H moved the cursor left |
| 0x19 | Macro 0 = press and release of h, i (20ms each). Macro 1 empty (offset 8 = 0xFF) | F12 gave `hi`. F11 (`[0x10, x, 1, 0]`) gave `x` (fallback HID) |
| 0x22 MT | Hold b / tap a, [9] = 30 (300ms) | Short press gave `a`, holding gave `b` |
| 0x22 TGL | Left Shift | Each press of F9 toggled between Shift press and release |
| 0x22 DKS | Stage positions 1.0 / 3.4 / 3.0 / 0.5mm. Key 1 = tap at stage 1, key 2 = tap at stage 2 | Shallow press gave `1`, pressing to the bottom gave `12` |
| 0x24 b[2]=0 | Pair 0 = z / x, mode 2 (always), last input wins | Pressing x while holding z switched to x. Releasing x resumed z repeat |
| 0x24 b[2]=0 | 0x81 in byte0 (mode 1 + bit7) | Query returned 0x01. State query mask was 0 (bit7 cannot be written) |
| 0x24 b[2]=3 | Global ON → OFF | No response for either. State query mask was 0xFFFFF when ON, 0 when OFF |
| 0x21 sub7 | Calibrate Esc | Completion report (b[7]=0) arrived |
| 0x21 sub6 | Reset all keys to default | All keys normal 1.8mm. RT sensitivity (+6 / +7) kept the original values. Calibrated flag in 0x01 also stayed 1 |
| 0x23 | Started without ever sending a heartbeat | About 6000 packets per second kept arriving after 2 s. Sending stop dropped it to 0 |

Read-only checks:
- **0x10**: the default HID codes of the 81 keys matched the table in [protocol.md](protocol.md) section 9.
- **0x0E**: there were 22 fixed-function keys ([protocol.md](protocol.md) section 7).

## Measurement method

- **Recording**: recorded the 0x23 stream and the sub1 reports for keys registered with sub2 at the same time, with receive timestamps. The only changes made to the keyboard were a temporary monitor bitmap and the stream flag, both cleared at the end.
- **Values at rest**: recorded for 3 s without touching the keys.
- **Bottom-out values**: within 90 s, pressed every key to the bottom one at a time and took each key's minimum value.
- **Simulation**: implemented [firmware.md](firmware.md) sections 3.2 and 3.3 directly in Python (including C truncating division and the run-length counter cap of 255).
