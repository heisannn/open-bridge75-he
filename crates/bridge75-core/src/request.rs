//! Building request packets.
//!
//! Write commands save to flash "when the designated last packet is received",
//! so send all returned packets in order (docs/protocol.md section 13).

use crate::Result;
use crate::advanced::{self, AdvancedKey};
use crate::cmd;
use crate::he::{Debounce, PollingRate, TriggerConfig};
use crate::info::SleepTimes;
use crate::keymap::{ADVANCED_SLOTS, Keymap, Layer, MACRO_SLOTS};
use crate::macros::{self, Macro};
use crate::matrix::{CELLS, KeyBitmap, KeyPos};
use crate::packet::{MAX_DATA_LEN, Packet};
use crate::socd::SocdPairs;

/// 0x01 heartbeat. Keep sending it at intervals of 1.5 seconds or less.
pub fn heartbeat() -> Packet {
    Packet::new(cmd::HEARTBEAT).with_data(&[1])
}

/// 0x0D device info (returned in 2 packets).
pub fn device_info() -> Packet {
    Packet::new(cmd::DEVICE_INFO)
}

/// 0x15 SN read.
pub fn serial_number() -> Packet {
    Packet::new(cmd::SERIAL_NUMBER)
}

/// 0x10 default keymap (returned in 6 packets).
pub fn default_keymap() -> Packet {
    Packet::new(cmd::DEFAULT_KEYMAP)
}

/// 0x0E mask of fixed-function keys that cannot be overridden on the Fn layer.
pub fn fixed_fn_mask() -> Packet {
    Packet::new(cmd::FIXED_FN_MASK)
}

// 0x14 performs a factory reset when b[6]==1, regardless of b[2]. Always put 0 in b[6].
// b[7] / b[8] (lighting flags) are overwritten every time. Send 0, as the official configurator does.
fn system(sub: u8) -> Packet {
    let mut p = Packet::new(cmd::SYSTEM).with_params(sub, 0, 0);
    p[6] = 0;
    p
}

/// 0x14 sleep time query.
pub fn sleep_query() -> Packet {
    system(2)
}

/// 0x14 set sleep time.
pub fn sleep_set(times: SleepTimes) -> Result<Packet> {
    times.validate()?;
    let mut p = system(1);
    p[5] = 11;
    p.put(9, &times.light.to_be_bytes());
    p.put(11, &times.keyboard.to_be_bytes());
    Ok(p)
}

// 0x21 always has b[5]=24. b[6] is the subcommand.
fn he(sub: u8) -> Packet {
    let mut p = Packet::new(cmd::HE);
    p[5] = 24;
    p[6] = sub;
    p
}

/// sub0 / 1 / 12 / 13: trigger settings for the selected keys.
pub fn he_set_trigger(keys: &KeyBitmap, config: &TriggerConfig) -> Result<Packet> {
    config.validate()?;
    let mut p = he(config.mode as u8);
    p.put(7, &keys.0);
    p.put(
        29,
        &[
            config.actuation.raw(),
            config.release.raw(),
            config.rt_press.raw(),
            config.rt_release.raw(),
        ],
    );
    Ok(p)
}

/// sub2: keys to monitor for live travel reports. A sub1 notification arrives each time a monitored key's travel changes.
pub fn he_monitor(keys: &KeyBitmap) -> Packet {
    let mut p = he(2);
    p.put(7, &keys.0);
    p
}

/// sub3: clears the monitored keys.
pub fn he_clear_monitor() -> Packet {
    he(3)
}

/// sub4: max travel query.
pub fn he_max_travel() -> Packet {
    he(4)
}

/// sub5: trigger settings query for one key. The response has no matrix position, so wait for each response before sending the next.
pub fn he_query_key(pos: KeyPos) -> Packet {
    let mut p = he(5);
    p.put(7, &[pos.row, pos.col]);
    p
}

/// sub6: resets the trigger settings of all keys to the defaults.
pub fn he_reset_triggers() -> Packet {
    he(6)
}

/// sub7: single-key calibration.
pub fn he_calibrate_key(pos: KeyPos) -> Packet {
    let mut p = he(7);
    p.put(7, &[pos.row, pos.col]);
    p
}

/// sub8: starts calibration of all keys (there is no way to query it; sending it always starts it).
pub fn he_calibrate_all_start() -> Packet {
    he(8)
}

/// sub8: aborts calibration of all keys.
pub fn he_calibrate_all_abort() -> Packet {
    let mut p = he(8);
    p[7] = 1;
    p
}

