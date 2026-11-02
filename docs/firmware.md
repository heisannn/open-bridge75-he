# Bridge75 HE (75ALHE) firmware analysis (V1.13.10)

Target: `M484,01,KB,OW,75ALHE,V1.13.10` (built 2024-11-19 10:36:50). Extracted as 0x40000 bytes from file offset 0x6D1008 of the upgrader `Bridge75HE_Upgrader.exe`.

- MCU: Nuvoton M484 (Cortex-M4F, APROM 512KB). USB is HSUSBD (USB 2.0 High-Speed).
- Analysis: static analysis in IDA (ARMv7-M Thumb, base 0x0). Addresses are offsets from the start of the image (= flash addresses). A function name `sub_XXXX` means the function at that address.
- Results verified on hardware are in [measurements.md](measurements.md). For the evidence markers, see the [README](README.md).

---

## 0. Image layout

| Range | Contents |
|---|---|
| 0x0000–0x1FFF | Bootloader (USB ISP). Vectors at 0x0, main=0xF4C |
| 0x1500 | `5A A5 32 32` = bootloader ID (0xA55A + "22"). Embedded in the 0x0D response |
| 0x2000– | Application. Vectors at 0x2000 (SP=0x20005208, Reset=0x22DD) |
| 0x12527–0x1FDFF | Free (zero-filled, about 54KB) |
| 0x1FE00 | `"10:36:50"`, `"Nov 19 2024"`, `"M484,01,KB,OW,75ALHE,V1.13.10"` |
| 0x1FE50 | `FF FF A5 5A` = valid-application signature 0x5AA5FFFF (checked by the bootloader) |
| 0x1FE54 | Bootloader config word 0x4A080006 |
| 0x1FE58 | Boot-key threshold 0xAF (175) |
| 0x1FE60 | 16B: unused-sensor mask (one byte per ADC channel, bit = group) |
| 0x1FE70 | 16B: boot-key mask (section 8) |
| 0x20000–0x20AEA | USB descriptors (HID report descriptors, device, config, strings) |
| 0x20AEB–0x3FFFF | Free (zero-filled, about 125KB) |

- Judging from the initial SP, RAM use is only about 20KB (the M484 has 160KB of SRAM).
- Application IRQs (vectors at 0x2000): IRQ32 TMR0 = 0x6B2C, IRQ33 TMR1 = 0x2AD6, IRQ43 EADC01 = 0x2C5C, IRQ65 HSUSBD = 0x760C, SysTick = 0x66E0.

---

## 1. Hardware

**The real keyboard has no lighting (LEDs)** **[HW]**. The FW contains lighting tasks and commands ("Lighting" in section 5), but this is code shared with other models. Nothing lights up on this keyboard.

### Clock (Reset_Handler 0x22DD → init)
- The PLL generates 192MHz from HIRC (12MHz) (PLLCTL = 0x0008842E: NF=48, NR=3, NO=2). HCLK = 192MHz, PCLK0/1 = 96MHz (PCLKDIV = 0x11).
- HXT (external crystal) is also enabled. TMR1 is clocked from HXT (12MHz per the BSP constant).

### Hall sensor wiring (sub_284A 0x284A)
- 128 sensor positions = **16 ADC channels × 8 groups**. 81 of them are real keys (unused ones are in the mask at 0x1FE60).
- **PA0–PA7 are the group select lines**. They are outputs. They idle High, and only the one group being read is driven Low.
- **PB0–PB15 are EADC0 CH0–CH15** (MFP=1, digital input disabled).
- The firmware does not tell whether the select lines switch sensor power or drive an analog switch. Powering 81 linear Hall sensors all the time would draw several hundred mA, so switching sensor power seems likely (inferred).

### ADC and timer
- **TMR1**: periodic mode, prescaler 0, CMP=1500 → **12MHz ÷ 1500 = 8kHz (every 125µs)**. The interrupt (0x2AD6) issues a software trigger to the EADC (SWTRG bit16 = module 16).
- **EADC**: clock is PCLK 96MHz ÷ 8 = 12MHz (`CLK_CLKDIV0_EADC(8)`).
  - Modules 0–15 = CH0–CH15, triggered by ADINT0, extended sampling 15 clocks.
  - Completion of module 16 raises ADINT0 → modules 0–15 convert in order → completion of module 15 raises ADINT1 → interrupt `EADC01_IRQHandler` (0x2C5C).
  - One trigger performs 17 conversions. Conversion time is roughly 40–50µs (to be checked against the M480 TRM).

### USB (HSUSBD)
- At startup OPER.HISPDEN=1 (High-Speed enabled). On bus reset, CURSPD is read to switch between HS and FS settings.
- Device: VID 0x0416 / PID 0x7372. There are two config descriptors, one for HS and one for FS (other-speed). **In both, every endpoint is 64B with bInterval=1**. On HS the host polls every 125µs, on FS every 1ms.

