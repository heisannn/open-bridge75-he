# Bridge75 HE (75ALHE) HID protocol specification

Scope: official configurator V_2.03.04 and witmodSdk.dll, upgrader V1.0.0.3, firmware V1.13.10. For the evidence markers ([App] [FW] [Up] [HW]), see [README](README.md).

Index notation: **`b[n]` is the absolute position in the 64-byte packet** (b[0]=Report ID).

---

## 1. Device identification

| Item | Value |
|---|---|
| Normal mode | VID:PID **0416:7372** (Nuvoton M484, USB 2.0 High-Speed) **[FW+HW]** |
| Bootloader | **0416:3F01** **[FW+Up]** |
| Interface used | **Interface 2**. On Windows the path contains `mi_02`, **Usage Page 0xFF1B / Usage 0x91** **[FW+HW]** |
| Report | Report ID `0x01`, Input / Output with 63 data bytes (64 bytes including the Report ID) **[FW+HW]** |

- Interface 0 is mouse, media keys, system control and NKRO keyboard. Interface 1 is the boot keyboard. Details in [firmware.md](firmware.md) section 1.
- The SDK device table also lists `1FC9:E8C7/E8D8`, `4A4C:E987/E988`, `1FC9:0091` for other OEMs **[App]**.
- `witmodSdk.dll` contains a **time bomb: every API fails from 2030-01-01 on** **[App]**. Custom apps should not use the SDK and should talk raw HID directly.

## 2. Packet format

```
b[0]  = 0x01 (Report ID)
b[1]  = command (wire cmd)
b[2]  = parameter (layer / macro number / slot / subtype)
b[3]  = parameter (usually 0; seq high byte for FW update)
b[4]  = parameter (split packet number, etc.)
b[5]  = data length
b[6..63] = data (up to 58 bytes)
```
- Responses (Input) use the same format, with the same cmd in b[1] **[App+FW+HW]**.
- **Responses are asynchronous and queued** **[FW+HW]**: each handler only sets a flag, and packets are sent later one at a time. Some requests return multiple response packets (0x0D, 0x10, 0x23, 0x24 query, 0x21 sub8). Dispatch on b[1] (and b[6] for 0x21).
- The SDK does not send packets with b[5] > 58 (except 0x21) **[App]**.

### 2.1 Official configurator internal numbers and wire cmd
The official configurator writes an internal number to b[1], and the SDK rewrites it to the wire cmd just before sending **[App]**. The upgrader writes wire cmds directly, and they match the converted values **[Up]**. **Custom apps should use wire cmds directly.**

| Configurator internal number | wire cmd |
|---|---|
| 0x00–0x1B | internal number + 1 |
| 0x1C–0x25 | 0x40–0x49 |
| 0x26–0x2A | 0x1E–0x22 |

Because they are not in this table, **wire 0x23 (raw sensor values) and 0x24 (SOCD) cannot be sent from the official configurator** (they work over raw HID) **[FW+HW]**.

## 3. Basic communication procedure

1. Open Interface 2.
2. Get device info with `0x0D` and the serial number with `0x15` (section 5).
3. **Keep sending `0x01` at intervals of 1.5 s or less** (the official configurator uses a 1 s period). If it stops, the FW treats the driver as disconnected, and live travel reports (0x21 sub1) and custom event notifications (0x20) stop **[FW]**.
   - However, **the 0x23 raw sensor value stream does not stop**. On timeout, only the sub2 monitor targets are cleared **[FW+HW]** (after starting the stream without a heartbeat, about 6000 packets per second still arrived 2 s later).
   - A stream may still be running from a previous abnormal exit, so it helps to send a 0x23 stop right after opening. A stop has no response and no save, so it is harmless.
4. Send settings commands. Serialize sends through a single queue, and read on a separate thread (the official configurator alternates "1 send → 1 receive" on a 100 ms period) **[App]**.
5. Onboard writes (0x18 / 0x19 / 0x22) are **saved to flash when the designated last packet is received**. If you stop partway, nothing is saved (sections 7 and 8) **[FW]**.
6. **Keymap, macros and advanced keys cannot be read from the keyboard**. The custom app must keep their contents (the official configurator also keeps profiles on the PC). Readable items are HE settings (0x21 sub5, etc.), lighting (0x07), sleep (0x14), SOCD (0x24) and the default key layout (0x10).

## 4. Command list

