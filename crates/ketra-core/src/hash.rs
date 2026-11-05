//! Self-implemented hash functions: SHA-256, MD5, XXH64 and CRC32C.
//!
//! All four are implemented from their public specifications (see
//! `docs/ALGORITHMS.md` for the references): FIPS 180-4 for SHA-256,
//! RFC 1321 for MD5, the XXH64 reference specification by Y. Collet and
//! the Castagnoli polynomial reflected table for CRC32C.

use std::fmt::Write as _;

/// Streaming hasher interface shared by all algorithms.
pub trait Hasher {
    /// Feed more bytes into the state.
    fn update(&mut self, data: &[u8]);
    /// Consume the state and return the lowercase hex digest.
    fn finish_hex(self: Box<Self>) -> String;
    /// Digest length in bytes.
    fn len_bytes(&self) -> usize;
}

// ---------------------------------------------------------------------------
// SHA-256 (FIPS 180-4)
// ---------------------------------------------------------------------------

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 streaming hasher (FIPS 180-4).
#[derive(Clone)]
pub struct Sha256 {
    h: [u32; 8],
    buf: [u8; 64],
    buflen: usize,
    total: u64,
}

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    /// Create a new hasher with the FIPS initial state.
    pub fn new() -> Self {
        Sha256 {
            h: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buf: [0u8; 64],
            buflen: 0,
            total: 0,
        }
    }

    fn compress(&mut self, block: &[u8]) {
        debug_assert_eq!(block.len(), 64);
        let mut w = [0u32; 64];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[4 * i], block[4 * i + 1], block[4 * i + 2], block[4 * i + 3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = self.h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(SHA256_K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
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
        self.h[0] = self.h[0].wrapping_add(a);
        self.h[1] = self.h[1].wrapping_add(b);
        self.h[2] = self.h[2].wrapping_add(c);
        self.h[3] = self.h[3].wrapping_add(d);
        self.h[4] = self.h[4].wrapping_add(e);
        self.h[5] = self.h[5].wrapping_add(f);
        self.h[6] = self.h[6].wrapping_add(g);
        self.h[7] = self.h[7].wrapping_add(h);
    }

    fn digest_bytes(mut self) -> [u8; 32] {
        let bitlen = self.total.wrapping_mul(8);
        // Padding: 0x80, zeros, 8-byte BE bit length. The padding is fed
        // through the same buffering as `update`, which flushes any pending
        // partial block first; `pending + pad_len` is a multiple of 64.
        let rem = 64 - self.buflen; // free space in the pending block
        let pad_len = if rem < 9 { rem + 64 } else { rem };
        let mut tail = vec![0u8; pad_len];
        tail[0] = 0x80;
        tail[pad_len - 8..].copy_from_slice(&bitlen.to_be_bytes());
        self.update(&tail);
        debug_assert_eq!(self.buflen, 0);
        let mut out = [0u8; 32];
        for (i, w) in self.h.iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_be_bytes());
        }
        out
    }
}

impl Hasher for Sha256 {
    fn update(&mut self, data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        let mut idx = 0;
        if self.buflen > 0 {
            let need = 64 - self.buflen;
            let take = need.min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            idx = take;
            if self.buflen == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buflen = 0;
            }
        }
        while idx + 64 <= data.len() {
            self.compress(&data[idx..idx + 64]);
            idx += 64;
        }
        if idx < data.len() {
            let rest = data.len() - idx;
            self.buf[..rest].copy_from_slice(&data[idx..]);
            self.buflen = rest;
        }
    }

    fn finish_hex(self: Box<Self>) -> String {
        to_hex(&(*self).digest_bytes())
    }

    fn len_bytes(&self) -> usize {
        32
    }
}

// ---------------------------------------------------------------------------
// MD5 (RFC 1321)
// ---------------------------------------------------------------------------

const MD5_S: [u32; 64] = [
    7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9,
    14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15,
    21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
];

const MD5_K: [u32; 64] = [
    0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613, 0xfd469501,
    0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193, 0xa679438e, 0x49b40821,
    0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d, 0x02441453, 0xd8a1e681, 0xe7d3fbc8,
    0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed, 0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a,
    0xfffa3942, 0x8771f681, 0x6d9d6122, 0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70,
    0x289b7ec6, 0xeaa127fa, 0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665,
    0xf4292244, 0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
    0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb, 0xeb86d391,
];

