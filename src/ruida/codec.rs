//! Ruida byte swizzle used on UDP port 50200.
//!
//! Wire bytes are scrambled. Cleartext command bytes are not. Coordinate and
//! power fields are 7-bit encoded, so a cleartext `0xA8` is a cut opcode and
//! not a coordinate fragment.

pub fn scramble_byte(b: u8) -> u8 {
    let hi = b & 0x80;
    let lo = b & 0x01;
    let mut out = b & 0x7E;
    out |= lo << 7;
    out |= hi >> 7;
    out ^= 0x88;
    out.wrapping_add(1)
}

pub fn unscramble_byte(b: u8) -> u8 {
    let mut b = b.wrapping_sub(1);
    b ^= 0x88;
    let hi = (b & 0x80) >> 7;
    let lo = b & 0x01;
    let mut out = b & 0x7E;
    out |= lo << 7;
    out |= hi;
    out
}

pub fn unscramble(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().map(unscramble_byte).collect()
}

pub fn scramble(bytes: &[u8]) -> Vec<u8> {
    bytes.iter().copied().map(scramble_byte).collect()
}

/// Job datagrams are a big-endian sum of the scrambled payload, then that payload.
pub fn encode_job(clear: &[u8]) -> Vec<u8> {
    let scrambled = scramble(clear);
    let sum = scrambled
        .iter()
        .fold(0u16, |acc, byte| acc.wrapping_add(u16::from(*byte)));
    let mut out = Vec::with_capacity(2 + scrambled.len());
    out.push((sum >> 8) as u8);
    out.push((sum & 0xFF) as u8);
    out.extend(scrambled);
    out
}

/// Payload after the checksum, unscrambled. `None` when the datagram is too short
/// or the checksum does not match.
pub fn decode_job(datagram: &[u8]) -> Option<Vec<u8>> {
    if datagram.len() < 3 {
        return None;
    }
    let sum = u16::from_be_bytes([datagram[0], datagram[1]]);
    let scrambled = &datagram[2..];
    let calc = scrambled
        .iter()
        .fold(0u16, |acc, byte| acc.wrapping_add(u16::from(*byte)));
    if calc != sum {
        return None;
    }
    Some(unscramble(scrambled))
}

/// Unscramble the bytes after a 2-byte prefix even when the checksum is wrong.
pub fn decode_job_unchecked(datagram: &[u8]) -> Option<Vec<u8>> {
    if datagram.len() < 3 {
        return None;
    }
    Some(unscramble(&datagram[2..]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swizzle_roundtrip() {
        for b in 0..=255 {
            assert_eq!(unscramble_byte(scramble_byte(b)), b);
        }
    }

    #[test]
    fn known_cut_opcode() {
        // 0xA8 is "cut absolute".
        assert_eq!(unscramble_byte(scramble_byte(0xA8)), 0xA8);
        assert_ne!(scramble_byte(0xA8), 0xA8);
    }

    #[test]
    fn checksum_roundtrip() {
        let packet = encode_job(&[0x88, 0x00, 0x01, 0x02, 0x03, 0x04]);
        let clear = decode_job(&packet).unwrap();
        assert_eq!(clear, vec![0x88, 0x00, 0x01, 0x02, 0x03, 0x04]);
    }
}