| wire | Purpose | Implemented on this keyboard |
|---|---|---|
| 0x01 | Heartbeat / status | Yes **[HW]** |
| 0x02 | Side light supported mode list | Yes (list is empty = no side light) |
| 0x07 | Keyboard lighting effect set / query | Yes (query **[HW]**) |
| 0x08 | Side lighting set / query | Only query is effective (all sets are rejected) |
| 0x09 | Save custom (per-key RGB) lighting | Yes |
| 0x0A | Keyboard lighting supported mode list | Yes |
| 0x0B | Enter bootloader | Yes (resets without a response) |
| 0x0C | FW data write | Bootloader only |
| 0x0D | Device info (2 packets) | Yes **[HW]** |
| 0x0E | Mask of fixed-function keys that the Fn layer cannot override | Yes **[HW]** |
| 0x0F | Real-time RGB stream | Yes (only in sync mode) |
| 0x10 | Read default key layout (6 packets) | Yes (not used by the official configurator) **[HW]** |
| 0x12 | Unknown (only stores 11 bytes) | Yes |
| 0x14 | Lock / sleep / factory reset | Yes (sleep query and set **[HW]**) |
| 0x15 / 0x16 | Serial number read / serial number write | Yes (do not send 0x16) |
| 0x17 | Light sync (host-driven) mode on/off | Yes |
| 0x18 | Keymap write | Yes **[HW]** |
| 0x19 | Macro write | Yes **[HW]** |
| 0x20 | Host-driven light (PC→KB) / custom event key notification (KB→PC) | Yes |
| 0x21 | **HE (magnetic switch) settings** | Yes **[HW]** |
| 0x22 | **Advanced keys (DKS / MT / TGL)** | Yes **[HW]** |
| 0x23 | Raw sensor ADC value stream | Yes (cannot be sent from the official configurator) **[HW]** |
| 0x24 | **SOCD pair settings** | Yes (cannot be sent from the official configurator) **[HW]** |

**Commands the FW ignores** **[FW]**: 0x00, 0x03–0x06, 0x11, 0x13, 0x1A–0x1F, 0x25 and above (including 0x40–0x49). The official configurator's "Six Key(0x04) / Full Key(0x05) / Multimedia(0x06) / MouseKey(0x13) / Mouse light(0x43)" are for other models and have no effect on this keyboard.

**This keyboard has no lighting (LEDs)** **[HW]**. The FW accepts the lighting commands (0x02, 0x07, 0x08, 0x09, 0x0A, 0x0F, 0x17, and the host-driven light part of 0x20), but they have no effect. The custom event key notification part of 0x20 (KB→PC) works independently of lighting.

---

## 5. Status and information

### 0x01 Heartbeat
- Request: `01 01 00 00 00 01 01`. The FW ignores the payload **[FW]**.
- Response (wired keyboard, so battery fields are fixed) **[FW+HW]**:

| byte | Value / meaning |
|---|---|
| b[5] | 6 |
| b[6] | Win lock state (when this is 1, the official configurator enables PC-side suppression of Alt+Tab, etc.) **[App+FW]** |
| b[7], b[8] | 2, 1 (fixed) |
| b[9] | Charging state (0=not charging 1=charging 2=fully charged; always 0 on this keyboard) **[App+FW]** |
| b[10] | Battery % (always 100 on this keyboard) |
| b[11] | Connection 0=USB / 1=2.4G (0 on this keyboard) |
| b[12]–b[14] | 0 (for other models: dongle state / onboard state / mode) |
| b[15] | 1 |
| b[20] | Calibrated flag (1 when 40 or more keys have calibration values). 0 right after a FW update, 1 after calibrating in the official configurator **[HW]** |

### 0x0D Device info (2 packets)
- Request `01 0D 00...` → wait about 250 ms, then read 2 packets **[App+Up]**.
- First: b[4]=0, b[5]=50, b[6..] = `"M484,22,KB,OW,75ALHE,V1.13.10"` **[HW]**
  - Comma-separated: MCU / bootloader version / type KB / customer code OW / **product name (hidName) 75ALHE** / FW version
  - The string in the image is `"M484,01,..."`, but the FW overwrites the second field with an ID value from the bootloader area, so it reads `22` on the real keyboard **[FW+HW]**
  - Older FW used underscore-separated versions such as `V1_12_08` **[HW]**
- Second: b[4]=1, b[6..16] = `"Nov 19 2024"`, b[17] = `,`, b[18..25] = `"10:36:50"` (build date and time) **[HW]**

### 0x15 Serial number read
Response b[5]=32, b[6..37] = ASCII serial number (32 characters). **0x16 (serial number write) is for factory use. Do not send it.**

### 0x10 Default key layout
Returned in 6 packets. b[3..4] = next row number (BE16; 1..5, 0xFFFF for the last), b[5]=22, b[6..27] = default HID usage for each column of that row **[FW]**.
- On the real keyboard, the values for all 81 keys matched the table in section 9. The Fn key cell is 0xFA **[HW]**.
- Positions with no key on this keyboard still contain defaults for other models (PrtSc, Pause, numpad, JIS keys, etc.) **[HW]**.

---

## 6. HE (magnetic switch) settings: wire 0x21

Requests are `01 21 00 00 00 18 <sub> ...` (b[5]=24 fixed, b[6]=subcommand).

### Key selection bitmap (22 bytes)
Keys whose **bit[row] of byte[col]** (col 0–21, row 0–5) is 1 are selected **[App+FW+HW]**. Matrix positions are in the table in section 9.

### Units and ranges **[FW+HW]**
- Travel is **0..36 (0.1 mm units, max 3.6 mm)**. The sub4 response is also 36.
- Defaults: actuation point and release point both **18 (1.8 mm)**, mode=0, polling 8 kHz, debounce 3.
- FW internal per-key struct (8 bytes): +0 mode, +1 release point / RT exit point, +2 actuation point, +3 bottom-out ADC calibration value, +4..5 rest ADC calibration value (0 = auto-learn), +6 RT press sensitivity, +7 RT release sensitivity. There is no way to read the calibration values.