| IF | Type | Reports | EP |
|---|---|---|---|
| 0 | HID (143B report descriptor) | ID 2 = mouse (5 buttons, X/Y 16bit, wheel 8bit), ID 3 = Consumer (16bit usage 0–0x23C), ID 4 = System Control (0x81–0x83), ID 5 = NKRO keyboard (bitmap of usages 0–0x9F) | 0x81 IN |
| 1 | Boot keyboard (subclass 1 / protocol 1 in one of the descriptors) | Standard 8-byte keyboard (65B) | 0x82 IN |
| 2 | Vendor HID (Usage Page 0xFF1B, Usage 0x91) | ID 1, 63B Input / Output | 0x83 IN, 0x04 OUT |

---

## 2. HID communication and dispatch

### Packet format (same for OUT / IN, 64 bytes)
```
[0] = 0x01 (Report ID)
[1] = wire cmd
[2],[3],[4] = parameters (per command)
[5] = data length
[6..63] = data
```
- Receive path: `HSUSBD_IRQHandler`(0x760C) → EP-D OUT → `hsusbd_ep_handler`(0x743A)
  - If `[1]==0x0F`, it is handled immediately inside the ISR as event 2 (calls `hid_cmd_dispatch` directly)
  - Otherwise `msgq_push(7, len, buf)`(0x6CB0) → task `usb_task_poll`(0x23BE) → `hid_cmd_dispatch`(0xE498)
- `hid_cmd_dispatch`: `if (buf[1] < 0x25) jumptable_0x12230[buf[1]](buf)`
- **Responses are sent asynchronously**. Each handler only sets a bit in the pending flag `dword_200048D0`. `hid_resp_dispatch`(0xE4AE) takes the bits one at a time from the lowest, builds the IN buffer (0x2000406C, all 64B cleared, then `[0]=1`) with the function from table 0x122C4, and sends it. For multi-packet responses, the flag stays set while the builder returns 2.

### wire cmd list (jump table 0x12230)

| cmd | Handler | Response bit → response function | Purpose |
|---|---|---|---|
| 0x01 | 0xD318 | bit0 → 0xD350 | Heartbeat / status |
| 0x02 | 0xD396 | bit1 → 0xD3A2 | Supported side light modes list |
| 0x07 | 0xD3C0 | bit17 → 0xD460 | Keyboard lighting effect set / query |
| 0x08 | 0xD50A | bit16 → 0xD586 | Side lighting set / query |
| 0x09 | 0xD606 | (bit19 → 0xD65E) | Save custom (per-key RGB) lighting |
| 0x0A | 0xD678 | bit6 → 0xD6B4 | Supported keyboard lighting modes list |
| 0x0B | 0xD6D2 | None (reset) | Jump to bootloader |
| 0x0D | 0xD6DE | bit9 → 0xD6EE | Device info (2 packets) |
| 0x0E | 0xD762 | bit7 → 0xD76E | Fixed-function key mask (22B) |
| 0x0F | 0xD87C | None | Real-time RGB stream (sync mode only) |
| 0x10 | 0xE22E | bit8 → 0xE240 | Read default key layout (6 packets) |
| 0x12 | 0xD8E2 | None | Unknown (only stores 11B) |
| 0x14 | 0xD8FA | bit18 → 0xD98E | Lock / sleep / reset |
| 0x15 | 0xD9E2 | bit10 → 0xD9EE | Read SN |
| 0x16 | 0xDA0C | None | Write SN |
| 0x17 | 0xDA20 | None | Light sync (host-driven) mode on/off |
| 0x18 | 0xDA52 | bit14 → 0xDB96 | Keymap write |
| 0x19 | 0xDBA8 | bit15 → 0xDBEA | Macro write |
| 0x20 | 0xD78C | (device→host notification bit20 → 0xDBFC) | Host-driven light effects / custom event key notification |
| 0x21 | 0xDC60 | bit21 → 0xDEB4, bit22 → 0xE08E | HE (magnetic switch) settings |
| 0x22 | 0xE358 | bit23 → 0xE48C | Advanced keys (DKS / MT / TGL) |
| 0x23 | 0xE07C | bit24 → 0xE2B8 | Raw sensor value stream on/off |
| 0x24 | 0xE384 | bit25 → 0xE0DC | SOCD (simultaneous press priority) pair settings |

**Not implemented (ignored)**: 0x00, 0x03, 0x04, 0x05, 0x06, 0x0C (application side; bootloader only), 0x11, 0x13, 0x1A–0x1F, 0x25 and above (0x40–0x49 are also ignored). The official configurator's "Write Six Key" (0x04), "Full Key" (0x05), "Multimedia" (0x06), "MouseKey" (0x13) and "Mouse light mode list" (0x43) have no effect on this model.

**Commands the SDK cannot send**: witmodSdk's logical→wire mapping table only maps to 0x01–0x1C, 0x1E–0x22 and 0x40–0x49, so **0x23 and 0x24 cannot be sent through the official SDK** (they work over raw HID).

---

## 3. Sensor processing (1kHz per key)

### 3.1 Flow
1. Each TMR1 tick (8kHz) converts one group (16 sensors).
2. `EADC01_IRQHandler` → `sub_2C6C`: stores the 16 channel values in `word_20000EB0[group][channel]`, drives the current group's select line High and the next group's Low.
3. When all 8 groups are done (`byte_20000075` goes 7 → 0), a frame event is pushed to queue `unk_2000008C`.
4. Task `sub_316C` waits for the event and processes **all 128 sensors at once**.

