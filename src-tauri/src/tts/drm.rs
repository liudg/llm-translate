//! Edge Read Aloud DRM token (`Sec-MS-GEC` v2) and clock-skew management.
//!
//! Matches the current Microsoft Edge 143 Read Aloud protocol:
//! - Token format: `<16 uppercase hex CS64>-<64 uppercase hex modified SHA-256>`
//! - Version param: `2-143.0.3650.75`
//! - Automatic clock-skew correction from the HTTP 403 `Date` response header.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;

pub const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";
pub const CHROMIUM_FULL_VERSION: &str = "143.0.3650.75";
pub const CHROMIUM_MAJOR_VERSION: &str = "143";
pub const SEC_MS_GEC_VERSION: &str = "2-143.0.3650.75";

/// Seconds between 1601-01-01 (Windows file time epoch) and 1970-01-01 (Unix epoch).
const WIN_EPOCH: i64 = 11_644_473_600;

/// Clock skew offset in milliseconds (`server_time - local_time`).
static CLOCK_SKEW_MS: AtomicI64 = AtomicI64::new(0);

const SHA256_INITIAL: [u32; 8] = [
    0x6A09E667, 0xBB67AE85, 0x3C6EF372, 0xA54FF53A,
    0x510E527F, 0x9B05688C, 0x1F83D9AB, 0x5BE0CD19,
];

const SHA256_STATE: [u32; 8] = [
    0xFD9FBEA2, 0x91243CE7, 0xBE499094, 0xCD2E1E0A,
    0x6C006F49, 0x04BD35AB, 0xFEB0A7E4, 0x03EFFBD2,
];

const SHA256_SCHEDULE: [u32; 14] = [
    0xEB24BF9E, 0x1558691F, 0xA0C09EE2, 0x0CFA32BA,
    0xA622EE8C, 0xD1DD9491, 0xC0DD25A0, 0xF9E70AB6,
    0xF0C31573, 0x7058CE4E, 0x7188E606, 0x00E5AB33,
    0xD258B060, 0x72766564,
];

const SHA256_ROUND_CONSTANTS: [u32; 64] = [
    0x428A2F98, 0x71374491, 0xB5C0FBCF, 0xE9B5DBA5,
    0x3956C25B, 0x59F111F1, 0x923F82A4, 0xAB1C5ED5,
    0xD807AA98, 0x12835B01, 0x243185BE, 0x550C7DC3,
    0x72BE5D74, 0x80DEB1FE, 0x9BDC06A7, 0xC19BF174,
    0xE49B69C1, 0xEFBE4786, 0x0FC19DC6, 0x240CA1CC,
    0x2DE92C6F, 0x4A7484AA, 0x5CB0A9DC, 0x76F988DA,
    0x983E5152, 0xA831C66D, 0xB00327C8, 0xBF597FC7,
    0xC6E00BF3, 0xD5A79147, 0x06CA6351, 0x14292967,
    0x27B70A85, 0x2E1B2138, 0x4D2C6DFC, 0x53380D13,
    0x650A7354, 0x766A0ABB, 0x81C2C92E, 0x92722C85,
    0xA2BFE8A1, 0xA81A664B, 0xC24B8B70, 0xC76C51A3,
    0xD192E819, 0xD6990624, 0xF40E3585, 0x106AA070,
    0x19A4C116, 0x1E376C08, 0x2748774C, 0x34B0BCB5,
    0x391C0CB3, 0x4ED8AA4A, 0x5B9CCA4F, 0x682E6FF3,
    0x748F82EE, 0x78A5636F, 0x84C87814, 0x8CC70208,
    0x90BEFFFA, 0xA4506CEB, 0xBEF9A3F7, 0xC67178F2,
];

const SHA256_ADJUSTMENT: u32 = 0xC9DC06A7;
const SHA256_SIGMA0_ADJUSTMENT: u32 = 0x059C0B80;
const SHA256_SIGMA1_ADJUSTMENT: u32 = 0x000B92C0;