### Subcommands

| b[6] | Function | Request | Response |
|---|---|---|---|
| 0 / 1 / 12 / 13 | **Trigger settings** (b[6] is the mode value) | b[7..28]=bitmap, **b[29]=actuation point, b[30]=release point / RT exit point, b[31]=RT press sensitivity, b[32]=RT release sensitivity** | b[5]=1, b[6]=0 **[HW]** |
| 2 | Set **monitor targets for live travel reports** | b[7..28]=bitmap | b[5]=1, b[6]=0 |
| 3 | Clear monitor targets | none | b[5]=1, b[6]=0 |
| 4 | Query max travel | none | b[5]=2, b[6]=4, b[7]=36 **[HW]** |
| 5 | Query key settings | b[7]=row, b[8]=col | b[6]=5, b[7]=mode, b[8]=actuation point, b[9]=release point, b[10]=RT press, b[11]=RT release **[HW]** |
| 6 | Reset all keys to defaults. **The rest calibration value T is also reset to 0** (re-learned automatically). RT sensitivity (+6 / +7) is not reset, and the bottom-out calibration value and calibrated flag are kept **[HW]** | none | b[5]=1, b[6]=0 |
| 7 | Single-key calibration. Sets that key's T to 0 on receipt. **Does not check the row / col range** (out-of-range values write outside the RAM array) | b[7]=row, b[8]=col | Immediately b[6]=7, b[7]=1 (in progress) → then automatically b[7]=0 on success or b[7]=2 on timeout |
| 8 | All-key calibration. Clears T for all keys and the calibrated flag on start | b[7]=0 start / nonzero abort | b[5]=24, b[6]=8, b[7]=state (1=in progress, 0=finished with every key calibrated **[HW]**, 4=aborted / finished), b[8..29]=bitmap of calibrated keys. Re-sent on each progress update. **There is no way to query, so sending b[7]=0 always starts calibration** |
| 9 | Polling rate. A query also schedules a flash save | b[7]=0 query / 1 set, b[8]=1/2/4/8 (kHz) | b[6]=9, b[7]=value **[HW]** |
| 10 | Debounce. A query also schedules a flash save | b[7]=0 query / 1 set, b[8]=value | b[6]=10, b[7]=value **[HW]** (the setting persists after replugging) |
| 1 (notification) | Live travel report | Sent by the KB each time the travel of a key registered with sub2 changes | b[5]=3, b[6]=1, b[7]=row, b[8]=col, b[9]=travel (0..36) **[HW]** |

- **Keep hands off the keys when sending sub6 / 7 / 8** **[FW]**.
  - The FW learns T as "the value after |v − previous value| ≤ 3 has held for 64 frames (about 64 ms)", and does not check whether the key is pressed ([firmware.md](firmware.md) section 3.5). If a key is held while sending, the pressed position becomes T.
  - If a key is held at bottom-out, the bottom-out calibration value B cannot be recorded (`v < T − 40` never happens), so that key stops responding. If it was held partway, it becomes calibrated, but the actuation point shifts deeper.
  - T is saved together with the HE table, so replugging does not restore it. To fix it, calibrate again.
- sub9 / sub10 schedule a flash save even on a query. Do not query them periodically, e.g. from a GUI **[FW]**.
- The sub1 notification has only one slot. If several monitored keys change in the same frame, only the last key is reported **[FW+HW]**.
- In the press direction, when travel is 2 or less, no notification and no detection happen (see next section) **[FW]**.

### Mode values (sub0/1/12/13) **[FW]**
| Value | Behavior | Official configurator UI |
|---|---|---|
| 0 | Normal: press when travel ≥ actuation point while pressing down, release when travel ≤ actuation point while returning | QuickTrigger OFF |
| 1 | Separate press and release points: press at the actuation point, release at release point b[30] | Secondary only (normally not reachable from the UI) |
| 12 | Rapid Trigger: RT becomes active past the actuation point. Release when the key returns from a peak by more than the RT release sensitivity, press again when it goes down from a trough by more than the RT press sensitivity. travel=36 is always pressed. RT ends when the key returns to the actuation point | QuickTrigger ON |
| 13 | RT + separate RT exit point (RT end is decided by b[30]) | QuickTrigger + SecondarySensitivity |

- In mode 0 / 12 the official configurator sends b[29]=b[30] and b[31]=b[32]. Slider value / 10 = mm, and the initial RT sensitivity is 5 (0.5 mm) **[App]**.
- **RT sensitivity is an "exceeds" check** (`sensitivity < change`). Travel is in 0.1 mm units, so **it triggers once movement is 0.1 mm larger than the set value** (e.g. a 0.4 mm setting triggers at 0.5 mm) **[FW]**. To get the intended value, send 0.1 mm less.
- **In the press direction, changes with travel ≤ 2 are ignored**, so an actuation point of 0.1 / 0.2 mm actually triggers at 0.3 mm **[FW]**.
- The detailed detection algorithm is in [firmware.md](firmware.md) section 3.

