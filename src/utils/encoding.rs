//! Base64 encoding without external dependencies.

/// Encode a byte slice as a standard Base64 string with padding.
#[must_use]
pub(crate) fn encode_base64(bytes: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let triple = (b0 << 16) | (b1 << 8) | b2;
        out.push(CHARS[(triple >> 18 & 0x3F) as usize] as char);
        out.push(CHARS[(triple >> 12 & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(CHARS[(triple >> 6 & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_base64_empty() {
        assert_eq!(encode_base64(&[]), "");
    }

    #[test]
    fn encode_base64_hello() {
        assert_eq!(encode_base64(b"Hello"), "SGVsbG8=");
    }

    #[test]
    fn encode_base64_three_byte_aligned() {
        assert_eq!(encode_base64(b"abc"), "YWJj");
    }

    #[test]
    fn encode_base64_single_byte() {
        assert_eq!(encode_base64(&[1, 2, 3]), "AQID");
    }
}