/// MD5 streaming hasher (RFC 1321). Provided for legacy checksum lists only.
#[derive(Clone)]
pub struct Md5 {
    a: u32,
    b: u32,
    c: u32,
    d: u32,
    buf: [u8; 64],
    buflen: usize,
    total: u64,
}

impl Default for Md5 {
    fn default() -> Self {
        Self::new()
    }
}

impl Md5 {
    /// Create a new hasher with the RFC 1321 initial state.
    pub fn new() -> Self {
        Md5 { a: 0x67452301, b: 0xefcdab89, c: 0x98badcfe, d: 0x10325476, buf: [0; 64], buflen: 0, total: 0 }
    }

    fn compress(&mut self, block: &[u8]) {
        let mut m = [0u32; 16];
        for i in 0..16 {
            m[i] = u32::from_le_bytes([
                block[4 * i],
                block[4 * i + 1],
                block[4 * i + 2],
                block[4 * i + 3],
            ]);
        }
        let (mut a, mut b, mut c, mut d) = (self.a, self.b, self.c, self.d);
        for i in 0..64 {
            let (f, g) = match i / 16 {
                0 => ((b & c) | ((!b) & d), i),
                1 => ((d & b) | ((!d) & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | (!d)), (7 * i) % 16),
            };
            let tmp = d;
            d = c;
            c = b;
            let x = a
                .wrapping_add(f)
                .wrapping_add(MD5_K[i])
                .wrapping_add(m[g]);
            b = b.wrapping_add(x.rotate_left(MD5_S[i]));
            a = tmp;
        }
        self.a = self.a.wrapping_add(a);
        self.b = self.b.wrapping_add(b);
        self.c = self.c.wrapping_add(c);
        self.d = self.d.wrapping_add(d);
    }

    fn digest_bytes(mut self) -> [u8; 16] {
        let bitlen = (self.total as u64).wrapping_mul(8);
        // Same absorb-through-update padding as SHA-256 (little-endian length).
        let rem = 64 - self.buflen;
        let pad_len = if rem < 9 { rem + 64 } else { rem };
        let mut tail = vec![0u8; pad_len];
        tail[0] = 0x80;
        tail[pad_len - 8..].copy_from_slice(&bitlen.to_le_bytes());
        self.update(&tail);
        debug_assert_eq!(self.buflen, 0);
        let mut out = [0u8; 16];
        for (i, w) in [self.a, self.b, self.c, self.d].iter().enumerate() {
            out[i * 4..i * 4 + 4].copy_from_slice(&w.to_le_bytes());
        }
        out
    }
}

impl Hasher for Md5 {
    fn update(&mut self, data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        let mut idx = 0;
        if self.buflen > 0 {
            let need = 64 - self.buflen;
            let take = need.min(data.len());
            self.buf[self.buflen..self.buflen + take].copy_from_slice(&data[..take]);
            self.buflen += take;
            idx = take;
            if self.buflen == 64 {
                let block = self.buf;
                self.compress(&block);
                self.buflen = 0;
            }
        }
        while idx + 64 <= data.len() {
            self.compress(&data[idx..idx + 64]);
            idx += 64;
        }
        if idx < data.len() {
            let rest = data.len() - idx;
            self.buf[..rest].copy_from_slice(&data[idx..]);
            self.buflen = rest;
        }
    }

    fn finish_hex(self: Box<Self>) -> String {
        to_hex(&(*self).digest_bytes())
    }

    fn len_bytes(&self) -> usize {
        16
    }
}

// ---------------------------------------------------------------------------
// XXH64
// ---------------------------------------------------------------------------

const P1: u64 = 0x9E3779B185EBCA87;
const P2: u64 = 0xC2B2AE3D27D4EB4F;
const P3: u64 = 0x165667B19E3779F9;
const P4: u64 = 0x85EBCA77C2B2AE63;
const P5: u64 = 0x27D4EB2F165667C5;

fn xxh_round(acc: u64, input: u64) -> u64 {
    (acc.wrapping_add(input.wrapping_mul(P2)))
        .rotate_left(31)
        .wrapping_mul(P1)
}

fn xxh_merge(h: u64, v: u64) -> u64 {
    (h ^ xxh_round(0, v)).wrapping_mul(P1).wrapping_add(P4)
}