### Polling rate and debounce in practice
- Sensors are read and evaluated per key at **1 kHz (every 1 ms)**. The sub9 value does not change this **[FW+HW]**.
- Over a High-Speed USB connection, the host polls every 125 µs (8 kHz). This is also unrelated to sub9 **[FW]**. Through USB 1.1 ports or some KVMs / hubs (Full-Speed), it becomes 1 ms.
- **The sub9 value only affects the hidden polling rate test mode**. While Fn+O+K is held, the keyboard keeps sending mouse input that draws a circle, at the configured rate ([firmware.md](firmware.md) section 9) **[FW]**.
- **Debounce (sub10)** is a noise filter that "accepts a change once changes in the same direction have continued for more than (value + 3) frames" **[FW]**.
  - It delays detection at the start of movement and at press / return turnarounds. Measured against no filter, **press is delayed by 2.8 ms on average with value 3 (default) and 0.5 ms on average with value 0** **[HW]** ([measurements.md](measurements.md) section 6).
  - The official configurator slider range is 0 to 6 (inferred) **[App]**.
  - **Do not send 252 or more**. The FW does not check the range, the threshold reaches or exceeds the limit of the consecutive-count counter (255), and travel almost never updates **[FW]** (inferred from code).

### 0x23 Raw sensor value stream (not available in the official configurator) **[FW+HW]**
- Start: `01 23 00 00 00 01 01`, stop: `01 23 00 00 00 01 00` (b[6] bit0).
- While enabled, 6 packets arrive per detection frame (1 ms): b[1]=0x23, b[3..4]=BE16 row number + 1 (1..6), b[5]=44, b[6..49]=22 columns × BE16 raw ADC value (12-bit). Cells with no key are 0.
- Values are before filtering (the FW divides them by 8 before use). They decrease as the key is pressed deeper. On the real keyboard, about 1890–2030 at rest and about 1170–1360 at bottom-out **[HW]**.
- If the 6 packets of the previous frame have not finished sending, that frame is skipped **[FW]**. On the real keyboard, 1000 frames per second arrive with almost no drops **[HW]**.

---

## 7. Keymap: wire 0x18

```
01 18 <layer> 00 <pkt> <len>  [entry 4B] × 14
```
- **layer**: `0`=base layer, `2`=Fn layer (while Fn is held). `1` is accepted but nothing is saved **[FW]** (the official configurator also sends only 0 and 2).
- **132 cells (6 rows × 22 columns) in row-major order** (index = row·22 + col), 14 entries per packet: pkt 0–8 have len=56, pkt 9 has 6 entries (len=24). Empty cells are `00 00 00 00` **[App+FW]**.
- Each packet gets a response with b[1]=0x18, b[5]=0.
- **Both layers are saved to flash when pkt 9 of layer 2 is received**. Sending only the base layer does not save, so always send both layers, even with no changes **[FW]**.
- In the Fn layer, fixed-function keys indicated by the 0x0E mask are not overridden **[FW]**.
- **Default Fn layer** **[FW]** (reset routine sub_CCC6):
  - Keys not in the 0x0E mask have the same default HID code as the base layer (`[0x00, HID, 0, 0]`). Pressing them while holding Fn works like a normal key.
  - Keys in the mask have FW-specific Fn functions. Their contents cannot be read.
  - The mask on the real keyboard has 22 keys: Esc, F1–F12, Del, Home, PgUp, A, S, PgDn, RShift, LWin, Fn **[HW]**.
  - So **reading 0x10 (default HID codes) and 0x0E (mask) lets you build the default keymap for both layers**.
- While Fn is held, only Fn layer entries are used. **A key whose Fn layer entry is `00 00 00 00` produces no input** (it does not fall back to the base layer) **[FW]**. Do not send an Fn layer filled with zeros.

### Key entry `[type, p1, p2, p3]`

