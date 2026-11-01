//! Parsing of received packets.
//!
//! Responses are asynchronous and queued, and do not always map one-to-one to requests.
//! They are dispatched on `b[1]` (and `b[6]` for 0x21).

use crate::cmd;
use crate::he::{AllCalibration, KeyCalibration, Travel};
use crate::info::{BuildTime, Heartbeat, SleepTimes};
use crate::keymap::KeyEntry;
use crate::matrix::{COLS, KeyBitmap, KeyPos, ROWS};
use crate::packet::Packet;
use crate::socd::{SocdPair, SocdStatus};

#[derive(Clone, PartialEq, Debug)]
pub enum Response {
    Heartbeat(Heartbeat),
    /// First 0x0D packet (`"M484,22,KB,OW,75ALHE,V1.13.10"`).
    DeviceId(String),
    /// Second 0x0D packet.
    BuildTime(BuildTime),
    SerialNumber(String),
    /// One row of 0x10.
    DefaultKeymapRow {
        row: u8,
        usages: [u8; COLS],
    },
    FixedFnMask(KeyBitmap),
    SleepTimes(SleepTimes),
    He(HeResponse),
    /// One row of 0x23 (12-bit raw ADC values; 0 for cells without a key).
    RawSensorRow {
        row: u8,
        values: [u16; COLS],
    },
    /// One packet of the 0x24 query (pairs 10 * page .. 10 * page + 9).
    /// The FW returns byte 0 with `& 7` applied, so the enabled flag (bit7) is not included.
    SocdPairs {
        page: u8,
        pairs: [SocdPair; 10],
    },
    /// 0x24 status query. This is the only way to read the enabled flags.
    SocdStatus(SocdStatus),
    /// 0x20: press / release of a type 0x16 key (only while the driver is connected).
    CustomEvent {
        entry: KeyEntry,
        pressed: bool,
    },
    /// Write response to 0x18 / 0x19 / 0x22 / 0x14 / 0x24. The value is the cmd.
    Ack(u8),
    Other(Packet),
}

/// Responses and notifications for 0x21.
#[derive(Clone, PartialEq, Debug)]
pub enum HeResponse {
    /// Response to sub0 / 1 / 2 / 3 / 6 / 12 / 13 (`b[6]=0`).
    Ack,
    /// sub1 notification: the travel of a key registered with sub2 changed.
    Travel {
        pos: KeyPos,
        travel: Travel,
    },
    /// sub4.
    MaxTravel(u8),
    /// sub5: mode, actuation point, release point, RT press, RT release (no matrix position included).
    KeyTrigger([u8; 5]),
    /// sub7.
    CalibrateKey(KeyCalibration),
    /// sub8: status and calibrated keys. Arrives on every progress update.
    CalibrateAll {
        state: AllCalibration,
        calibrated: KeyBitmap,
    },
    /// sub9 (kHz).
    PollingRate(u8),
    /// sub10.
    Debounce(u8),
    Other(u8),
}

fn bitmap_at(p: &Packet, at: usize) -> KeyBitmap {
    KeyBitmap(p.0[at..at + COLS].try_into().unwrap())
}

impl Response {
    pub fn parse(p: &Packet) -> Response {
        match p.cmd() {
            cmd::HEARTBEAT => Response::Heartbeat(Heartbeat {
                win_lock: p[6] != 0,
                calibrated: p[20] != 0,
            }),
            cmd::DEVICE_INFO => match p[4] {
                0 => Response::DeviceId(p.ascii(6, p.len())),
                1 => {
                    let text = p.ascii(6, 58);
                    let (date, time) = text.split_once(',').unwrap_or((&text, ""));
                    Response::BuildTime(BuildTime {
                        date: date.trim().into(),
                        time: time.trim().into(),
                    })
                }
                _ => Response::Other(*p),
            },
            cmd::SERIAL_NUMBER => Response::SerialNumber(p.ascii(6, 32)),
            cmd::DEFAULT_KEYMAP => {
                // b[3..4] is the next row number (1..5, 0xFFFF for the last).
                let next = p.be16(3);
                let row = if next == 0xFFFF {
                    ROWS as u8 - 1
                } else {
                    next.saturating_sub(1) as u8
                };
                Response::DefaultKeymapRow {
                    row,
                    usages: p.0[6..6 + COLS].try_into().unwrap(),
                }
            }
            cmd::FIXED_FN_MASK => Response::FixedFnMask(bitmap_at(p, 6)),
            cmd::SYSTEM if p[2] == 2 => Response::SleepTimes(SleepTimes {
                light: p.be16(9),
                keyboard: p.be16(11),
            }),
            cmd::HE => Response::He(parse_he(p)),
            cmd::RAW_SENSOR => {
                let row = p.be16(3).saturating_sub(1) as u8;
                Response::RawSensorRow {
                    row,
                    values: std::array::from_fn(|col| p.be16(6 + 2 * col)),
                }
            }
            cmd::SOCD if p[2] == 1 => Response::SocdPairs {
                page: p[4],
                pairs: std::array::from_fn(|i| {
                    SocdPair::decode(p.0[6 + 4 * i..10 + 4 * i].try_into().unwrap())
                }),
            },
            cmd::SOCD if p[2] == 2 => Response::SocdStatus(SocdStatus {
                global: p[6] != 0,
                enabled_mask: u32::from_be_bytes([0, p[7], p[8], p[9]]),
            }),
            cmd::CUSTOM_EVENT => Response::CustomEvent {
                entry: KeyEntry(p.0[6..10].try_into().unwrap()),
                pressed: p[10] != 0,
            },
            c @ (cmd::KEYMAP | cmd::MACRO | cmd::ADVANCED_KEY | cmd::SYSTEM | cmd::SOCD) => {
                Response::Ack(c)
            }
            _ => Response::Other(*p),
        }
    }
}