→ **Sampling and detection run at 1kHz per key (every 1ms)** **[FW+HW]**. The value of the first group read is up to about 0.9ms old by the time it is processed. No code changes the TMR1 period.

### 3.2 Per-key processing (`sub_316C`, every frame)
1. The raw value (12bit) is stored in `word_200015D8[row][col]` (the 0x23 stream sends this). From here on, calculations use `v = raw value >> 3`.
2. If `v > 400`, it is treated as a sensor fault and no detection is done.
3. If the rest calibration value T (+4..5) is 0, or calibration is running, go to calibration processing (section 3.5).
4. **Noise filter** (the filtered value F is `word_20000FB0`)
   - If `v < F`, the press run-length counter is incremented (max 255) and the release counter is set to 0. Direction = press.
   - If `v > F`, the release run-length counter is incremented and the press counter is set to 0. Direction = release.
   - If `v == F`, both are set to 0 (direction keeps its previous value).
   - Threshold = debounce value + 3. But if the direction is press and `0 ≤ T − v ≤ 3` (right next to the rest position), it is 150.
   - When either run-length counter exceeds the threshold, `F = v`. Once exceeded, `F = v` every frame while movement continues in the same direction. Delay only occurs at the start of movement and at reversals.
5. **Travel calculation** (B = bottom-out calibration value, +3)
   - If `F ≥ T`, travel = 0. The pressed state and RT state are also cleared.
   - Otherwise `n = ((F − B) << 8) / (T − B)` (C truncating division), `travel = (((70·n²) >> 16) + 38) · (256 − n) >> 8`, capped at 36.
6. If travel is the same as the previous frame, processing ends here. **It also ends if the direction is press and travel ≤ 2** (an actuation point of 0.1 / 0.2mm is effectively 0.3mm).
7. If the key is monitored by sub2, its row / col is recorded for the notification (there is only one slot, so a key processed later in the same frame overwrites it).
8. If the key's entry (in the Fn layer while Fn is held) is type 0x17 (advanced key), it is passed to advanced key processing (sub_2E3C) and processing ends.
9. Otherwise trigger detection (section 3.3) runs and the result is applied to the bitmap. If press / release changes, `usb_task_poll` builds a report.

### 3.3 Trigger detection (HE struct: +0 mode, +1 release point / RT exit point, +2 actuation point, +6 RT press sensitivity, +7 RT release sensitivity)
- **mode 0 / 1 (normal)**
  - While pressing, press when `travel ≥ actuation point`.
  - While releasing, release when `travel ≤ release threshold`. The release threshold is the actuation point in mode 0 and the release point in mode 1.
- **mode 12 / 13 (Rapid Trigger)**. The RT exit point is the actuation point in mode 12 and the release point in mode 13.
  - RT inactive: while pressing, press when `travel ≥ actuation point` and activate RT (peak = travel, trough = 0). While releasing, release.
  - RT active, releasing: if pressed and `peak − travel > RT release sensitivity`, release. Trough = travel (updated while released too). Also, if `travel ≤ exit point`, deactivate RT and release.
  - RT active, pressing: if pressed, peak = travel. If released: when `travel == 36`, press. Otherwise press when `travel − trough > RT press sensitivity`, and set peak = travel.
  - The sensitivity comparison is "greater than" (`>`), so **the effective sensitivity is the set value + 0.1mm**.

### 3.4 Filter delay (measured)
Compared with detecting immediately without the filter, the press delay averaged 2.8ms with debounce 3 and 0.5ms with debounce 0 (833 presses, [measurements.md](measurements.md) section 6) **[HW]**. After a reversal, the finger takes several frames to move by the sensitivity anyway, so most of the filter delay is hidden there.

### 3.5 Calibration
- **Automatic learning of the rest value T**: when T is 0 and `|v − previous value| ≤ 3` holds for 64 frames, the value at that moment becomes T.
- **All-key calibration (sub8)**: clears T and the calibrated flag (key descriptor +18) for all keys. After T is learned, a key that stays deeply pressed with `v < T − 40` for 64 frames gets that value recorded as B (+3) and is marked calibrated. It ends about 10000 frames (about 10 seconds) after the last key was calibrated.
- **Single key (sub7)**: does the same for one key only.
- **Requests that set T to 0**: sub6 (all keys to defaults, sub_3A72(0)), sub7 (that key), start of sub8 (all keys). The automatic learning above starts right after. Learning does not check whether the key is pressed, so sending this while holding a key makes the pressed position T ([protocol.md](protocol.md) section 6).
- **b[20] of 0x01**: 1 if 40 or more keys have calibration values (0xC480).
- The default B is 150 (set only on full reset). Calibration values on the real keyboard were 146–170 ([measurements.md](measurements.md) section 4).

---

## 4. HE settings (wire 0x21)

### 4.1 Per-key HE struct
`0x200011B8 + 176·row + 8·col` (6 rows × 22 columns × 8B = 1056B). Saved to flash at 0x42020 (12B signature `"10:36:50"` at 0x42000). Save / load at 0x39D6 / 0x3A40.