| type | Content | Evidence |
|---|---|---|
| 0x00 | Normal key: p1 = HID Keyboard usage (including modifiers 0xE0–0xE7). `00 00 00 00` = disabled | App+FW |
| 0x01 | Fn | App+FW |
| 0x02 | Lighting mode switch (p1=0 for next) | App+FW |
| 0x03 | Brightness ± (p1=0 to increase; range 0..4) | App+FW |
| 0x04 | Speed ± (0..4) | App+FW |
| 0x05 | Lighting related (details unknown) | FW |
| 0x06 | Color (p1=2 cycles through 8 preset colors, p1=1 toggles colorful) | FW |
| 0x07 | Media (Consumer): p1 = FW internal bit number 0..63. Official configurator assignments: Volume+=16, Volume−=17, Mute=18; the 16 items on the media screen are, in order, 16,17,18,19,20,21,22,23,48,50,52,35,32,56,57,33 (the bit number to function mapping is not identified) | App+FW |
| 0x08 | Custom lighting slot select (p1 = slot) | App+FW |
| 0x09 | Custom lighting record / apply | App+FW |
| 0x0A–0x0D | Side lighting (this keyboard has no side lighting) | FW |
| 0x10 | Macro: p1 = HID sent instead when the macro is empty, **p2 = macro number 0..9**. "Empty" means only a slot whose offset 8 is 0xFF (section 8) | App+FW |
| 0x11 | System function (table below) | App+FW |
| 0x12 | Mouse: p1=0 means button mask p2 (1=left 2=right 4=middle 8=back 16=forward), p1=1 means wheel p2 (1=up, 0xFF=down) | App+FW |
| 0x13 | Combo: p1, p2, p3 = up to 3 HID usages (the official configurator uses `[0x13, mod, key, 0]` or `[0x13, mod1, mod2, key]`) | App+FW |
| 0x16 | Custom event key: notifies the PC via 0x20 while the driver is connected, acts as the default HID code when not connected (the official configurator uses `[0x16, 0, 0, keycode]` and runs text input etc. on the PC side) | App+FW |
| 0x17 | Advanced key: p1 = 0x22 slot 0..39. The FW uses it without a range check (sub_2E42), so do not assign 40 or more | App+FW |
| 0x1A | SOCD toggle: p2=1 toggles global on/off, p2=0 toggles pair number p1 | FW |

This keyboard has no lighting, so lighting types (0x02–0x06, 0x08, 0x09, 0x0A–0x0D) have no effect when assigned **[HW]**.

p1 for type 0x11 (system function) **[FW]**:
| p1 | Action |
|---|---|
| 0 | Toggle all-key lock |
| 1 | 6KRO / NKRO (p2: 0=toggle, 1=6KRO, 2=NKRO) |
| 2 | Toggle Win lock (the official configurator puts 0=toggle / 1=lock / 2=unlock in p2) |
| 9 | OS switch (p2: 1=Win, 3=Mac) |
| 12 | Toggle internal flag (unknown) |
| 15 | Set internal value (unknown) |

- The official configurator also has p1=8 (layer switch), 10 (battery display), 11 (connection switch: USB/2.4G/BT), but this keyboard's FW does not implement them.
- The official configurator's "system hotkeys" are `[0x13, mod, key, 0]`, 12 kinds: Alt+F4 / Alt+Tab / Win+D / Win+E / Win+L / Ctrl+C / Ctrl+X / Ctrl+V / Ctrl+F / Ctrl+N / Ctrl+P / Ctrl+S **[App]**.

---

## 8. Macros (0x19) and advanced keys (0x22)

### 0x19 Macros: 10 slots × 256 bytes fixed
```
pkt0   : 01 19 <idx> 00 00 3A  <slot bytes 0..57>
pkt1-3 : 01 19 <idx> 00 <n> 3A <slot bytes 58n .. 58n+57>
pkt4   : 01 19 <idx> 00 04 18  <slot bytes 232..255>
```
256-byte slot layout **[App+FW]**:

| offset | Content |
|---|---|
| 0 | 0 |
| 1 | Repeat mode (low 2 bits): 0=once, 1=N times, 2=toggle (press again to stop), 3=repeat while held |
| 2–3 | Byte count of the event section (BE16) = event count × 4 |
| 4–5 | Repeat count N (BE16) |
| 6–7 | 0 |
| 8– | Events (4 bytes each). If offset 8 is 0xFF, the macro is empty. The rest is zero-filled |

Event `[code, flags, delayHi, delayLo]`:
- code: HID usage, or mouse button mask (1=left 2=right 4=middle)
- flags: bit4 = press (1) / release (0); a nonzero low nibble means a mouse event (the official configurator sets bit0)
- delay: wait time until the next event (ms, BE16, 4 ms steps, minimum 4)

Save conditions **[FW]**: pkt0 of macro 0 erases the flash sector, and pkt4 of macro 9 commits. **Always send all 10 slots, 256 bytes each**. Wait for each packet's response (b[1]=0x19) before sending the next.
- The FW sums b[5] of the received packets and writes the slot to flash **when the total is exactly 256** (sub_C41A).
- If the same packet is received twice, the total skips past 256 and that slot is not written. The total does not reset to 0, so later slots are also misaligned. **Do not resend even if there is no response** (the official configurator resends up to 4 times, which is dangerous).
- Packets with a macro number greater than 9 or a pkt greater than 4 are ignored, with no response.

**Empty macros** **[FW]**: only a slot whose offset 8 is 0xFF counts as empty.
- When a type 0x10 key is pressed and its macro is empty, it sends the fallback HID in p1 (key_press_type10 → sub_E638).
- The official configurator sends unused slots filled with zeros. These are treated as zero-length macros, and the fallback HID is not sent.
- To use the fallback HID, send unused slots with offset 8 set to 0xFF (the rest can be 0). With `[0x10, x, 1, 0]` assigned to F11 and slot 1 sent in this form, F11 produced x **[HW]**.