/// XXH64 non-cryptographic 64-bit hash (xxHash specification by Y. Collet).
pub fn xxh64(data: &[u8], seed: u64) -> u64 {
    let n = data.len();
    let mut i = 0usize;
    let mut h: u64;
    if n >= 32 {
        let mut v1 = seed.wrapping_add(P1).wrapping_add(P2);
        let mut v2 = seed.wrapping_add(P2);
        let mut v3 = seed;
        let mut v4 = seed.wrapping_sub(P1);
        let limit = n - 32;
        while i <= limit {
            v1 = xxh_round(v1, u64_le(&data[i..]));
            v2 = xxh_round(v2, u64_le(&data[i + 8..]));
            v3 = xxh_round(v3, u64_le(&data[i + 16..]));
            v4 = xxh_round(v4, u64_le(&data[i + 24..]));
            i += 32;
        }
        h = v1
            .rotate_left(1)
            .wrapping_add(v2.rotate_left(7))
            .wrapping_add(v3.rotate_left(12))
            .wrapping_add(v4.rotate_left(18));
        h = xxh_merge(h, v1);
        h = xxh_merge(h, v2);
        h = xxh_merge(h, v3);
        h = xxh_merge(h, v4);
    } else {
        h = seed.wrapping_add(P5);
    }
    h = h.wrapping_add(n as u64);
    while i + 8 <= n {
        let k = xxh_round(0, u64_le(&data[i..]));
        h = (h ^ k).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        i += 8;
    }
    if i + 4 <= n {
        h = (h ^ (u32_le(&data[i..]) as u64).wrapping_mul(P1))
            .rotate_left(23)
            .wrapping_mul(P2)
            .wrapping_add(P3);
        i += 4;
    }
    while i < n {
        h = (h ^ (data[i] as u64).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
        i += 1;
    }
    h ^= h >> 33;
    h = h.wrapping_mul(P2);
    h ^= h >> 29;
    h = h.wrapping_mul(P3);
    h ^= h >> 32;
    h
}

fn u64_le(b: &[u8]) -> u64 {
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

fn u32_le(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Convenience: XXH64 formatted as 16-char lowercase hex.
pub fn xxh64_hex(data: &[u8], seed: u64) -> String {
    let mut out = String::with_capacity(16);
    let _ = write!(out, "{:016x}", xxh64(data, seed));
    out
}

// ---------------------------------------------------------------------------
// CRC32C (Castagnoli, reflected polynomial 0x82F63B78)
// ---------------------------------------------------------------------------

fn crc32c_table() -> &'static [u32; 256] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<[u32; 256]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for i in 0..256u32 {
            let mut crc = i;
            for _ in 0..8 {
                crc = if crc & 1 != 0 { (crc >> 1) ^ 0x82F63B78 } else { crc >> 1 };
            }
            t[i as usize] = crc;
        }
        t
    })
}

/// CRC32C (Castagnoli) of `data`.
pub fn crc32c(data: &[u8]) -> u32 {
    let table = crc32c_table();
    let mut crc: u32 = 0xFFFF_FFFF;
    for &b in data {
        crc = (crc >> 8) ^ table[((crc ^ b as u32) & 0xFF) as usize];
    }
    !crc
}