| off | Meaning |
|---|---|
| +0 | Mode: 0 / 1 / 12 / 13 |
| +1 | Release point / RT exit point (0.1mm units) |
| +2 | Actuation point (0.1mm units) |
| +3 | Bottom-out calibration value B (ADC>>3, initial value 150) |
| +4..5 | Rest calibration value T (ADC>>3). 0 means auto-learn |
| +6 | RT press sensitivity (0.1mm) |
| +7 | RT release sensitivity (0.1mm) |

Defaults (sub_3A72, on sub6 / reset): mode=0, +1=18, +2=18 (1.8mm), +4..5=0 (auto-learn). +3=150 only on full reset. +6 / +7 are not initialized here.

### 4.2 Subcommands (request `[6]` = sub)

| sub | Request | Action | Response (IN [1]=0x21) |
|---|---|---|---|
| 0 / 1 / 12 / 13 | `[7..28]` 22B bitmap (byte i = column 0..21, bit j = row 0..5), `[29]`→+2 actuation point, `[30]`→+1 release / RT exit point, `[31]`→+6 RT press sensitivity, `[32]`→+7 RT release sensitivity | Sets mode=sub and the 4 values on the selected keys, and schedules a save (sub_C93E) | `[5]=1,[6]=0` |
| 2 | `[7..28]` 22B bitmap (0x20003F97) | Sets the keys monitored for live travel reports. Each time a monitored key's travel changes, a sub1 notification is sent | `[5]=1,[6]=0` |
| 3 | None | Clears the monitor bitmap | `[5]=1,[6]=0` |
| 4 | None | Max travel query | `[5]=2,[6]=4,[7]=36` |
| 5 | `[7]=row,[8]=col` | Key setting query | `[5]=4,[6]=5,[7]=mode,[8]=actuation point(+2),[9]=release point(+1),[10]=RT press(+6),[11]=RT release(+7)` (actually 5 bytes) |
| 6 | None | Resets all keys to defaults (sub_3A72(0)+sub_27E0). Schedules a save | `[5]=1,[6]=0` |
| 7 | `[7]=row,[8]=col` | Single-key calibration (section 3.5). Sets +4..5 at that position to 0. row / col are not range-checked | Immediately `[5]=2,[6]=7,[7]=1`. On success `[7]=0` (automatic notification), on timeout `[7]=2` |
| 8 | `[7]=0` start / `[7]≠0` abort | All-key calibration (section 3.5) | `[5]=24,[6]=8,[7]=state(1=running, 0=finished with every key calibrated (seen on the real keyboard), 4=aborted / finished),[8..29]=bitmap of calibrated keys(22B)`. Re-sent on each progress update |
| 9 | `[7]=0` query / `[7]=1` set, `[8]=1/2/4/8` | Polling rate (kHz). Values other than 1/2/4/8 are ignored. Default 8. Only the hidden measurement mode (section 9) uses the value. Even a query calls sub_C954 (schedules a settings save) | `[5]=2,[6]=9,[7]=rate` |
| 10 | `[7]=0` query / `[7]=1` set, `[8]=value` | Debounce ("AntiShake" in the official configurator). Threshold in section 3.2 = value + 3. Default 3. Not range-checked. Even a query calls sub_C954 | `[5]=2,[6]=10,[7]=value` |

- **Automatic notification sub1 from the device** (bit22, 0xE08E): `[5]=3,[6]=1,[7]=row,[8]=col,[9]=travel(0..36)`. Only for keys registered with sub2. Corresponds to `sigBackDynamicDisplaySensitivityNum` in the official configurator.
- The official configurator sends `(mode, LEFT, RIGHT, Press, Release)` with sub0/1/12/13. The mapping is `[29]`=LEFT=actuation point, `[30]`=RIGHT=release / RT exit point, `[31]`=RT press, `[32]`=RT release.

---

## 5. Other commands

### 0x18 Keymap write (0xDA52)
- `[2]` = layer: **0 = base (0x200040F8), 2 = Fn layer (0x20004308, used while the Fn key is held), 1 = accepted but nothing is saved**
- `[4]` = packet number 0..9, `[5]` = length (56; 24 for the last packet), `[6..]` = 14 × 4B entries
- Entries are in row-major matrix order (index = row·22 + col, 132 entries = 528B)
- sub_EFAE runs when packet 0 of layer 0 is received
- In the Fn layer, keys whose bit is set in the fixed-function key mask (0x20003EB8, same as the 0x0E response) are not overwritten
- Defaults (sub_CCC6) are built from the key descriptors. Base = `[0, +12]`. Fn layer: if +13 is 0, `[0, +12]` (same as base), otherwise `[+13, +14, +15, +16]` (FW-specific Fn function). **The fixed-function key mask is "keys where +13 is not 0"**. If +13 is 1 (the Fn key itself), both layers get `[1, +12]`. There are also special cases where +13 is 16 / 21 / 0x80
- Key resolution (sub_F3A8): except for the Fn key itself (+13 == 1), a key pressed while Fn is held uses the Fn layer entry, otherwise the base entry. Even if the Fn layer entry is 0, it does not fall back to the base
- **Both layers are saved to flash when packet 9 of layer 2 is received** (0x40020 / 0x40230). Sending only layer 0 does not save anything
- Response: `[1]=0x18,[5]=0` per packet

