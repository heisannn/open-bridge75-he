# Official configurator (Short Cut HE Config)

Target: V_2.03.04 (2024-06-07). Reference for compatibility with our own configurator, and for building a feature that imports settings from the official app.

## Structure **[App]**

- A Qt5 32bit app. Communication is handled by the bundled `witmodSdk.dll` (based on hidapi).
- The app writes an "internal number" to b[1], and the SDK rewrites it to the wire cmd right before sending ([protocol.md](protocol.md) section 2.1).
- From 2030-01-01 on, every SDK API fails (time-limited shutdown).
- How it communicates
  - Sends a heartbeat (0x01) every second.
  - Serializes sends through a single queue, alternating "1 send → 1 receive" every 100ms.
  - Resends a macro (0x19) up to 4 times if there is no response.
- Names of UI widgets can be recovered from the Qt meta objects and setupUi. The resources (qrc) contain 325 files. The main ones:
  - `KeyInfo_<model>.config`: key positions and matrix (the source data for [protocol.md](protocol.md) section 9). Arrays per row (`EveryKeyInfo_Row01..07`). Each key has its on-screen position, label (`Character`), default HID code (`Dec`), and so on.
  - `WitmodDefaultConfig.ini`: default settings.

## UI values and protocol values **[App]**

| UI | Value sent |
|---|---|
| Actuation point, release point, RT sensitivity sliders | value ÷ 10 = mm (an integer in 0.1mm units is sent). RT sensitivity starts at 5 (0.5mm) |
| QuickTrigger | OFF gives mode 0, ON gives mode 12, ON with SecondarySensitivity also ON gives mode 13. In mode 0 / 12, actuation point = release point and RT press = RT release are sent |
| AntiShake (debounce) | 0 to 6 (inferred from the slider widget initialization) |
| Lighting brightness, speed | UI value − 1 (0..4) |
| Sleep time | Requires "lighting ≤ keyboard" and "keyboard ≥ 300 s" |

## Features that do nothing on this keyboard **[App+FW]**

- **None of the lighting settings have any effect**. The real keyboard has no lighting (LEDs) **[HW]**. The official app is shared with other models, so it has lighting screens, and profiles also store lighting fields.
- "Six Key" (0x04), "Full Key" (0x05), "Multimedia" (0x06), "MouseKey" (0x13), and "Mouse light" (0x43) are for other models, and the FW ignores them.
- Among the function keys, "layer switch", "battery display", and "connection switch" (type 0x11 with p1 = 8 / 10 / 11) are not implemented in this keyboard's FW.

## Features missing from the official app **[FW]**

- 0x23 (raw sensor value stream) and 0x24 (SOCD) are not in the SDK's translation table, so the app cannot send them.
- 0x10 (read default key layout) is not used.

## Profiles stored on the PC **[HW]**

Location: `%APPDATA%\Short Cut HE Config\UserConfig\`

| File | Contents |
|---|---|
| `WitmodConfig.ini` | Settings of the app itself (autostart, language, theme, minimize to tray, previous version) |
| `appMacro.xml` | Macros |
| `SeniorKeyConfig.xml` | Advanced keys. `item` elements under `configDKS` / `configMT` / `configTGL`. DKS has `keyCode1..4` (assigned keys), `K1A..K4D` (actions for stages 1 to 4 × bindings A to D), `Value1..4` (stage positions, travel 0..36). `strokeNum` holds the DKS default positions |
| `75ALHE\appProfile_Default.xml` | Base layer profile |
| `75ALHE\appProfile_FN.xml` | Fn layer profile |
| `75ALHE\appProfile_Magnetic.xml` | Named "Magnetic", but it contains lighting, functions, and sleep time, with the same structure as the other 2 |
| `75ALHE\<32 hex digits>\` | The same set as the 3 files above. Purpose not confirmed (probably per device, or a backup copy of the profiles) |

Profile XML structure (`<Parameters profileIndex>` → `<config index name>`):
- `light0` / `light1` / `light2`: lighting. `item` has `ledType`, `ledBrightness`, `ledSpeed`, `ledModeR/G/B`, `ledBgR/G/B`, `ledDirection`, and so on (corresponds to 0x07 in [protocol.md](protocol.md) section 10)
- `customColor` / `customLight`: custom lighting. Named presets (`Alphabets`, `WASD+Arrows`, `Custom 3` to `Custom 5`)
- `onboardBinding`: key assignments (empty if left at defaults)
- `keyEvent`, `dpi`, `ribbon`, `bindMacro`
- `function`: `Setting feature1..4`, `SleepTime lightSleepTime keyboardSleepTime` (unit not confirmed. It did not match the keyboard-side 60 / 1800 s)

Notes:
- **Trigger settings (actuation point, RT sensitivity, and so on) were not found in any file**. This was checked while the keyboard held custom RT settings. Trigger settings are most likely stored only in the keyboard, so read them with 0x21 sub5 and back them up before a FW update.
- The detailed meaning of each XML attribute has not been checked yet.