/// sub9: polling rate query.
pub fn he_polling_rate_query() -> Packet {
    he(9)
}

/// sub9: set polling rate.
pub fn he_set_polling_rate(rate: PollingRate) -> Packet {
    let mut p = he(9);
    p.put(7, &[1, rate.khz()]);
    p
}

/// sub10: debounce query.
pub fn he_debounce_query() -> Packet {
    he(10)
}

/// sub10: set debounce.
pub fn he_set_debounce(value: Debounce) -> Packet {
    let mut p = he(10);
    p.put(7, &[1, value.value()]);
    p
}

/// 0x18: keymap for one layer (10 packets).
fn keymap_layer(layer: Layer, map: &Keymap) -> Vec<Packet> {
    const PER_PACKET: usize = 14;
    map.0
        .chunks(PER_PACKET)
        .enumerate()
        .map(|(pkt, entries)| {
            let data: Vec<u8> = entries.iter().flat_map(|e| e.0).collect();
            Packet::new(cmd::KEYMAP)
                .with_params(layer as u8, 0, pkt as u8)
                .with_data(&data)
        })
        .collect()
}

/// 0x18: both the base layer and the Fn layer (20 packets).
///
/// Both layers are saved when the last packet of the Fn layer is received, so always send both.
/// Do not fill the Fn layer with `none` (those keys would stop working while Fn is held).
pub fn keymaps(base: &Keymap, fn_layer: &Keymap) -> Vec<Packet> {
    debug_assert_eq!(CELLS.div_ceil(14), 10);
    let mut packets = keymap_layer(Layer::Base, base);
    packets.extend(keymap_layer(Layer::Fn, fn_layer));
    packets
}

/// 0x19: macros for all 10 slots (50 packets).
///
/// The first packet of macro 0 erases the sector, and the last packet of macro 9 commits it.
pub fn macros(slots: &[Option<Macro>; MACRO_SLOTS as usize]) -> Result<Vec<Packet>> {
    let mut packets = Vec::with_capacity(50);
    for (index, slot) in slots.iter().enumerate() {
        let bytes = match slot {
            Some(m) => m.encode()?,
            None => macros::EMPTY_SLOT,
        };
        for (pkt, chunk) in bytes.chunks(MAX_DATA_LEN).enumerate() {
            packets.push(
                Packet::new(cmd::MACRO)
                    .with_params(index as u8, 0, pkt as u8)
                    .with_data(chunk),
            );
        }
    }
    Ok(packets)
}

/// 0x22: advanced keys for all 40 slots (40 packets). Saved when slot 39 is received.
pub fn advanced_keys(
    slots: &[Option<AdvancedKey>; ADVANCED_SLOTS as usize],
) -> Result<Vec<Packet>> {
    slots
        .iter()
        .enumerate()
        .map(|(index, slot)| {
            let bytes = match slot {
                Some(key) => key.encode()?,
                None => advanced::EMPTY_SLOT,
            };
            Ok(Packet::new(cmd::ADVANCED_KEY)
                .with_params(index as u8, 0, 0)
                .with_data(&bytes))
        })
        .collect()
}

/// 0x23: start / stop the raw sensor value stream. While enabled, 6 packets arrive every 1ms.
pub fn raw_sensor_stream(enable: bool) -> Packet {
    Packet::new(cmd::RAW_SENSOR).with_data(&[enable as u8])
}

/// 0x24: write all 20 SOCD pairs (2 packets).
pub fn socd_write(pairs: &SocdPairs) -> [Packet; 2] {
    [0, 1].map(|page| {
        let data: Vec<u8> = pairs[page * 10..page * 10 + 10]
            .iter()
            .flat_map(|p| p.encode())
            .collect();
        Packet::new(cmd::SOCD)
            .with_params(0, 0, page as u8)
            .with_data(&data)
    })
}

/// 0x24: SOCD pair query (returned in 2 packets).
pub fn socd_query() -> Packet {
    Packet::new(cmd::SOCD).with_params(1, 0, 0)
}

/// 0x24: SOCD status query.
pub fn socd_status() -> Packet {
    Packet::new(cmd::SOCD).with_params(2, 0, 0)
}