#### Keymap 4B entry `[type, p1, p2, p3]`
Press is handled by table 0x1232C, release by table 0x12398, and lighting by 0x10E88 (sub_F3A8 0xF3A8). The tables are arrays of function pointers indexed by type. For example, type 0x01 (Fn) press is 0xEC0E and release is 0xEC7E.

| type | Meaning |
|---|---|
| 0x00 | Normal key: p1 = HID Keyboard Usage (goes through SOCD processing) |
| 0x01 | Fn (Fn layer while held) |
| 0x02 | KB lighting mode switch (p1=0 for next) |
| 0x03 | KB brightness ± (p1=0 to increase, 0..4) |
| 0x04 | KB speed ± (0..4) |
| 0x05 | Lighting related (0x8800, details unknown) |
| 0x06 | KB color (p1=2 cycles through 8 preset colors, p1=1 toggles colorful) |
| 0x07 | Consumer (media) key: p1 = bit number (0..63). Mapping table not identified |
| 0x08 | Custom lighting slot select (p1 = slot) |
| 0x09 | Custom lighting record / apply |
| 0x0A–0x0D | Side lighting on/off, brightness, speed, color |
| 0x10 | Macro: p1 = fallback HID for an empty macro, p2 = macro number 0..9 |
| 0x11 | System function: p1=0 toggle all-key lock, 1 6KRO/NKRO (p2=0 toggle / 1 6KRO / 2 NKRO), 2 toggle Win lock, 9 OS switch (p2=1 Win / 3 Mac), 12 toggle flag 0x20000, 15 set internal value |
| 0x12 | Mouse: p1=0 for button mask p2, p1=1 for wheel p2 |
| 0x13 | Combo: p1, p2, p3 = up to 3 HID Usages |
| 0x16 | Custom event key: while the driver is connected, notifies the host with 0x20; when not connected, acts as the default HID code |
| 0x17 | Advanced key reference: p1 = slot 0..39 |
| 0x1A | SOCD toggle: p2=1 toggles global on/off, p2=0 toggles pair number p1 |

Lighting keys (0x02–0x06, 0x08–0x0D) are passed to the lighting task via msgq(2) (0x8C3E).

### 0x19 Macro write (0xDBA8 → sub_C41A 0xC41A)
- `[2]` = macro number 0..9, `[4]` = packet 0..4, `[5]` = length. Data is placed at offset 58·pkt within the macro
- Flash: macro i is a fixed 256B at `0x41000 + 0x100·i`
- Received data is placed in a RAM buffer (offset 58·pkt), and the `[5]` length is added to a running total. When the total reaches exactly 256, that macro is written and the total is reset to 0 (sub_C41A)
  - **Each macro must be sent as exactly 256B in total (58×4 + 24)**
  - Sending the same packet twice makes the total skip past 256. That macro is not written, and the total is not reset, so all later macros are misaligned too
- Packet 0 of macro 0 erases sector 0x41000 (4KB). Packet 4 of macro 9 commits (sub_C84A)
- Packets with `[2]` > 9 or `[4]` > 4 are ignored and get no response (cmd_19 0xDBA8)
- Response: `[1]=0x19,[5]=0`

Macro data format (0xEDB2, 0xEED4):
```
hdr[0]   : ?
hdr[1]&3 : repeat mode 0=once, 1=N times, 2=toggle (press again to stop), 3=repeat while held
hdr[2..3]: BE16 byte count of the event area
hdr[4..5]: BE16 repeat count N
hdr[8]   : 0xFF means empty macro (events start at +8)
event 4B: [0]=HID Usage or mouse button mask,
          [1]: bit4=press(1)/release(0), low nibble≠0 means mouse button event,
          [2..3]=BE16 delay until the next event (ms, 4ms steps, min 4)
```
- The only emptiness check is hdr[8] == 0xFF (key_press_type10 0xEDB2). If empty, type 0x10's p1 (fallback HID) is sent (sub_E638). A zero-filled slot is treated as a zero-length macro, and the fallback HID is not sent

### 0x22 Advanced keys (0xE358)
- `[2]` = slot 0..39, `[6..45]` = 40B → `0x20001820 + 40·slot`. **Saved to flash at 0x40968 (0x640B) when slot 39 is received**
- `[5]` is ignored and 40B is always copied. The slot is not range-checked (cmd_22 0xE358)
- p1 of keymap type 0x17 is used directly as the slot number in sub_2E42. This is not range-checked either
- Response: `[1]=0x22`

40B struct (sub_2E42 0x2E42). `[0]` = kind:
- `1` **DKS**: `[1]`=stage 1 press point, `[2]`=stage 2 press point (bottom side), `[3]`=stage 3 return point, `[4]`=stage 4 return point (travel 0..36). For binding i=0..3, `[5+8i..8+8i]` = keymap 4B entry, `[9+8i],[10+8i],[11+8i],[12+8i]` = actions for stages 1–4
  - Action values (events are press=1/release=0): 0 = release, 1 = tap (release→press→release), 2 = press and hold, 3 = re-press (release→press). Value 2 in stage 2 / stage 3 does nothing if the previous stage was also 2. Stage 4 always ends with a release (interpretation partly inferred)