fn parse_he(p: &Packet) -> HeResponse {
    match p[6] {
        0 => HeResponse::Ack,
        1 if (p[7] as usize) < ROWS && (p[8] as usize) < COLS => HeResponse::Travel {
            pos: KeyPos::new(p[7], p[8]),
            travel: Travel::from_raw(p[9]),
        },
        4 => HeResponse::MaxTravel(p[7]),
        5 => HeResponse::KeyTrigger(p.0[7..12].try_into().unwrap()),
        7 => HeResponse::CalibrateKey(p[7].into()),
        8 => HeResponse::CalibrateAll {
            state: p[7].into(),
            calibrated: bitmap_at(p, 8),
        },
        9 => HeResponse::PollingRate(p[7]),
        10 => HeResponse::Debounce(p[7]),
        sub => HeResponse::Other(sub),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(bytes: &[u8]) -> Packet {
        let mut p = Packet::new(0);
        p.put(0, bytes);
        p
    }

    #[test]
    fn device_info() {
        let mut p = Packet::new(0x0D).with_data(b"M484,22,KB,OW,75ALHE,V1.13.10");
        p[5] = 50;
        assert_eq!(
            Response::parse(&p),
            Response::DeviceId("M484,22,KB,OW,75ALHE,V1.13.10".into())
        );

        let p = Packet::new(0x0D)
            .with_params(0, 0, 1)
            .with_data(b"Nov 19 2024,10:36:50");
        assert_eq!(
            Response::parse(&p),
            Response::BuildTime(BuildTime {
                date: "Nov 19 2024".into(),
                time: "10:36:50".into()
            })
        );
    }

    #[test]
    fn he_responses() {
        let r = |b: &[u8]| match Response::parse(&packet(b)) {
            Response::He(he) => he,
            other => panic!("{other:?}"),
        };
        assert_eq!(r(&[1, 0x21, 0, 0, 0, 1, 0]), HeResponse::Ack);
        assert_eq!(r(&[1, 0x21, 0, 0, 0, 2, 4, 36]), HeResponse::MaxTravel(36));
        assert_eq!(
            r(&[1, 0x21, 0, 0, 0, 4, 5, 12, 16, 16, 8, 8]),
            HeResponse::KeyTrigger([12, 16, 16, 8, 8])
        );
        assert_eq!(
            r(&[1, 0x21, 0, 0, 0, 3, 1, 3, 2, 20]),
            HeResponse::Travel {
                pos: KeyPos::new(3, 2),
                travel: Travel::from_raw(20)
            }
        );
        assert_eq!(r(&[1, 0x21, 0, 0, 0, 2, 10, 3]), HeResponse::Debounce(3));
        assert_eq!(
            r(&[1, 0x21, 0, 0, 0, 2, 7, 2]),
            HeResponse::CalibrateKey(KeyCalibration::TimedOut)
        );
    }

    #[test]
    fn heartbeat() {
        let mut p = packet(&[1, 0x01, 0, 0, 0, 6, 0, 2, 1, 0, 100, 0, 0, 0, 0, 1]);
        p[20] = 1;
        assert_eq!(
            Response::parse(&p),
            Response::Heartbeat(Heartbeat {
                win_lock: false,
                calibrated: true
            })
        );
    }

    #[test]
    fn sleep_and_ack() {
        let p = packet(&[1, 0x14, 2, 0, 0, 0, 0, 0, 0, 0, 60, 0x07, 0x08]);
        assert_eq!(
            Response::parse(&p),
            Response::SleepTimes(SleepTimes {
                light: 60,
                keyboard: 1800
            })
        );
        assert_eq!(Response::parse(&packet(&[1, 0x14, 1])), Response::Ack(0x14));
        assert_eq!(Response::parse(&packet(&[1, 0x18])), Response::Ack(0x18));
    }

    #[test]
    fn socd_responses() {
        let mut p = packet(&[1, 0x24, 1, 0, 1, 40]);
        p.put(6, &[0x01, 3, 0x1A, 0x16]);
        match Response::parse(&p) {
            Response::SocdPairs { page, pairs } => {
                assert_eq!(page, 1);
                assert_eq!(pairs[0].mode, crate::socd::SocdMode::Global);
                assert_eq!(pairs[0].encode(), [0x01, 3, 0x1A, 0x16]);
                assert!(pairs[1].is_empty());
            }
            other => panic!("{other:?}"),
        }
        // b[7..=9] is BE24. bit(19 - i) is pair i.
        let p = packet(&[1, 0x24, 2, 0, 0, 4, 1, 0x08, 0x00, 0x01, 0xFA]);
        match Response::parse(&p) {
            Response::SocdStatus(s) => {
                assert!(s.global);
                assert!(s.is_enabled(0));
                assert!(s.is_enabled(19));
                assert!(!s.is_enabled(1));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn raw_sensor_row() {
        let mut p = packet(&[1, 0x23, 0, 0, 6, 44]);
        p.put(6, &2001u16.to_be_bytes());
        match Response::parse(&p) {
            Response::RawSensorRow { row, values } => {
                assert_eq!(row, 5);
                assert_eq!(values[0], 2001);
            }
            other => panic!("{other:?}"),
        }
    }
}