const CS64_COEFFICIENTS: [(u32, u32); 14] = [
    (0xE82E91B1, 0xB32700D0),
    (0x44DE24C6, 0x88DFF51D),
    (0x3BEB65BC, 0x29514038),
    (0xC89C5933, 0xB328CBDF),
    (0x8D1EC211, 0xD405B4D5),
    (0x314EF5D5, 0x26E55F3C),
    (0xE92491B1, 0x607801C6),
    (0xF0A5B9D5, 0x82B2C1FB),
    (0x06064CF3, 0xFA51A3DA),
    (0xDA79FA60, 0x207CC464),
    (0xBED20560, 0x3C3C74F3),
    (0x89F9C9F8, 0xD2D14B61),
    (0x35FA38C6, 0xBB04A2C1),
    (0xCB49E3C6, 0xC5626167),
];
const CS64_FACTOR: u32 = 0x82893933;
const CS64_ADJUSTMENT0: u32 = 0xF2F90B1A;
const CS64_ADJUSTMENT1: u32 = 0x0B1A0000;

#[inline]
fn rol16(v: u32) -> u32 {
    v.rotate_left(16)
}

#[inline]
fn sigma0(v: u32) -> u32 {
    v.rotate_right(7) ^ v.rotate_right(18) ^ (v >> 3)
}

#[inline]
fn sigma1(v: u32) -> u32 {
    v.rotate_right(17) ^ v.rotate_right(19) ^ (v >> 10)
}

fn expand_sha256_schedule(words: &mut [u32; 64], first_word: usize) {
    for index in first_word..64 {
        let s1 = sigma1(words[index - 2]);
        let s0 = sigma0(words[index - 15]);
        words[index] = s1
            .wrapping_add(words[index - 7])
            .wrapping_add(s0)
            .wrapping_add(words[index - 16]);
    }
}

fn sha256_rounds(state: [u32; 8], words: &[u32; 64], first_round: usize) -> [u32; 8] {
    let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = state;
    for index in first_round..64 {
        let k = if first_round == 14 && index == 14 {
            SHA256_ADJUSTMENT
        } else {
            SHA256_ROUND_CONSTANTS[index]
        };
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = g ^ (e & (f ^ g));
        let t1 = h
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(k)
            .wrapping_add(words[index]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) | (c & (a | b));
        let t2 = s0.wrapping_add(maj);

        h = g;
        g = f;
        f = e;
        e = d.wrapping_add(t1);
        d = c;
        c = b;
        b = a;
        a = t1.wrapping_add(t2);
    }
    [a, b, c, d, e, f, g, h]
}

fn sha256_init(prefix: &[u8]) -> [u32; 8] {
    let mut words = [0u32; 64];
    words[14] = u32::from_be_bytes([0, prefix[0], prefix[1], prefix[2]]);
    words[15] = u32::from_be_bytes([prefix[3], prefix[4], prefix[5], prefix[6]]);

    for index in 16..30 {
        let mut value = SHA256_SCHEDULE[index - 16];
        if index - 2 >= 14 {
            let s1 = sigma1(words[index - 2]);
            value = value.wrapping_add(if index == 16 { s1 ^ SHA256_SIGMA1_ADJUSTMENT } else { s1 });
        }
        if index - 7 >= 14 {
            value = value.wrapping_add(words[index - 7]);
        }
        if index - 15 >= 14 {
            let s0 = sigma0(words[index - 15]);
            value = value.wrapping_add(if index == 29 { s0 ^ SHA256_SIGMA0_ADJUSTMENT } else { s0 });
        }
        words[index] = value;
    }

    words[30] = sigma1(words[28])
        .wrapping_add(words[23])
        .wrapping_add(sigma0(words[15]))
        .wrapping_add(words[14])
        .wrapping_add(SHA256_ADJUSTMENT)
        .wrapping_sub(SHA256_ROUND_CONSTANTS[14]);

    expand_sha256_schedule(&mut words, 31);
    let working = sha256_rounds(SHA256_STATE, &words, 14);
    std::array::from_fn(|i| SHA256_INITIAL[i].wrapping_add(working[i]))
}