- `2` **MT (hold / tap)**: uses the key's HE actuation point (+2) and release point (+1). `[1..4]` = hold entry, `[5..8]` = tap entry, `[9]` = hold time
  - sub_310E is called every 1ms from frame processing (sub_316C). For each pressed MT slot, it increments a counter once every 10 calls (stops at 250). If the counter equals `[9]` right after incrementing, the hold fires
  - So `[9]` is in 10ms units and only 1–250 is valid. With 0 or 251 and above it never becomes equal, and hold never fires
- `3` **TGL (toggle)**: each time the actuation point is crossed, the key in `[1..4]` alternates between press and release

### 0x24 SOCD pairs (0xE384) — cannot be sent from the SDK
20 pairs × 4B (0x200049B4): `[0]` bit0-2 = mode (0 = disabled, 1 = only when bit7 is set, 2 = always), bit7 = enabled flag, bit4-6 = press state (used by the FW at runtime). `[1]` = behavior, `[2]` = key A (HID), `[3]` = key B (HID)
- Active when (key_press_type00 0xEA10): `(byte0 & 7) == 2`, or `(byte0 & 0x87) == 0x81`
- Only global on/off (`[2]=3`) and type 0x1A keys change bit7. A write (`[2]=0`) does not change it

Behavior (key_press / release_type00 0xEA10 / 0xEB4A):
- 0 = last input wins (on release, the earlier key is re-pressed)
- 1 = A wins
- 2 = B wins
- 3 = pressing both releases both (neutral)

| Request | Action |
|---|---|
| `[2]=0,[4]=0` | Writes pairs 0–9 (`[6..45]` = 10×4B). byte0 becomes `existing & 0xF8 \| new & 7`, byte1–3 are copied as is. Response `[2]=0` |
| `[2]=0,[4]=1` | Writes pairs 10–19. Schedules a save (sub_C992) |
| `[2]=1` | Query. Response is 2 packets: `[2]=1,[4]=0/1,[5]=40`, data 10×4B (byte0 is `& 7`, sub_E0DE) |
| `[2]=2` | Status query. Response `[2]=2,[5]=4,[6]=global ON,[7..9]=BE24 enabled mask (bit(19−i)=bit7 of pair i),[10]=0xFA,[11]=byte_20000C78` |
| `[2]=3,[6]&1` | Global on/off (settings word bit20). Sets bit7 of all pairs to the same value and calls sub_C954. **No response bit is set, so no response is returned** |
| Otherwise | Does nothing, only sets the response bit (the `[2]=0` response) |

### 0x23 Raw sensor value stream (0xE07C / 0xE2BA) — cannot be sent from the SDK
- `[6]&1` = enable (flag 0x40000)
- While enabled, `sub_316C` sets response bit 24 when it starts processing a frame (not set if sending of the previous frame has not finished). Each row packet is built from the values at send time, so rows can be up to 1 frame apart. Response is `[1]=0x23,[3..4]=BE16 row+1 (1..6),[5]=44,[6..49]=22 columns × BE16 raw ADC value` (1 frame = 6 packets)
- On the real keyboard, 1000 frames per second arrived **[HW]**

### 0x20 Host-driven light (0xD78C)
- Only effective in light sync mode (turned ON with 0x17)
- `[4]` = KB effect kind: 0/1/3 → (`[5]`,`[6]`,`[7]`), 6/7/8/14/16/11/12, 100(=17). `[7..9]` is RGB, valid only if it matches the preset color table 0x11944 (8 colors)
- `[34]` = side kind: 0/1/4/5/3, parameters `[35..37]`
- The meaning of each effect is largely inferred
- **Device→host notification** (type 0x16 key): `[1]=0x20,[5]=5,[6..9]=keymap entry 4B,[10]=1 press/0 release` (ring buffer of 20 entries)

### Lighting (no effect, since this keyboard has no LEDs)
- **0x07 KB lighting**
  - Query with `[2]=1`
  - Set: `[2]=0`, `[6]`=mode (a value in the 0x0A list, 10=custom, 100=internal 17; inferred to be music-reactive)
  - `[7..14]` = 8B of the per-mode 9B block: `[0]`=brightness 0..4, `[1]`=speed 0..4, `[2..4]` unknown, `[5..7]`=RGB
  - `[15]&7` = option (inferred to be direction), `[16]&1` = colorful, `[17]&1` = global flag (inferred to be OFF)
  - Response `[5]=12,[6]=mode,[7..14]=8B,[15]=direction code,[16]=colorful,[17]=flag`. Also sent as an automatic notification when lighting is changed with the Fn key (if the driver is connected)
- **0x0A** Supported KB modes list: response `[5]=18`, data = `02 01 03 09 04 00 0E 0F 06 07 0B 0C 10 08 A5 5A 04 04` (14 modes before A5 5A; the 04 04 after it is inferred to be the max brightness / speed)
- **0x02** Side list: `A5 5A 04 04` (0 modes = this model has no side lighting). Since the supported list is empty, 0x08 settings are effectively all rejected (queries are answered)
- **0x09** Custom lighting: `[2]`=slot 0/1, `[4]`=packet 0..7, `[5]`=54, `[6..]`=RGB×18 keys (matrix order row·22+col). Packet 7 commits, switches to mode 10 and saves (0x40650 / 0x407DC, 132×3B each)
- **0x0F** Real-time RGB: same format as 0x09 (`[2]=0` KB / `1` side, side is 3 packets). Sync mode only. No response
- **0x17** Sync mode: `[6]&1`. Automatically turned off if 0x0F / 0x20 / 0x17 stop for 1.5 seconds

