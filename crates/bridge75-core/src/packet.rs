use std::fmt;

/// Packet length including the Report ID.
pub const PACKET_LEN: usize = 64;
/// Report ID of the configuration interface.
pub const REPORT_ID: u8 = 0x01;
/// Start of the data section (`b[6]`).
pub const DATA_OFFSET: usize = 6;
/// Maximum length of the data section.
pub const MAX_DATA_LEN: usize = PACKET_LEN - DATA_OFFSET;

/// A 64-byte packet. Indices are absolute positions including the Report ID (`b[0]` = Report ID).
///
/// ```text
/// b[0] = 0x01 (Report ID)   b[1] = cmd   b[2..=4] = parameters   b[5] = data length   b[6..] = data
/// ```
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Packet(pub [u8; PACKET_LEN]);

impl Packet {
    pub fn new(cmd: u8) -> Self {
        let mut bytes = [0; PACKET_LEN];
        bytes[0] = REPORT_ID;
        bytes[1] = cmd;
        Self(bytes)
    }

    /// Builds from a received report. Accepts both 64 bytes including the Report ID and 63 bytes or fewer without it.
    pub fn from_report(report: &[u8]) -> Option<Self> {
        let mut bytes = [0; PACKET_LEN];
        match report.len() {
            PACKET_LEN if report[0] == REPORT_ID => bytes.copy_from_slice(report),
            1..PACKET_LEN => {
                bytes[0] = REPORT_ID;
                bytes[1..=report.len()].copy_from_slice(report);
            }
            _ => return None,
        }
        Some(Self(bytes))
    }

    pub fn cmd(&self) -> u8 {
        self.0[1]
    }

    /// Data length in `b[5]`.
    pub fn len(&self) -> usize {
        self.0[5] as usize
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The part of `b[6..]` covered by the length in `b[5]` (at most 58 bytes).
    pub fn data(&self) -> &[u8] {
        &self.0[DATA_OFFSET..DATA_OFFSET + self.len().min(MAX_DATA_LEN)]
    }

    pub fn with_params(mut self, p2: u8, p3: u8, p4: u8) -> Self {
        self.0[2] = p2;
        self.0[3] = p3;
        self.0[4] = p4;
        self
    }

    /// Puts the length in `b[5]` and writes the data to `b[6..]`.
    pub fn with_data(mut self, data: &[u8]) -> Self {
        assert!(data.len() <= MAX_DATA_LEN, "data is too long");
        self.0[5] = data.len() as u8;
        self.0[DATA_OFFSET..DATA_OFFSET + data.len()].copy_from_slice(data);
        self
    }

    /// Writes bytes from absolute position `at` without changing the length in `b[5]`.
    pub fn put(&mut self, at: usize, bytes: &[u8]) {
        self.0[at..at + bytes.len()].copy_from_slice(bytes);
    }

    /// BE16 at absolute position `at`.
    pub fn be16(&self, at: usize) -> u16 {
        u16::from_be_bytes([self.0[at], self.0[at + 1]])
    }

    /// Reads ASCII from absolute position `at` up to a NUL or the end.
    pub fn ascii(&self, at: usize, max_len: usize) -> String {
        let end = (at + max_len).min(PACKET_LEN);
        let bytes = &self.0[at..end];
        let bytes = bytes.split(|&b| b == 0).next().unwrap_or_default();
        String::from_utf8_lossy(bytes).trim().to_string()
    }
}

impl std::ops::Index<usize> for Packet {
    type Output = u8;

    fn index(&self, index: usize) -> &u8 {
        &self.0[index]
    }
}

impl std::ops::IndexMut<usize> for Packet {
    fn index_mut(&mut self, index: usize) -> &mut u8 {
        &mut self.0[index]
    }
}

/// Hex dump with trailing zeros omitted.
impl fmt::Debug for Packet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let end = self
            .0
            .iter()
            .rposition(|&b| b != 0)
            .map_or(2, |i| (i + 1).max(2));
        for (i, b) in self.0[..end].iter().enumerate() {
            if i > 0 {
                f.write_str(" ")?;
            }
            write!(f, "{b:02X}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_report_accepts_both_lengths() {
        let mut with_id = [0u8; 64];
        with_id[0] = 1;
        with_id[1] = 0x0D;
        let p = Packet::from_report(&with_id).unwrap();
        assert_eq!(p.cmd(), 0x0D);

        let without_id = [0x21u8, 0, 0, 0, 1, 0];
        let p = Packet::from_report(&without_id).unwrap();
        assert_eq!(p[0], REPORT_ID);
        assert_eq!(p.cmd(), 0x21);
        assert_eq!(p.len(), 1);
    }

    #[test]
    fn debug_trims_trailing_zeros() {
        let p = Packet::new(0x01).with_data(&[1]);
        assert_eq!(format!("{p:?}"), "01 01 00 00 00 01 01");
    }
}