fn sha256_digest(data: &[u8]) -> [u8; 32] {
    debug_assert!(data.len() >= 7);
    let length = 57 + data.len();
    let pad_zeros = (55isize - (length as isize)).rem_euclid(64) as usize;

    let mut padded = Vec::with_capacity(data.len() - 7 + 1 + pad_zeros + 8);
    padded.extend_from_slice(&data[7..]);
    padded.push(0x80);
    padded.resize(padded.len() + pad_zeros, 0);
    padded.extend_from_slice(&((length as u64) * 8).to_be_bytes());

    let mut state = sha256_init(&data[..7]);
    for chunk in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for i in 0..16 {
            words[i] = u32::from_be_bytes(chunk[i * 4..i * 4 + 4].try_into().unwrap());
        }
        expand_sha256_schedule(&mut words, 16);
        let working = sha256_rounds(state, &words, 0);
        for i in 0..8 {
            state[i] = state[i].wrapping_add(working[i]);
        }
    }

    let mut out = [0u8; 32];
    for (i, w) in state.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
    }
    out
}

fn words_le4_even(data: &[u8]) -> Vec<u32> {
    let rem = data.len() % 4;
    let pad = if rem == 0 { 0 } else { 4 - rem };
    let total = data.len() + pad;
    let mut words = Vec::with_capacity(total / 4 + 1);
    for chunk in data.chunks(4) {
        let mut buf = [0u8; 4];
        buf[..chunk.len()].copy_from_slice(chunk);
        words.push(u32::from_le_bytes(buf));
    }
    if words.len() % 2 == 1 {
        words.push(0);
    }
    words
}

fn cs64_digest(data: &[u8], digest: &[u8; 32]) -> (u32, u32) {
    let key_a = u32::from_le_bytes(digest[0..4].try_into().unwrap()) | 1;
    let key_b = u32::from_le_bytes(digest[4..8].try_into().unwrap()) | 1;
    let factors = [key_a.wrapping_mul(CS64_FACTOR), key_b.wrapping_mul(CS64_FACTOR)];

    let mut terms = Vec::with_capacity(14 + (data.len() + 8) / 4);
    let mut offsets = Vec::with_capacity(14 + (data.len() + 8) / 4);
    for (index, &(coeff, offset)) in CS64_COEFFICIENTS.iter().enumerate() {
        terms.push(coeff.wrapping_mul(factors[index % 2]));
        offsets.push(offset);
    }

    let mut prefixed = Vec::with_capacity(1 + data.len());
    prefixed.push(0);
    prefixed.extend_from_slice(data);

    for (index, word) in words_le4_even(&prefixed).into_iter().enumerate() {
        let mut term = word.wrapping_mul(if index % 2 == 0 { key_a } else { key_b });
        let mut offset = rol16(word).wrapping_mul(if index % 2 == 0 { 0xEF0569FB } else { 0xC31713DB });
        if index == 0 {
            term = term.wrapping_add(CS64_ADJUSTMENT0.wrapping_mul(factors[0]));
            offset = offset.wrapping_add(CS64_ADJUSTMENT1);
        }
        terms.push(term);
        offsets.push(offset);
    }

    let (mut swap_acc, mut swap_last, mut state, mut acc) = (0u32, 0u32, 0u32, 0u32);
    for index in (0..terms.len()).step_by(2) {
        let (x, y) = (terms[index], terms[index + 1]);

        let t = rol16(offsets[index].wrapping_add(x));
        let q = t.wrapping_mul(0x76F50F34);
        swap_acc = swap_acc.wrapping_add(rol16(q).wrapping_mul(0xC3EFEA97).wrapping_add(q));

        let t = rol16(offsets[index + 1].wrapping_add(y));
        let q = t.wrapping_mul(0x1881CE3C);
        swap_last = rol16(q).wrapping_mul(0x35BD1EC9).wrapping_add(q);
        swap_acc = swap_acc.wrapping_add(swap_last);

        state = state.wrapping_mul(key_a).wrapping_add(x);
        state = rol16(state).wrapping_mul(0xCF98B111);
        state = rol16(state).wrapping_mul(0x87085B9F);
        state = rol16(state).wrapping_mul(0x12CEB96D);
        state = rol16(state).wrapping_mul(0x257E1D83);
        acc = acc.wrapping_add(state);

        let t = state.wrapping_mul(key_b).wrapping_add(y);
        let t = rol16(t).wrapping_mul(0xA27416F5);
        let t = rol16(t).wrapping_mul(0xD38396FF);
        state = rol16(rol16(t).wrapping_mul(0x7C932B89)).wrapping_mul(0xBFA49F69);
        acc = acc.wrapping_add(state);
    }

    (swap_last ^ state, swap_acc ^ acc)
}