### 0x14 Lock / sleep / reset (0xD8FA)
| `[2]` | Action |
|---|---|
| 0 | Shortcut disable: `[13]`=Win key, `[14]`=Shift+Tab, `[15]`=Alt+F4, `[16]`=Alt+Tab (non-zero disables; byte_2000403D bit0..3) |
| 1 | Sleep time: `[9..10]`=BE16 lighting sleep (s), `[11..12]`=BE16 keyboard sleep (s). Ignored if both are 0. If only one is 0, it is saved as is |
| 2 | Sleep query. Response `[2]=2,[9..10]`=lighting (default 60),`[11..12]`=keyboard (default 1800). If a saved value is 0, 60 / 1800 is returned instead (resp_bit18 0xD98E) |

- Handled every time regardless of `[2]`: `[6]==1` triggers a **factory reset** (sub_EFDE), `[7]&1` → KB lighting flag bit1, `[8]&1` → side lighting flag bit1 (meaning inferred)
- **`[7]` and `[8]` are overwritten every time, so the current values must be sent**
- Regardless of `[2]` (even for a query), sub_C954 (schedules a settings save) is called every time (cmd_14 0xD8FA)
- Response: `[1]=0x14,[5]=0` (only a query has `[2]=2` and the values)
- Shortcut disable only takes effect when Win lock (0x100) is ON and not in Mac mode (0xE63E)

---

## 6. Basic commands

- **0x01 Heartbeat**
  - The payload is ignored. Sets the driver-connected flag and starts a 100ms timer with a 1.5-second timeout. On timeout, the sub2 monitor bitmap is also cleared (sub_D2A8). The 0x23 raw sensor value stream flag stays as is
  - Response (mostly fixed values):
    ```
    [5]=6
    [6]=Win lock state (0x100)
    [7]=2, [8]=1
    [9]=0  (charging state)
    [10]=100 (battery %, fixed because wired)
    [11]=0, [12]=0, [13]=0, [14]=0
    [15]=1
    [20]=calibrated flag (1 if 40 or more keys have calibration values, 0xC480)
    ```