### 0x22 Advanced keys: 40 slots × 40 bytes
```
01 22 <slot 0..39> 00 00 32  <40 bytes>
```
- **Saved to flash when slot 39 is received**. The official configurator sends the slots in use, then sends the rest filled with zeros up to 39 **[App+FW]**.
- The FW ignores b[5] and always copies 40 bytes (the official configurator puts b[5]=0x32). **It does not check the slot number range**, so sending 40 or more writes outside the RAM array **[FW]**.
- Entries in the table below use the 4-byte format from section 7. Offsets are positions within the 40 bytes.

| [0] Kind | Layout |
|---|---|
| 1 = DKS | [1]=stage 1 press point, [2]=stage 2 press point (bottom side), [3]=stage 3 return point, [4]=stage 4 return point (all travel 0..36). For each bind i=0..3, [5+8i..8+8i]=entry, [9+8i..12+8i]=actions for stages 1–4 (0=release, 1=tap, 2=press and hold, 3=re-press) |
| 2 = MT | [1..4]=hold entry, [5..8]=tap entry, [9]=hold time (ms÷10). Press / release detection uses the key's HE actuation point and release point. Hold time details are below the table |
| 3 = TGL | [1..4]=entry. Alternates press / release each time the actuation point is crossed |

MT hold time ([9]) **[FW]**:
- In the 1 ms frame processing, the FW increments the counter of a pressed MT slot by 1 every 10 frames (every 10 ms). The counter stops at 250 (sub_310E).
- If the counter equals [9] right after incrementing, the key becomes a hold.
- So **valid values are 1–250 (10–2500 ms)**. With 0 or 251 and above, they never become equal, and the key always acts as a tap.

### 0x24 SOCD (cannot be set from the official configurator) **[FW]**
20 pairs × 4 bytes:

| byte | Content |
|---|---|
| [0] | bit0-2 = mode (0 = disabled, 1 = only when bit7 is set, 2 = always), bit7 = enabled flag. The FW uses bit4-6 to record press state |
| [1] | Behavior (0=last input wins, 1=A wins, 2=B wins, 3=pressing both releases both) |
| [2], [3] | Key A, key B (HID usage) |

- SOCD is active when `mode == 2`, or when `mode == 1` and bit7 is set (`(byte0 & 0x87) == 0x81`) (key_press_type00).
- **bit7 cannot be changed by a write (b[2]=0)**. It can only be changed by global on/off (b[2]=3, all pairs at once) and type 0x1A keys. Writing 0x81 to byte0 was saved as 0x01 **[HW]**.
- A query (b[2]=1) also cannot read bit7. Only the mask from the status query (b[2]=2) shows it.

| Request | Action | Response |
|---|---|---|
| b[2]=0, b[4]=0 / 1 | Write pairs 0–9 / 10–19 (b[6..45] = 10 × 4B). Only the low 3 bits of byte0 are applied (`existing & 0xF8 \| new & 7`); byte1–3 are written as is. b[4]=1 schedules a flash save | b[2]=0 |
| b[2]=1 | Query | 2 packets: b[2]=1, b[4]=0/1, b[5]=40, data 10 × 4B (byte0 is returned `& 7`). Response verified on hardware **[HW]** |
| b[2]=2 | Status query | b[2]=2, b[5]=4, b[6]=global on, b[7..9]=BE24 mask (bit(19−i) = bit7 of pair i), b[10]=0xFA. Response verified on hardware **[HW]** |
| b[2]=3 | Global on/off (b[6] bit0). Also sets bit7 of all pairs to the same value and schedules a flash save. A status query after ON returned mask 0xFFFFF (all 20 pairs) **[HW]** | **None** (nothing arrives even after waiting 0.8 s **[HW]**). Check the result with a status query |
| Other | Does nothing | b[2]=0 |

---

## 9. 75ALHE key matrix (6 rows × 22 columns, 81 keys)

Source: the `KeyInfo_75ALHE.config` resource in the official configurator. Each cell is "label / default HID usage". Columns 16–21 are unused. The keymap send order, the HE bitmap and the 0x23 layout use this (row, col).