/// 0x24: SOCD global on/off.
pub fn socd_set_global(on: bool) -> Packet {
    Packet::new(cmd::SOCD)
        .with_params(3, 0, 0)
        .with_data(&[on as u8])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::he::{Travel, TriggerMode};

    #[test]
    fn heartbeat_bytes() {
        assert_eq!(&heartbeat().0[..7], &[0x01, 0x01, 0, 0, 0, 0x01, 0x01]);
    }

    #[test]
    fn trigger_bytes() {
        let keys: KeyBitmap = [KeyPos::new(3, 2)].into_iter().collect();
        let config = TriggerConfig {
            mode: TriggerMode::Rapid,
            actuation: Travel::from_raw(16),
            release: Travel::from_raw(16),
            rt_press: Travel::from_raw(8),
            rt_release: Travel::from_raw(7),
        };
        let p = he_set_trigger(&keys, &config).unwrap();
        assert_eq!(&p.0[..7], &[0x01, 0x21, 0, 0, 0, 24, 12]);
        assert_eq!(p[7 + 2], 1 << 3);
        assert_eq!(&p.0[29..33], &[16, 16, 8, 7]);

        let bad = TriggerConfig {
            actuation: Travel::from_raw(37),
            ..config
        };
        assert!(he_set_trigger(&keys, &bad).is_err());
    }

    #[test]
    fn he_queries() {
        assert_eq!(&he_query_key(KeyPos::new(5, 11)).0[5..9], &[24, 5, 5, 11]);
        assert_eq!(
            &he_set_debounce(Debounce::new(0).unwrap()).0[5..9],
            &[24, 10, 1, 0]
        );
        assert_eq!(&he_debounce_query().0[5..9], &[24, 10, 0, 0]);
        assert_eq!(&he_calibrate_all_start().0[5..8], &[24, 8, 0]);
        assert_eq!(&he_calibrate_all_abort().0[5..8], &[24, 8, 1]);
    }

    #[test]
    fn system_never_factory_resets() {
        let times = SleepTimes {
            light: 60,
            keyboard: 1800,
        };
        for p in [sleep_query(), sleep_set(times).unwrap()] {
            assert_eq!(p[6], 0);
        }
        let p = sleep_set(times).unwrap();
        assert_eq!(
            &p.0[..13],
            &[1, 0x14, 1, 0, 0, 11, 0, 0, 0, 0, 60, 0x07, 0x08]
        );
        assert!(
            sleep_set(SleepTimes {
                light: 60,
                keyboard: 120
            })
            .is_err()
        );
        assert!(
            sleep_set(SleepTimes {
                light: 0,
                keyboard: 1800
            })
            .is_err()
        );
    }

    #[test]
    fn keymap_packets() {
        let map = Keymap::default_layer();
        let packets = keymaps(&map, &map);
        assert_eq!(packets.len(), 20);
        let lens: Vec<_> = packets[..10].iter().map(|p| p.len()).collect();
        assert_eq!(lens, [56, 56, 56, 56, 56, 56, 56, 56, 56, 24]);
        assert_eq!(
            &packets[0].0[..10],
            &[1, 0x18, 0, 0, 0, 56, 0x00, 0x29, 0, 0]
        );
        let last = packets.last().unwrap();
        assert_eq!((last[2], last[4]), (2, 9));
    }

    #[test]
    fn macro_packets() {
        let mut slots: [Option<Macro>; MACRO_SLOTS as usize] = Default::default();
        slots[1] = Some(Macro::default());
        let packets = macros(&slots).unwrap();
        assert_eq!(packets.len(), 50);
        let lens: Vec<_> = packets[..5].iter().map(|p| p.len()).collect();
        assert_eq!(lens, [58, 58, 58, 58, 24]);
        let last = packets.last().unwrap();
        assert_eq!((last[2], last[4]), (9, 4));
        // Both unused slots and macros with empty events are made empty by setting offset 8 (b[14]) to 0xFF.
        for slot in [0, 1, 9] {
            let first = &packets[slot * 5];
            assert_eq!((first[2], first[4]), (slot as u8, 0));
            assert_eq!(&first.0[6..16], &[0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0]);
        }
    }

    #[test]
    fn advanced_packets() {
        let packets = advanced_keys(&std::array::from_fn(|_| None)).unwrap();
        assert_eq!(packets.len(), 40);
        assert_eq!(&packets[39].0[..6], &[1, 0x22, 39, 0, 0, 40]);
    }

    #[test]
    fn socd_packets() {
        let [p0, p1] = socd_write(&Default::default());
        assert_eq!(&p0.0[..6], &[1, 0x24, 0, 0, 0, 40]);
        assert_eq!(&p1.0[..6], &[1, 0x24, 0, 0, 1, 40]);
    }
}
