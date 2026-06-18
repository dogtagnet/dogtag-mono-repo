//! Small dependency-free helpers.

/// Big-endian bytes -> decimal string (schoolbook), for the modulus-confusion guard.
pub fn be_bytes_to_dec(be: &[u8]) -> String {
    let mut digits: Vec<u8> = vec![0];
    for &byte in be {
        // digits = digits * 256 + byte
        let mut carry = byte as u32;
        for d in digits.iter_mut() {
            let cur = (*d as u32) * 256 + carry;
            *d = (cur % 10) as u8;
            carry = cur / 10;
        }
        while carry > 0 {
            digits.push((carry % 10) as u8);
            carry /= 10;
        }
    }
    digits.iter().rev().map(|d| (b'0' + d) as char).collect()
}

/// Decimal string -> little-endian bytes (schoolbook /256).
pub fn dec_to_le_bytes(s: &str) -> Vec<u8> {
    let mut digits: Vec<u8> = s.bytes().map(|b| b - b'0').collect();
    let mut out = Vec::new();
    while !(digits.len() == 1 && digits[0] == 0) {
        let mut rem = 0u32;
        let mut next = Vec::with_capacity(digits.len());
        for &d in &digits {
            let cur = rem * 10 + d as u32;
            next.push((cur / 256) as u8);
            rem = cur % 256;
        }
        let mut i = 0;
        while i + 1 < next.len() && next[i] == 0 {
            i += 1;
        }
        digits = next[i..].to_vec();
        out.push(rem as u8);
    }
    if out.is_empty() {
        out.push(0);
    }
    out
}