/// Generate a v2 `Sec-MS-GEC` token for a given Unix timestamp in seconds.
pub fn generate_sec_ms_gec_v2_at(unix_secs: f64) -> String {
    let win_secs = (unix_secs + WIN_EPOCH as f64).floor() as i64;
    let rounded = win_secs.div_euclid(300) * 300;
    let ticks_100ns = rounded as u64 * 10_000_000;
    let input = format!("{ticks_100ns}{TRUSTED_CLIENT_TOKEN}");

    let digest = sha256_digest(input.as_bytes());
    let (w0, w1) = cs64_digest(input.as_bytes(), &digest);
    let mut cs64_bytes = [0u8; 8];
    cs64_bytes[..4].copy_from_slice(&w0.to_le_bytes());
    cs64_bytes[4..].copy_from_slice(&w1.to_le_bytes());

    format!("{}-{}", hex::encode_upper(cs64_bytes), hex::encode_upper(digest))
}

/// Current Unix timestamp adjusted for any recorded server clock skew.
pub fn adjusted_unix_secs() -> f64 {
    let local_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let skew_ms = CLOCK_SKEW_MS.load(Ordering::Relaxed);
    (local_ms + skew_ms) as f64 / 1000.0
}

/// Generate the current v2 `Sec-MS-GEC` token using the skew-adjusted clock.
pub fn generate_sec_ms_gec_v2() -> String {
    generate_sec_ms_gec_v2_at(adjusted_unix_secs())
}

/// Parse an HTTP `Date` header (RFC 2822 / RFC 2616) into a Unix timestamp in seconds.
pub fn parse_http_date(header: &str) -> Option<i64> {
    DateTime::parse_from_rfc2822(header.trim()).ok().map(|dt| dt.timestamp())
}

/// Adjust global clock skew from a server `Date` header. Returns `true` if adjusted.
pub fn adjust_clock_skew_from_date(date_header: &str) -> bool {
    let Some(server_secs) = parse_http_date(date_header) else {
        return false;
    };
    let local_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    let new_skew_ms = server_secs * 1000 - local_ms;
    CLOCK_SKEW_MS.store(new_skew_ms, Ordering::Relaxed);
    tracing::info!("tts: adjusted clock skew to {:.1}s from server Date header", new_skew_ms as f64 / 1000.0);
    true
}

/// Generate a random 16-byte uppercase hex MUID cookie value.
pub fn generate_muid() -> String {
    uuid::Uuid::new_v4().simple().to_string().to_uppercase()
}

/// Generate a random 32-char lowercase hex request / connection ID.
pub fn connect_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rfc2616_date() {
        assert_eq!(parse_http_date("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(parse_http_date("Wed, 25 Nov 2026 09:20:00 GMT"), Some(1_795_598_400));
        assert_eq!(parse_http_date("invalid"), None);
    }

    #[test]
    fn sec_ms_gec_v2_format_and_window() {
        let t1 = generate_sec_ms_gec_v2_at(1_700_000_000.0);
        let t2 = generate_sec_ms_gec_v2_at(1_700_000_050.9);
        assert_eq!(t1, t2, "timestamps inside the same 5-minute window must match");
        let (cs64, sha) = t1.split_once('-').expect("v2 format <16 hex>-<64 hex>");
        assert_eq!(cs64.len(), 16);
        assert_eq!(sha.len(), 64);
        assert!(cs64.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
        assert!(sha.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_lowercase()));
    }
}