/// Convenience: CRC32C formatted as 8-char lowercase hex.
pub fn crc32c_hex(data: &[u8]) -> String {
    let mut out = String::with_capacity(8);
    let _ = write!(out, "{:08x}", crc32c(data));
    out
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Hex-encode a byte slice (lowercase).
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Select a hasher by name: `sha256` (default), `md5`, `xxh64`, `crc32c`.
pub fn by_name(name: &str) -> Option<Box<dyn Hasher>> {
    match name.to_ascii_lowercase().as_str() {
        "sha256" | "sha-256" => Some(Box::new(Sha256::new())),
        "md5" => Some(Box::new(Md5::new())),
        "xxh64" | "xxhash64" => Some(Box::new(Xxh64State::new())),
        "crc32c" => Some(Box::new(Crc32cState::new())),
        _ => None,
    }
}

/// Stateful XXH64 wrapper implementing [`Hasher`].
pub struct Xxh64State {
    seed: u64,
    buf: Vec<u8>,
}

impl Default for Xxh64State {
    fn default() -> Self {
        Self::new()
    }
}

impl Xxh64State {
    /// New state with seed 0.
    pub fn new() -> Self {
        Xxh64State { seed: 0, buf: Vec::new() }
    }
}

impl Hasher for Xxh64State {
    fn update(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }
    fn finish_hex(self: Box<Self>) -> String {
        xxh64_hex(&self.buf, self.seed)
    }
    fn len_bytes(&self) -> usize {
        8
    }
}

/// Stateful CRC32C wrapper implementing [`Hasher`].
pub struct Crc32cState {
    buf: Vec<u8>,
}

impl Default for Crc32cState {
    fn default() -> Self {
        Self::new()
    }
}

impl Crc32cState {
    /// New state.
    pub fn new() -> Self {
        Crc32cState { buf: Vec::new() }
    }
}

impl Hasher for Crc32cState {
    fn update(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }
    fn finish_hex(self: Box<Self>) -> String {
        crc32c_hex(&self.buf)
    }
    fn len_bytes(&self) -> usize {
        4
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_vectors_fips180_4() {
        // FIPS 180-4 / NIST vectors
        assert_eq!(
            Sha256::finish_hex_str(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            Sha256::finish_hex_str(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            Sha256::finish_hex_str(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn test_sha256_incremental_matches_one_shot() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let one = Sha256::finish_hex_str(&data);
        let mut h = Sha256::new();
        for chunk in data.chunks(997) {
            h.update(chunk);
        }
        assert_eq!(Box::new(h).finish_hex(), one);
    }

    #[test]
    fn test_sha256_one_million_a() {
        // "million a" vector from NIST examples
        let mut h = Sha256::new();
        for _ in 0..100_000 {
            h.update(&[b'a'; 10]);
        }
        assert_eq!(
            Box::new(h).finish_hex(),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn test_md5_vectors_rfc1321() {
        assert_eq!(Md5::finish_hex_str(b""), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(Md5::finish_hex_str(b"a"), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(Md5::finish_hex_str(b"abc"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(
            Md5::finish_hex_str(b"abcdefghijklmnopqrstuvwxyz"),
            "c3fcd3d76192e4007dfb496cca67e13b"
        );
    }

    #[test]
    fn test_xxh64_vectors() {
        // Reference values (xxHash spec / independent implementation)
        assert_eq!(xxh64(b"", 0), 0xef46db3751d8e999);
        assert_eq!(xxh64(b"a", 0), 0xd24ec4f1a98c6e5b);
        assert_eq!(
            xxh64(b"Nobody inspects the spammish repetition", 0),
            0xfbcea83c8a378bf1
        );
        assert_eq!(xxh64(b"abc", 0), 0x44bc2cf5ad770999);
    }

    #[test]
    fn test_crc32c_vectors_rfc3720() {
        // RFC 3720 B.4 / iSCSI CRC32C test vectors
        assert_eq!(crc32c(b""), 0x00000000);
        // RFC 3720 B.4 vectors (raw register values):
        assert_eq!(crc32c(&[0u8; 32]), 0x8a9136aa);
        assert_eq!(crc32c(&[0xffu8; 32]), 0x62a8ab43);
        assert_eq!(crc32c(&(0u8..32).collect::<Vec<u8>>()), 0x46dd794e);
        assert_eq!(crc32c(&(0u8..32).rev().collect::<Vec<u8>>()), 0x113fdb5c);
        // Canonical iSCSI check value
        assert_eq!(crc32c(b"123456789"), 0xe3069283);
        // incremental == one-shot
        let data: Vec<u8> = (0..70_000u32).map(|i| (i % 253) as u8).collect();
        let whole = crc32c(&data);
        let mut c = Crc32cState::new();
        for chunk in data.chunks(4096) {
            c.update(chunk);
        }
        assert_eq!(Box::new(c).finish_hex(), format!("{whole:08x}"));
    }

    #[test]
    fn test_by_name_dispatch() {
        assert_eq!(by_name("sha256").unwrap().len_bytes(), 32);
        assert_eq!(by_name("MD5").unwrap().len_bytes(), 16);
        assert_eq!(by_name("xxh64").unwrap().len_bytes(), 8);
        assert_eq!(by_name("crc32c").unwrap().len_bytes(), 4);
        assert!(by_name("nope").is_none());
    }

    #[test]
    fn test_to_hex() {
        assert_eq!(to_hex(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(to_hex(&[]), "");
    }

    impl Sha256 {
        fn finish_hex_str(data: &[u8]) -> String {
            let mut h = Sha256::new();
            h.update(data);
            Box::new(h).finish_hex()
        }
    }
    impl Md5 {
        fn finish_hex_str(data: &[u8]) -> String {
            let mut h = Md5::new();
            h.update(data);
            Box::new(h).finish_hex()
        }
    }
}