| row/col | 0 | 1 | 2 | 3 | 4 | 5 | 6 | 7 | 8 | 9 | 10 | 11 | 12 | 13 | 14 | 15 |
|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|
| 0 | Esc<br>0x29 |  | F1<br>0x3A | F2<br>0x3B | F3<br>0x3C | F4<br>0x3D | F5<br>0x3E | F6<br>0x3F | F7<br>0x40 | F8<br>0x41 | F9<br>0x42 | F10<br>0x43 | F11<br>0x44 | F12<br>0x45 |  | Del<br>0x4C |
| 1 | `~<br>0x35 | 1!<br>0x1E | 2@<br>0x1F | 3#<br>0x20 | 4$<br>0x21 | 5%<br>0x22 | 6^<br>0x23 | 7&<br>0x24 | 8*<br>0x25 | 9(<br>0x26 | 0)<br>0x27 | -_<br>0x2D | =+<br>0x2E |  | Back<br>0x2A | Home<br>0x4A |
| 2 | Tab<br>0x2B | Q<br>0x14 | W<br>0x1A | E<br>0x08 | R<br>0x15 | T<br>0x17 | Y<br>0x1C | U<br>0x18 | I<br>0x0C | O<br>0x12 | P<br>0x13 | [{<br>0x2F | ]}<br>0x30 |  | \\\|<br>0x31 | PgUp<br>0x4B |
| 3 | Caps<br>0x39 |  | A<br>0x04 | S<br>0x16 | D<br>0x07 | F<br>0x09 | G<br>0x0A | H<br>0x0B | J<br>0x0D | K<br>0x0E | L<br>0x0F | ;:<br>0x33 | '"<br>0x34 |  | Enter<br>0x28 | PgDn<br>0x4E |
| 4 | L_Shift<br>0xE1 |  | Z<br>0x1D | X<br>0x1B | C<br>0x06 | V<br>0x19 | B<br>0x05 | N<br>0x11 | M<br>0x10 | ,<<br>0x36 | .><br>0x37 | /?<br>0x38 |  | R_Shift<br>0xE5 | Up<br>0x52 | End<br>0x4D |
| 5 | L_Ctrl<br>0xE0 | L_Win<br>0xE3 | L_Alt<br>0xE2 |  |  |  | Space<br>0x2C |  |  |  | R_Alt<br>0xE6 | Fn<br>(type 1) |  | Left<br>0x50 | Down<br>0x51 | Right<br>0x4F |

(In the config file, the labels for 0x31 and 0x34 were swapped; they are corrected here to match the HID usages.)

---

## 10. Lighting

> **The real keyboard has no lighting (LEDs)** **[HW]**. The FW still contains lighting code shared with other models and responds to the commands, but nothing lights up. Custom apps do not need lighting features. The following is kept for the record.

### 0x0A Supported mode list
Response data: `02 01 03 09 04 00 0E 0F 06 07 0B 0C 10 08 | A5 5A | 04 04` → 14 modes, a separator, and max brightness and speed of 4 (the max value interpretation is inferred) **[FW]**.

### 0x07 Keyboard lighting
- Query: b[2]=1 **[FW+HW]**
- Set: b[2]=0, b[5]=14 **[App+FW]**

| byte | Content |
|---|---|
| b[6] | Mode (a value from the 0x0A list; 10=custom, 100=music-reactive, inferred) |
| b[7] | Brightness 0..4 (the official configurator sends UI value − 1) |
| b[8] | Speed 0..4 (same as above) |
| b[9..11] | Official configurator ledModeR/G/B (FW use not identified) |
| b[12..14] | RGB (the value the FW uses as the color; ledBgR/G/B in the official configurator) |
| b[15] | Direction (low 3 bits) |
| b[16] | bit0 = colorful |
| b[17] | bit0 = global flag (inferred to be OFF) |
| b[62..63] | The official configurator puts the low 16 bits of the send time here (inferred to be unused by the FW) |

Response: b[5]=12, b[6]=mode, b[7..14], b[15]=direction, b[16]=colorful, b[17]=flag. It is also sent automatically when lighting is changed with the Fn key (while the driver is connected).

### 0x09 Per-key custom RGB
```
01 09 <slot 0/1> 00 <pkt 0..7> <len>  R G B × 18 keys (row-major)
```
pkt 0–6 have len=54, pkt 7 has 6 keys (len=18). **pkt 7 commits, switches to mode 10 (custom) and saves** **[App+FW]**.

### 0x17 / 0x0F / 0x20 Host-driven lighting
- `01 17 00 00 00 01 <bit0: 1=start 0=end>` enters sync mode.
- In sync mode, send 0x0F (real-time RGB in the same format as 0x09) or 0x20 (effect selection).
- **If 0x0F / 0x20 / 0x17 stop for 1.5 s, sync mode ends automatically** **[FW]**.

### 0x20 Custom event key notification (KB → PC)
Sent when a type 0x16 key is pressed or released: b[1]=0x20, b[5]=5, b[6..9]=the key's entry, b[10]=1 press / 0 release **[App+FW]**.

---

## 11. 0x14 Lock / sleep / factory reset

| b[2] | Content |
|---|---|
| 0 | Shortcut disable: b[13]=Win key, b[14]=Shift+Tab, b[15]=Alt+F4, b[16]=Alt+Tab (nonzero disables). Only effective when Win lock is ON and not in Mac mode **[FW]** |
| 1 | Sleep time: b[9..10]=seconds until lighting turns off (BE16; meaningless on this keyboard, which has no lighting), b[11..12]=seconds until the keyboard sleeps (BE16). Ignored if both are 0. If only one is 0, the 0 is saved as is (the meaning of 0 is unverified) **[FW]**. Defaults are 60 / 1800. The official configurator requires "lighting ≤ keyboard" and "keyboard ≥ 300", and also puts a sleep-disable flag in b[13] (FW handling unverified) |
| 2 | Sleep query: response b[2]=2, b[9..12] (defaults 60 / 1800 verified on hardware **[HW]**). If a stored value is 0, the default (60 / 1800) is returned in its place **[FW]** |

**Caution** **[FW]**:
- Regardless of b[2], **b[6]==1 triggers a factory reset**.
- b[7] bit0 and b[8] bit0 (lighting flags) are overwritten every time. The official configurator always sends 0.
- Regardless of b[2] (even for a query), every request schedules a flash save. Do not query periodically.

Response: b[1]=0x14, b[5]=0 (values are present only for a query).

---

## 12. Firmware update

FW image layout **[FW]**:
- 0x0000–0x1FFF: USB bootloader
- 0x2000–: application
- 0x1FE00: build date / time and ID string (`M484,01,KB,OW,75ALHE,V1.13.10`, from 0x1FE1C)
- 0x1FE50: application valid signature `FF FF A5 5A`

Procedure **[Up+FW]**:
1. In normal mode, send 0x01 → 0x0D and compare with the image's ID string. Abort if the MCU (1st field), customer code (4th field) or product name (5th field) differ.
2. Sending `01 0B 00...` makes the FW write 0x5AA55A55 to 0x1FE50 and reset, and the device re-enumerates as 0416:3F01.
3. Open the bootloader and send the 256 KB image 48 bytes at a time:
   ```
   01 0C 00 <seqHi> <seqLo> 32  ~data[48]  dummy dummy
   ```
   - Data is sent bit-inverted. The last 2 bytes are dummies (the upgrader fills them with random values 1..200).
   - seq counts up from 0. seq 0 erases from 0x2000 on.
   - The write address is the offset from the start of the image as is. Addresses below 0x2000 (the bootloader area) are not written.
   - Response b[2]: 0=OK, 2=sequence mismatch (the upgrader treats 1/2/3 as errors 102/103/105).
   - The final seq is 5461 (16 bytes, len=0x12).
4. `01 0C 00 FF FF 32` completes the update, and the device resets.

- The bootloader does not verify the image checksum or signature. Even if the application is corrupted, the bootloader itself is not overwritten, so it can be rewritten **[FW]**.
- **Plugging in USB while holding only S+X+F pressed deep appears to enter the bootloader without starting the application** **[FW]** (unverified; [firmware.md](firmware.md) section 8).
- The upgrader matches both the normal mode PID and the bootloader PID when searching for devices, so it can start writing to a device that stayed in the bootloader (inferred) **[Up]**.

**A FW update resets all settings stored in the keyboard** **[FW+HW]**.
- Settings are in flash at 0x40000–0x42FFF and are not erased by the update. However, at startup the FW compares the settings area signature with its own build date and time (`"10:36:50"` / `"Nov 19 2024"`), and if they differ, rewrites all settings with defaults. The same applies to the HE key table (including calibration values).
- Verified by updating the real keyboard from V1_12_08 to V1.13.10. Keymap, Fn layer, macros, advanced keys, lighting, trigger settings, calibration values, debounce and so on were all reset to defaults.
- If a custom app handles updates, it should back up the readable settings (step 6 of section 3) before the update and write them back afterward. Calibration must be redone.

> A failed write can leave the keyboard unable to boot. The bootloader is designed to survive, but take great care.

---

## 13. Checklist for building a custom app

- Keep sending 0x01 at about a 1 s period (1.5 s timeout).
- Responses are asynchronous. Dispatch on b[1] (and b[6] for 0x21). Requests and responses are not always one-to-one.
- Onboard save conditions:
  - 0x18: send up to pkt 9 of the Fn layer (layer 2)
  - 0x19: send all of macros 0–9, 256 bytes each
  - 0x22: send up to slot 39
- Keymap, macros and advanced keys cannot be read from the keyboard. Keep them in the app. The default keymap can be built from 0x10 and 0x0E (section 7).
- Do not send an Fn layer filled with `00 00 00 00` (those keys stop working while Fn is held).
- 0x14 with b[6]=1 is a factory reset. b[7]/b[8] are overwritten every time.
- 0x21 sub8 cannot be used as a query (sending it starts all-key calibration).
- 0x21 sub6 / 7 / 8 re-learn the rest calibration value. Have the user release all keys before sending (section 6).
- Do not send 252 or more to sub10 (debounce).
- Do not resend 0x19 even if there is no response (if the b[5] total skips past 256, the slot is not written). To use the macro fallback HID, set offset 8 of unused slots to 0xFF (section 8).
- Set the advanced key MT hold time ([9]) to 1–250 (10–2500 ms).
- 0x24 b[2]=3 (SOCD global on/off) returns no response. Writes do not change bit7 of SOCD pairs (section 8).
- Values the FW does not range-check: the 0x22 slot number, the 0x21 sub7 row / col, and p1 of keymap type 0x17. Send only in-range values.
- 0x14 and 0x21 sub9 / sub10 schedule a flash save even on a query. Do not query them periodically.
- 0x23 does not stop when the heartbeat stops. Always stop it when done. Stopping it right after opening also covers a previous abnormal exit.
- RT sensitivity acts 0.1 mm larger than the set value. An actuation point of 0.1 / 0.2 mm acts as 0.3 mm.
- Do not send: 0x16 (serial number write). Do not send 0x0B / 0x0C unless intended.
- A FW update resets all settings stored in the keyboard.
- Features not in the official configurator: 0x23 (raw ADC value stream), 0x24 (SOCD), 0x10 (default layout read).