- **0x0D Device info** (2 packets)
  - First: `[4]=0,[5]=50,[6..]="M484,01,KB,OW,75ALHE,V1.13.10"`. But when 0x1500 is 0xA55A, `[11..12]` is overwritten with the bootloader's 2B ("22" in this image) → on the real keyboard `"M484,22,KB,OW,75ALHE,V1.13.10"` **[HW]**
  - Second: `[4]=1,[6..16]="Nov 19 2024",[17]=',',[18..25]="10:36:50"`
  - Field meanings (from the upgrader's comparison logic): chip M484 / second = bootloader version / kind KB / customer code OW / product name 75ALHE / FW version
- **0x15** Read SN: response `[5]=32,[6..37]`
- **0x16** Write SN: saves the 32B at `[6..37]` (record ID 0x1C). No response
- **0x0B Jump to boot**: when `[2]=0`, writes 0x5AA55A55 to 0x1FE50 and resets via AIRCR → re-enumerates as the bootloader (0416:3F01). No response
- **0x10 Default layout**: 6 packets. `[3..4]`=BE16 next row number (1..5, 0xFFFF for the last), `[5]=22`, `[6..27]`=default HID Usage of each column (key descriptor +12)
- **0x0E**: `[5]=22`, `[6..27]`=fixed-function key mask (one byte per column, bit = row)

---

## 7. Matrix, storage layout, defaults

- **Logical matrix**: 6 rows × 22 columns (132 positions). Matrix order is always row·22 + col
- **Key descriptor table**: 0x11F04 (132 pointers, structs in SRAM)
  - +0..2 = RGB, +12 = default HID, +13 = default type, +14 = default parameter, +17 = (row<<5 | col), +18 = calibrated flag
  - Side LED table: 0x12114 (51 entries)
- **Physical sensor table**: 0x11AF4 ([group][channel] → pointer to key descriptor, 128 entries). Unused mask at 0x1FE60
- **Layers**: two, base + Fn (a third at 0x20004518 is referenced internally, but there is no way to write it)
- **Flash storage area** (APROM from 0x40000, outside the FW image):

| Address | Contents |
|---|---|
| 0x40000 | Signature (12B, FW build time `"10:36:50"`) |
| 0x40020 | Base layer (528B) |
| 0x40230 | Fn layer (528B) |
| 0x40650 / 0x407DC | Custom lighting slot 0 / 1 (396B each) |
| 0x40968 | Advanced keys (1600B = 40×40) |
| 0x41000 + 0x100·i | Macros 0..9 |
| 0x42000 / 0x42020 | HE signature (build time) / HE key table (1056B) |

- There is also a separate record store with IDs (sub_5DEE / sub_5D28): ID 1 = build date, 2 = build time, 3 = lighting state, 4 = misc settings (6KRO / Win lock / Mac-related flags, lock bits, sleep time, OS mode, debounce, polling), 5–21 = per-mode 9B KB lighting, 22–27 = side, 0x1C = SN. Physical address not identified
- **Startup check** (sub_C99C 0xC99C): compares the signature at 0x40000 and records 1 and 2 with its own build date and time. If any differ, it rewrites all settings with defaults and writes a new signature (sub_C500(1)). The HE key table is also not loaded if the signature at 0x42000 differs (0x3A40). **Any change to the FW changes the build date and time, so settings are reset on every update** (confirmed going from V1_12_08 → V1.13.10 **[HW]**)
- **Defaults**:
  - Actuation point / release point = 18 (1.8mm), max travel 36 (3.6mm)
  - Debounce 3, polling 8kHz
  - Sleep: lighting 60 seconds / keyboard 1800 seconds
  - Brightness and speed range 0..4
- While writing to flash (sub_C500), TMR1 is stopped (scanning pauses)

---

## 8. Bootloader and FW update

### Boot decision (0xF4C)
If any of the following holds, the application is not started and the device stays in ISP (0416:3F01).
- The top byte of the config word at 0x1FE54 is not 0x4A
- Config bit19 is set and PA1 is Low
- Config bits 20–23 have the value 1
- **The boot-time key check matches**
- 0x1FE50 is not 0x5AA5FFFF

### Boot-time key check (sub_10E8 / sub_1504)
- Scans the Hall sensors 8 times using the same wiring as the application (PA0–PA7 / PB0–PB15). Config word bit0 also supports a 16 groups × 8 channels wiring (this keyboard is 8 × 16).
- Keys with `ADC>>3 < 175` (0x1FE58) and `≤ 400` are treated as pressed. If that set exactly matches the mask at 0x1FE70, it enters ISP.
- The mask is (ch2, grp5), (ch3, grp6), (ch4, grp5). Converted to (row, col) with the physical sensor table and key descriptor table, these are (3,3), (4,3), (3,5), that is **S, X, F**.
- → **Plugging in USB while holding only S+X+F deeply should enter the bootloader even if the application is broken** (not verified). Replugging without sending anything starts the application normally.

### Write (sub_BAC / sub_AE4)
- `01 0C 00 <seqHi> <seqLo> <len>` + inverted data. seq 0 erases from 0x2000 to the end of the application. If seq is not sequential, `[2]=2` is returned. 0xFFFF ends the transfer and resets.
- **There is no checksum or signature verification**. The `FF FF A5 5A` at 0x1FE50 is just written as part of the image.
- For the procedure details, see [protocol.md](protocol.md) section 12.

---

## 9. Hidden features

### Polling rate measurement mode (Fn+O+K)
- While Fn is held, pressing both physical keys whose default HID codes are 0x12 (O) and 0x0E (K) sets flag 0x10000 (sub_F3A8 0xF3A8). Releasing any of them clears it.
- While active, the mouse (Report ID 2) continuously outputs input that draws a circle with a radius of 300 counts (sub_2AF6). No buttons are pressed.
- The send interval is set by the **sub9 polling rate**.
  - 8: on every USB IN completion (`hsusbd_ep_handler` 0x743A). 8kHz on HS
  - 1 / 2 / 4: once every 8 / 4 / 2 ADC interrupts (`sub_2C6C`). 8kHz ÷ 8 / 4 / 2 = 1 / 2 / 4kHz
- **The sub9 value (`byte_200040F1`) is only read in these 2 places and in save, load and response**. It does not affect the interval of normal keyboard reports.

### Other
- 0x23 (raw sensor value stream), 0x24 (SOCD) and 0x10 (default key layout) are not used by the official configurator ([protocol.md](protocol.md)).

---

## 10. Hints for modification

- **Scan period**: the TMR1 CMP is `MOVW R1, #0x5DC` (1500) at 0x2A10. 750 gives 2kHz and 375 gives 4kHz (per key).
- **ADC clock**: `MOV.W R2, #0x70000` (÷8) at 0x28BE. To shorten the period, it must be raised to shorten the conversion time.
- **Constants whose time changes with the period** (all counted in frames): debounce (value + 3), the 150 near rest, the 64 for auto-learning and calibration, the calibration timeout of 10000, and the measurement mode dividers 8 / 4 / 2.
- **Period limit**: the time from switching groups to the next conversion must be longer than the sensor output settling time. Currently it is about 75µs out of 125µs. If sensor power is being switched, its rise time sets the limit.
- **Processing time**: `sub_316C` (about 2KB) processes 128 sensors in one frame. Roughly 100–300µs (needs measuring).
- **Free space**: 0x12527–0x1FDFF (about 54KB) and 0x20AEB–0x3FFFF (about 125KB). New code can be placed there and patched in by branching from existing code.
- **Signature**: keep `FF FF A5 5A` at 0x1FE50. Changing the build date/time strings resets all settings at startup (section 7).
- **Recovery**: the bootloader is not rewritten. S+X+F should enter ISP (section 8, not verified).
- For the expected effects of modifications, see [measurements.md](measurements.md) section 7.
