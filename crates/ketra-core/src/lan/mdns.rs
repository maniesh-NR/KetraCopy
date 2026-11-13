//! Multicast DNS (RFC 6762) + DNS-Based Service Discovery (RFC 6763).
//!
//! Everything mDNS needs is implemented here on top of the OS socket API
//! (hand-declared FFI, no external crates):
//!
//! * a DNS message **codec** — questions/answers, RFC 1035 names with
//!   pointer decompression (including inside SRV/PTR rdata) — see [`packet`],
//! * one shared **socket thread** bound to `224.0.0.251:5353` that answers
//!   queries for our announced services and feeds responses to active
//!   [`Mdns::browse`] waiters, and
//! * registration helpers: [`Mdns::announce`] / [`Mdns::withdraw`].
//!
//! Coexistence: the socket is created with `SO_REUSEADDR` (+`SO_REUSEPORT`
//! where available) so it can share port 5353 with system resolvers such as
//! Avahi or mDNSResponder, matching standard mDNS practice.
//!
//! IPv4 only, like the services we interoperate with (LocalSend announces
//! `A` records under `_localsend._tcp.local`).

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The mDNS multicast group.
pub const MDNS_GROUP: [u8; 4] = [224, 0, 0, 251];
/// The mDNS port.
pub const MDNS_PORT: u16 = 5353;

// ---- record types --------------------------------------------------------
const TYPE_A: u16 = 1;
const TYPE_PTR: u16 = 12;
const TYPE_TXT: u16 = 16;
const TYPE_SRV: u16 = 33;

const CLASS_IN: u16 = 1;
/// mDNS cache-flush bit for unique records (RFC 6762 §10.2).
const CLASS_CACHE_FLUSH: u16 = 0x8000;

// ---------------------------------------------------------------------------
// DNS wire codec
// ---------------------------------------------------------------------------

pub mod packet {
    use super::{CLASS_CACHE_FLUSH, CLASS_IN};
    use std::collections::BTreeMap;

    /// One encoded question.
    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct Question {
        pub name: String,
        pub qtype: u16,
        pub qclass: u16,
    }

    /// One resource record. `ptr_target` / `srv` are convenience views
    /// resolved against the *full message* (they follow RFC 1035 §4.1.4
    /// compression pointers, which cannot be resolved from rdata alone).
    #[derive(Debug, Clone)]
    pub struct Record {
        pub name: String,
        pub rtype: u16,
        pub rclass: u16,
        pub ttl: u32,
        pub rdata: Vec<u8>,
        /// Resolved target when `rtype == PTR`.
        pub ptr_target: Option<String>,
        /// Resolved `(port, target)` when `rtype == SRV`.
        pub srv: Option<(u16, String)>,
    }

    /// A decoded DNS message.
    #[derive(Debug, Clone, Default)]
    pub struct Message {
        pub id: u16,
        pub is_response: bool,
        pub questions: Vec<Question>,
        pub answers: Vec<Record>,
        pub additional: Vec<Record>,
    }

    fn put_u16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_be_bytes());
    }
    fn put_u32(out: &mut Vec<u8>, v: u32) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    /// Encode a dotted display name (`foo.bar.local.`) as DNS labels.
    pub fn encode_name(out: &mut Vec<u8>, name: &str) {
        for label in name.trim_end_matches('.').split('.') {
            let bytes = label.as_bytes();
            let n = bytes.len().min(63) as u8;
            out.push(n);
            out.extend_from_slice(&bytes[..n as usize]);
        }
        out.push(0);
    }

    /// Decode a possibly-compressed name starting at `pos` inside `msg`;
    /// returns `(display_name_with_trailing_dot, next_index)`.
    fn decode_name(msg: &[u8], pos: usize) -> Option<(String, usize)> {
        let mut labels: Vec<String> = Vec::new();
        let mut i = pos;
        let mut jumped = false;
        let mut next = msg.len();
        let mut hops = 0;
        loop {
            let len = *msg.get(i)?;
            match len {
                0 => {
                    i += 1;
                    if !jumped {
                        next = i;
                    }
                    break;
                }
                0xC0..=0xFF => {
                    let ptr = (((len & 0x3F) as usize) << 8) | (*msg.get(i + 1)? as usize);
                    if !jumped {
                        next = i + 2;
                    }
                    hops += 1;
                    if hops > 32 || ptr >= msg.len() {
                        return None; // pointer loop / out of range
                    }
                    i = ptr;
                    jumped = true;
                }
                n => {
                    let n = n as usize;
                    let s = msg.get(i + 1..i + 1 + n)?;
                    labels.push(String::from_utf8_lossy(s).into_owned());
                    i += 1 + n;
                }
            }
            if labels.len() > 128 || i > msg.len() {
                return None;
            }
        }
        let mut name = labels.join(".");
        name.push('.');
        Some((name, next))
    }

    impl Message {
        /// Encode the message into wire format.
        pub fn encode(&self) -> Vec<u8> {
            let mut out = Vec::with_capacity(256);
            let flags = if self.is_response { 0x8400 } else { 0x0000 };
            put_u16(&mut out, self.id);
            put_u16(&mut out, flags);
            put_u16(&mut out, self.questions.len() as u16);
            put_u16(&mut out, self.answers.len() as u16);
            put_u16(&mut out, 0); // authority count
            put_u16(&mut out, self.additional.len() as u16);
            for q in &self.questions {
                encode_name(&mut out, &q.name);
                put_u16(&mut out, q.qtype);
                put_u16(&mut out, q.qclass);
            }
            for (set, flush) in [(&self.answers, true), (&self.additional, false)] {
                for r in set {
                    encode_name(&mut out, &r.name);
                    put_u16(&mut out, r.rtype);
                    put_u16(&mut out, r.rclass | if flush { CLASS_CACHE_FLUSH } else { 0 });
                    put_u32(&mut out, r.ttl);
                    put_u16(&mut out, r.rdata.len() as u16);
                    out.extend_from_slice(&r.rdata);
                }
            }
            out
        }

        /// Decode a message from wire format.
        pub fn decode(msg: &[u8]) -> Option<Message> {
            if msg.len() < 12 {
                return None;
            }
            let u16at = |o: usize| u16::from_be_bytes([msg[o], msg[o + 1]]);
            let qd = u16at(4) as usize;
            let an = u16at(6) as usize;
            let ar = u16at(10) as usize;
            let mut m = Message {
                id: u16at(0),
                is_response: u16at(2) & 0x8000 != 0,
                ..Default::default()
            };
            let mut pos = 12;
            for _ in 0..qd {
                let (name, next) = decode_name(msg, pos)?;
                if next + 4 > msg.len() {
                    return None;
                }
                m.questions.push(Question {
                    name,
                    qtype: u16at(next),
                    qclass: u16at(next + 2),
                });
                pos = next + 4;
            }
            for idx in 0..(an + ar) {
                let (name, next) = decode_name(msg, pos)?;
                if next + 10 > msg.len() {
                    return None;
                }
                let rtype = u16at(next);
                let rclass = u16at(next + 2);
                let ttl = u32::from_be_bytes([
                    msg[next + 4],
                    msg[next + 5],
                    msg[next + 6],
                    msg[next + 7],
                ]);
                let rdlen = u16at(next + 8) as usize;
                let start = next + 10;
                let end = start.checked_add(rdlen)?;
                let rdata = msg.get(start..end)?.to_vec();

                // Resolve rdata names against the full message (compression).
                let ptr_target = if rtype == super::TYPE_PTR {
                    decode_name(msg, start).map(|(n, _)| n)
                } else {
                    None
                };
                let srv = if rtype == super::TYPE_SRV && rdata.len() >= 8 {
                    let port = u16::from_be_bytes([rdata[4], rdata[5]]);
                    // target begins at rdata offset 6 (after prio/weight/port)
                    decode_name(msg, start + 6).map(|(n, _)| (port, n))
                } else {
                    None
                };

                let rec = Record {
                    name,
                    rtype,
                    rclass,
                    ttl,
                    rdata,
                    ptr_target,
                    srv,
                };
                if idx < an {
                    m.answers.push(rec);
                } else {
                    m.additional.push(rec);
                }
                pos = end;
            }
            Some(m)
        }

        /// Iterate all records (answers + additional).
        pub fn all_records(&self) -> impl Iterator<Item = &Record> {
            self.answers.iter().chain(self.additional.iter())
        }
    }

    /// Build an SRV rdata block (priority, weight, port, target).
    pub fn srv_rdata(priority: u16, weight: u16, port: u16, target: &str) -> Vec<u8> {
        let mut out = Vec::new();
        put_u16(&mut out, priority);
        put_u16(&mut out, weight);
        put_u16(&mut out, port);
        encode_name(&mut out, target);
        out
    }

    /// Build a TXT rdata block from key=value pairs.
    pub fn txt_rdata(txt: &BTreeMap<String, String>) -> Vec<u8> {
        let mut out = Vec::new();
        for (k, v) in txt {
            let entry = format!("{k}={v}");
            let bytes = entry.as_bytes();
            let n = bytes.len().min(255) as u8;
            out.push(n);
            out.extend_from_slice(&bytes[..n as usize]);
        }
        if out.is_empty() {
            out.push(0);
        }
        out
    }

    /// Parse a TXT rdata block into key=value pairs.
    pub fn parse_txt(rdata: &[u8]) -> BTreeMap<String, String> {
        let mut map = BTreeMap::new();
        let mut i = 0;
        while i < rdata.len() {
            let n = rdata[i] as usize;
            if n == 0 {
                i += 1;
                continue;
            }
            i += 1;
            if i + n > rdata.len() {
                break;
            }
            let s = String::from_utf8_lossy(&rdata[i..i + n]).into_owned();
            if let Some((k, v)) = s.split_once('=') {
                map.insert(k.to_string(), v.to_string());
            } else {
                map.insert(s, String::new());
            }
            i += n;
        }
        map
    }

    /// Build an A rdata block (IPv4).
    pub fn a_rdata(ip: [u8; 4]) -> Vec<u8> {
        ip.to_vec()
    }
}

// ---------------------------------------------------------------------------
// OS socket layer (unified over Linux/macOS/BSD/Windows; hand-declared FFI)
// ---------------------------------------------------------------------------

/// File-descriptor / socket-handle type used by the [`sys`] layer.
pub type Fd = isize;

/// sockaddr_in for IPv4: family, port (BE), addr (BE), zero padding.
#[repr(C)]
#[derive(Clone, Copy)]
struct SockAddrIn {
    sin_family: u16,
    sin_port: u16,
    sin_addr: u32,
    sin_zero: [u8; 8],
}

impl SockAddrIn {
    fn new(ip: [u8; 4], port: u16) -> Self {
        SockAddrIn {
            sin_family: sys::AF_INET as u16,
            sin_port: port.to_be(),
            // Raw network-order octets: store the bytes as-is. `from_be_bytes`
            // would byte-reverse them on little-endian hosts and send every
            // multicast packet to a mirrored address (224.0.0.251 ->
            // 251.0.0.224), silently killing all discovery.
            sin_addr: u32::from_ne_bytes(ip),
            sin_zero: [0; 8],
        }
    }
    fn ip(&self) -> [u8; 4] {
        self.sin_addr.to_ne_bytes()
    }
    fn port(&self) -> u16 {
        self.sin_port
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod sys {
    pub const AF_INET: i32 = 2;
    pub const SOCK_DGRAM: i32 = 2;
    pub const SOL_SOCKET: i32 = 1;
    pub const SO_REUSEADDR: i32 = 2;
    pub const SO_REUSEPORT: i32 = 15;
    pub const IPPROTO_IP: i32 = 0;
    pub const IP_ADD_MEMBERSHIP: i32 = 35;
    pub const IP_MULTICAST_TTL: i32 = 33;
    pub const IP_MULTICAST_LOOP: i32 = 34;
    pub const POLLIN: i16 = 0x001;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    unsafe extern "C" {
        fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
        fn bind(fd: i32, addr: *const u8, len: u32) -> i32;
        fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
        fn recvfrom(fd: i32, buf: *mut u8, len: usize, flags: i32, addr: *mut u8, alen: *mut u32) -> isize;
        fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
        #[link_name = "close"]
        fn raw_close(fd: i32) -> i32;
    }

    pub unsafe fn make_socket() -> super::Fd {
        let fd = socket(AF_INET, SOCK_DGRAM, 0);
        if fd < 0 {
            -1
        } else {
            fd as super::Fd
        }
    }
    pub unsafe fn set_i32(fd: super::Fd, level: i32, name: i32, v: i32) -> i32 {
        setsockopt(fd as i32, level, name, &v as *const i32 as *const u8, 4)
    }
    pub unsafe fn set_bytes(fd: super::Fd, level: i32, name: i32, v: &[u8]) -> i32 {
        setsockopt(fd as i32, level, name, v.as_ptr(), v.len() as u32)
    }
    pub unsafe fn bind_port(fd: super::Fd, port: u16) -> i32 {
        let a = super::SockAddrIn::new([0, 0, 0, 0], port);
        bind(fd as i32, &a as *const super::SockAddrIn as *const u8, std::mem::size_of::<super::SockAddrIn>() as u32)
    }
    pub unsafe fn send_to(fd: super::Fd, buf: &[u8], dst: &super::SockAddrIn) -> isize {
        sendto(
            fd as i32,
            buf.as_ptr(),
            buf.len(),
            0,
            dst as *const super::SockAddrIn as *const u8,
            std::mem::size_of::<super::SockAddrIn>() as u32,
        )
    }
    pub unsafe fn recv_from(fd: super::Fd, buf: &mut [u8], from: &mut super::SockAddrIn) -> isize {
        let mut alen = std::mem::size_of::<super::SockAddrIn>() as u32;
        recvfrom(
            fd as i32,
            buf.as_mut_ptr(),
            buf.len(),
            0,
            from as *mut super::SockAddrIn as *mut u8,
            &mut alen,
        )
    }
    pub unsafe fn wait_readable(fd: super::Fd, timeout_ms: i32) -> i32 {
        let mut p = PollFd { fd: fd as i32, events: POLLIN, revents: 0 };
        poll(&mut p, 1, timeout_ms)
    }
    pub unsafe fn close_fd(fd: super::Fd) {
        raw_close(fd as i32);
    }
}

#[cfg(target_os = "macos")]
mod sys {
    pub const AF_INET: i32 = 2;
    pub const SOCK_DGRAM: i32 = 2;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_REUSEADDR: i32 = 4;
    pub const SO_REUSEPORT: i32 = 0x200;
    pub const IPPROTO_IP: i32 = 0;
    pub const IP_ADD_MEMBERSHIP: i32 = 19;
    pub const IP_MULTICAST_TTL: i32 = 10;
    pub const IP_MULTICAST_LOOP: i32 = 11;
    pub const POLLIN: i16 = 0x001;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    unsafe extern "C" {
        fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
        fn bind(fd: i32, addr: *const u8, len: u32) -> i32;
        fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
        fn recvfrom(fd: i32, buf: *mut u8, len: usize, flags: i32, addr: *mut u8, alen: *mut u32) -> isize;
        fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
        #[link_name = "close"]
        fn raw_close(fd: i32) -> i32;
    }

    pub unsafe fn make_socket() -> super::Fd {
        let fd = socket(AF_INET, SOCK_DGRAM, 0);
        if fd < 0 {
            -1
        } else {
            fd as super::Fd
        }
    }
    pub unsafe fn set_i32(fd: super::Fd, level: i32, name: i32, v: i32) -> i32 {
        setsockopt(fd as i32, level, name, &v as *const i32 as *const u8, 4)
    }
    pub unsafe fn set_bytes(fd: super::Fd, level: i32, name: i32, v: &[u8]) -> i32 {
        setsockopt(fd as i32, level, name, v.as_ptr(), v.len() as u32)
    }
    pub unsafe fn bind_port(fd: super::Fd, port: u16) -> i32 {
        let a = super::SockAddrIn::new([0, 0, 0, 0], port);
        bind(fd as i32, &a as *const super::SockAddrIn as *const u8, std::mem::size_of::<super::SockAddrIn>() as u32)
    }
    pub unsafe fn send_to(fd: super::Fd, buf: &[u8], dst: &super::SockAddrIn) -> isize {
        sendto(
            fd as i32,
            buf.as_ptr(),
            buf.len(),
            0,
            dst as *const super::SockAddrIn as *const u8,
            std::mem::size_of::<super::SockAddrIn>() as u32,
        )
    }
    pub unsafe fn recv_from(fd: super::Fd, buf: &mut [u8], from: &mut super::SockAddrIn) -> isize {
        let mut alen = std::mem::size_of::<super::SockAddrIn>() as u32;
        recvfrom(
            fd as i32,
            buf.as_mut_ptr(),
            buf.len(),
            0,
            from as *mut super::SockAddrIn as *mut u8,
            &mut alen,
        )
    }
    pub unsafe fn wait_readable(fd: super::Fd, timeout_ms: i32) -> i32 {
        let mut p = PollFd { fd: fd as i32, events: POLLIN, revents: 0 };
        poll(&mut p, 1, timeout_ms)
    }
    pub unsafe fn close_fd(fd: super::Fd) {
        raw_close(fd as i32);
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "android", target_os = "macos"))))]
mod sys {
    // Generic BSD-family fallback; constants follow the BSD layout.
    pub const AF_INET: i32 = 2;
    pub const SOCK_DGRAM: i32 = 2;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_REUSEADDR: i32 = 4;
    pub const SO_REUSEPORT: i32 = 0x200;
    pub const IPPROTO_IP: i32 = 0;
    pub const IP_ADD_MEMBERSHIP: i32 = 19;
    pub const IP_MULTICAST_TTL: i32 = 10;
    pub const IP_MULTICAST_LOOP: i32 = 11;
    pub const POLLIN: i16 = 0x001;

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    unsafe extern "C" {
        fn socket(domain: i32, ty: i32, protocol: i32) -> i32;
        fn setsockopt(fd: i32, level: i32, name: i32, value: *const u8, len: u32) -> i32;
        fn bind(fd: i32, addr: *const u8, len: u32) -> i32;
        fn sendto(fd: i32, buf: *const u8, len: usize, flags: i32, addr: *const u8, alen: u32) -> isize;
        fn recvfrom(fd: i32, buf: *mut u8, len: usize, flags: i32, addr: *mut u8, alen: *mut u32) -> isize;
        fn poll(fds: *mut PollFd, nfds: u64, timeout: i32) -> i32;
        fn close(fd: i32) -> i32;
    }

    pub unsafe fn make_socket() -> super::Fd {
        let fd = socket(AF_INET, SOCK_DGRAM, 0);
        if fd < 0 {
            -1
        } else {
            fd as super::Fd
        }
    }
    pub unsafe fn set_i32(fd: super::Fd, level: i32, name: i32, v: i32) -> i32 {
        setsockopt(fd as i32, level, name, &v as *const i32 as *const u8, 4)
    }
    pub unsafe fn set_bytes(fd: super::Fd, level: i32, name: i32, v: &[u8]) -> i32 {
        setsockopt(fd as i32, level, name, v.as_ptr(), v.len() as u32)
    }
    pub unsafe fn bind_port(fd: super::Fd, port: u16) -> i32 {
        let a = super::SockAddrIn::new([0, 0, 0, 0], port);
        bind(fd as i32, &a as *const super::SockAddrIn as *const u8, std::mem::size_of::<super::SockAddrIn>() as u32)
    }
    pub unsafe fn send_to(fd: super::Fd, buf: &[u8], dst: &super::SockAddrIn) -> isize {
        sendto(
            fd as i32,
            buf.as_ptr(),
            buf.len(),
            0,
            dst as *const super::SockAddrIn as *const u8,
            std::mem::size_of::<super::SockAddrIn>() as u32,
        )
    }
    pub unsafe fn recv_from(fd: super::Fd, buf: &mut [u8], from: &mut super::SockAddrIn) -> isize {
        let mut alen = std::mem::size_of::<super::SockAddrIn>() as u32;
        recvfrom(
            fd as i32,
            buf.as_mut_ptr(),
            buf.len(),
            0,
            from as *mut super::SockAddrIn as *mut u8,
            &mut alen,
        )
    }
    pub unsafe fn wait_readable(fd: super::Fd, timeout_ms: i32) -> i32 {
        let mut p = PollFd { fd: fd as i32, events: POLLIN, revents: 0 };
        poll(&mut p, 1, timeout_ms)
    }
    pub unsafe fn close(fd: super::Fd) {
        close(fd as i32);
    }
}

#[cfg(target_os = "windows")]
mod sys {
    pub const AF_INET: i32 = 2;
    pub const SOCK_DGRAM: i32 = 2;
    pub const SOL_SOCKET: i32 = 0xffff;
    pub const SO_REUSEADDR: i32 = 4;
    pub const IPPROTO_IP: i32 = 0;
    pub const IP_ADD_MEMBERSHIP: i32 = 5;
    pub const IP_MULTICAST_TTL: i32 = 10;
    pub const IP_MULTICAST_LOOP: i32 = 11;
    pub const POLLIN: i16 = 0x001;

    type Socket = usize; // SOCKET

    #[repr(C)]
    struct PollFd {
        fd: Socket,
        events: i16,
        revents: i16,
    }

    #[repr(C)]
    struct WsaData {
        bytes: [u8; 400],
    }

    unsafe extern "system" {
        fn WSAStartup(version: u16, data: *mut WsaData) -> i32;
        fn socket(domain: i32, ty: i32, protocol: i32) -> Socket;
        fn setsockopt(fd: Socket, level: i32, name: i32, value: *const u8, len: i32) -> i32;
        fn bind(fd: Socket, addr: *const u8, len: i32) -> i32;
        fn sendto(fd: Socket, buf: *const u8, len: i32, flags: i32, addr: *const u8, alen: i32) -> i32;
        fn recvfrom(fd: Socket, buf: *mut u8, len: i32, flags: i32, addr: *mut u8, alen: *mut i32) -> i32;
        fn WSAPoll(fds: *mut PollFd, nfds: u32, timeout: i32) -> i32;
        fn closesocket(fd: Socket) -> i32;
    }

    /// Ensure Winsock is initialized (idempotent; std initializes it too).
    fn wsa_init() {
        unsafe {
            let mut data = WsaData { bytes: [0; 400] };
            WSAStartup(0x0202, &mut data);
        }
    }

    pub unsafe fn make_socket() -> super::Fd {
        wsa_init();
        let fd = socket(AF_INET, SOCK_DGRAM, 0);
        if fd == usize::MAX {
            -1
        } else {
            fd as super::Fd
        }
    }
    pub unsafe fn set_i32(fd: super::Fd, level: i32, name: i32, v: i32) -> i32 {
        setsockopt(fd as Socket, level, name, &v as *const i32 as *const u8, 4)
    }
    pub unsafe fn set_bytes(fd: super::Fd, level: i32, name: i32, v: &[u8]) -> i32 {
        setsockopt(fd as Socket, level, name, v.as_ptr(), v.len() as i32)
    }
    pub unsafe fn bind_port(fd: super::Fd, port: u16) -> i32 {
        let a = super::SockAddrIn::new([0, 0, 0, 0], port);
        bind(fd as Socket, &a as *const super::SockAddrIn as *const u8, std::mem::size_of::<super::SockAddrIn>() as i32)
    }
    pub unsafe fn send_to(fd: super::Fd, buf: &[u8], dst: &super::SockAddrIn) -> isize {
        sendto(
            fd as Socket,
            buf.as_ptr(),
            buf.len() as i32,
            0,
            dst as *const super::SockAddrIn as *const u8,
            std::mem::size_of::<super::SockAddrIn>() as i32,
        ) as isize
    }
    pub unsafe fn recv_from(fd: super::Fd, buf: &mut [u8], from: &mut super::SockAddrIn) -> isize {
        let mut alen = std::mem::size_of::<super::SockAddrIn>() as i32;
        recvfrom(
            fd as Socket,
            buf.as_mut_ptr(),
            buf.len() as i32,
            0,
            from as *mut super::SockAddrIn as *mut u8,
            &mut alen,
        ) as isize
    }
    pub unsafe fn wait_readable(fd: super::Fd, timeout_ms: i32) -> i32 {
        let mut p = PollFd { fd: fd as Socket, events: POLLIN, revents: 0 };
        WSAPoll(&mut p, 1, timeout_ms)
    }
    pub unsafe fn close_fd(fd: super::Fd) {
        closesocket(fd as Socket);
    }
}

// ---------------------------------------------------------------------------
// Multicast socket wrapper
// ---------------------------------------------------------------------------

struct McastSocket {
    fd: Fd,
}

impl McastSocket {
    /// Create the socket, shareable-bind it to :5353 and join the mDNS group.
    fn new() -> Option<McastSocket> {
        unsafe {
            let fd = sys::make_socket();
            if fd < 0 {
                return None;
            }
            let s = McastSocket { fd };
            let _ = sys::set_i32(fd, sys::SOL_SOCKET, sys::SO_REUSEADDR, 1);
            #[cfg(unix)]
            {
                let _ = sys::set_i32(fd, sys::SOL_SOCKET, sys::SO_REUSEPORT, 1);
            }
            if sys::bind_port(fd, MDNS_PORT) != 0 {
                sys::close_fd(fd);
                return None;
            }
            // ip_mreq { imr_multiaddr, imr_interface } — two BE u32 addresses.
            let mreq: [u8; 8] = [MDNS_GROUP[0], MDNS_GROUP[1], MDNS_GROUP[2], MDNS_GROUP[3], 0, 0, 0, 0];
            if sys::set_bytes(fd, sys::IPPROTO_IP, sys::IP_ADD_MEMBERSHIP, &mreq) != 0 {
                sys::close_fd(fd);
                return None;
            }
            let _ = sys::set_i32(fd, sys::IPPROTO_IP, sys::IP_MULTICAST_TTL, 255);
            let _ = sys::set_i32(fd, sys::IPPROTO_IP, sys::IP_MULTICAST_LOOP, 1);
            Some(s)
        }
    }

    fn send_multicast(&self, data: &[u8]) -> bool {
        let dst = SockAddrIn::new(MDNS_GROUP, MDNS_PORT);
        let r = unsafe { sys::send_to(self.fd, data, &dst) };
        if cfg!(feature = "mdns_debug") || std::env::var_os("KETRA_MDNS_DEBUG").is_some() {
            eprintln!("[mdns] send_multicast {} bytes -> {}", data.len(), r);
        }
        r == data.len() as isize
    }

    /// Receive one datagram (with timeout). Returns `(data, src_ip, src_port)`.
    fn recv(&self, timeout_ms: i32) -> Option<(Vec<u8>, [u8; 4], u16)> {
        unsafe {
            let pr = sys::wait_readable(self.fd, timeout_ms);
            if std::env::var_os("KETRA_MDNS_DEBUG").is_some() && pr != 0 {
                eprintln!("[mdns sock] poll fd={} -> {}", self.fd, pr);
            }
            if pr <= 0 {
                return None;
            }
            let mut buf = [0u8; 9000];
            let mut from = SockAddrIn::new([0, 0, 0, 0], 0);
            let n = sys::recv_from(self.fd, &mut buf, &mut from);
            if n <= 0 {
                return None;
            }
            Some((buf[..n as usize].to_vec(), from.ip(), from.port()))
        }
    }
}

impl Drop for McastSocket {
    fn drop(&mut self) {
        unsafe { sys::close_fd(self.fd) };
    }
}

// ---------------------------------------------------------------------------
// Public discovery API
// ---------------------------------------------------------------------------

/// One discovered service instance (before the optional HTTP identity probe).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovered {
    /// SRV target hostname (`<instance>.local.`).
    pub hostname: String,
    /// IPv4 address from the A record.
    pub ip: String,
    /// TCP port from the SRV record.
    pub port: u16,
    /// TXT record contents.
    pub txt: BTreeMap<String, String>,
    /// Service type it was found under (e.g. `_localsend._tcp.local.`).
    pub service: String,
    /// Instance name (the PTR target).
    pub instance: String,
}

impl Discovered {
    /// Alias announced in the TXT record (`alias=...`), or the instance name.
    pub fn alias(&self) -> String {
        self.txt
            .get("alias")
            .cloned()
            .unwrap_or_else(|| self.instance.trim_end_matches('.').to_string())
    }

    /// True when the TXT record advertises the Ketra accelerated protocol
    /// (`ketra=1`).
    pub fn is_ketra(&self) -> bool {
        self.txt.get("ketra").map(|v| v == "1").unwrap_or(false)
    }

    /// Device fingerprint from the TXT record, if present.
    pub fn fingerprint(&self) -> Option<String> {
        self.txt.get("fingerprint").cloned()
    }
}

#[derive(Clone)]
struct Announcement {
    service: String,
    instance: String,
    hostname: String,
    port: u16,
    txt: BTreeMap<String, String>,
    ip: [u8; 4],
}

#[derive(Clone)]
struct Waiter {
    id: u64,
    services: Vec<String>,
    sender: Sender<Discovered>,
}

struct Shared {
    socket: McastSocket,
    announced: Mutex<Vec<Announcement>>,
    waiters: Mutex<Vec<Waiter>>,
    next_id: AtomicU64,
    running: AtomicBool,
    quit: Sender<()>,
}

/// A shared mDNS endpoint (spawn one per process; it owns the :5353 socket).
pub struct Mdns {
    shared: Arc<Shared>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl std::fmt::Debug for Mdns {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Mdns")
    }
}

/// Best-effort local IPv4 (the address of a default-route interface).
/// Connecting a UDP socket picks a route without sending any packets.
pub fn local_ip() -> String {
    let s = match std::net::UdpSocket::bind("0.0.0.0:0") {
        Ok(s) => s,
        Err(_) => return "127.0.0.1".to_string(),
    };
    if s.connect("8.8.8.8:53").is_err() {
        return "127.0.0.1".to_string();
    }
    match s.local_addr() {
        Ok(std::net::SocketAddr::V4(v4)) => v4.ip().to_string(),
        _ => "127.0.0.1".to_string(),
    }
}

impl Mdns {
    /// Start the shared socket thread. Returns `None` only when the multicast
    /// socket cannot be created at all (no network / no multicast support).
    pub fn spawn() -> Option<Arc<Mdns>> {
        let socket = McastSocket::new()?;
        let (tx, rx): (Sender<()>, Receiver<()>) = std::sync::mpsc::channel();
        let shared = Arc::new(Shared {
            socket,
            announced: Mutex::new(Vec::new()),
            waiters: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            running: AtomicBool::new(true),
            quit: tx,
        });
        let s2 = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("ketra-mdns".into())
            .spawn(move || socket_loop(s2, rx))
            .ok()?;
        Some(Arc::new(Mdns { shared, thread: Some(thread) }))
    }

    /// Is the socket thread still alive?
    pub fn alive(&self) -> bool {
        self.shared.running.load(Ordering::Relaxed)
    }

    /// Announce `instance` under every service with the given TXT records.
    ///
    /// Sends an unsolicited registration packet immediately (RFC 6762 §8.3)
    /// and keeps answering queries until [`Mdns::withdraw`].
    pub fn announce(
        &self,
        services: &[&str],
        instance: &str,
        port: u16,
        txt: &BTreeMap<String, String>,
    ) {
        let ip4 = parse_ipv4(&local_ip());
        let hostname = format!("{}.local.", instance.trim_end_matches('.'));
        let mut msg = packet::Message {
            id: 0,
            is_response: true,
            ..Default::default()
        };
        let mut anns = Vec::new();
        for svc in services {
            let service = service_names(svc).1;
            let inst_full = format!("{instance}.{}", service.trim_end_matches('.'));
            let mut ptr_d = Vec::new();
            packet::encode_name(&mut ptr_d, &inst_full);
            msg.answers.push(packet::Record {
                name: service.clone(),
                rtype: TYPE_PTR,
                rclass: CLASS_IN,
                ttl: 4500,
                rdata: ptr_d,
                ptr_target: None,
                srv: None,
            });
            msg.additional.push(packet::Record {
                name: inst_full.clone(),
                rtype: TYPE_SRV,
                rclass: CLASS_IN,
                ttl: 120,
                rdata: packet::srv_rdata(0, 0, port, &hostname),
                ptr_target: None,
                srv: None,
            });
            msg.additional.push(packet::Record {
                name: inst_full.clone(),
                rtype: TYPE_TXT,
                rclass: CLASS_IN,
                ttl: 4500,
                rdata: packet::txt_rdata(txt),
                ptr_target: None,
                srv: None,
            });
            msg.additional.push(packet::Record {
                name: hostname.clone(),
                rtype: TYPE_A,
                rclass: CLASS_IN,
                ttl: 120,
                rdata: packet::a_rdata(ip4),
                ptr_target: None,
                srv: None,
            });
            anns.push(Announcement {
                service,
                instance: inst_full,
                hostname: hostname.clone(),
                port,
                txt: txt.clone(),
                ip: ip4,
            });
        }
        self.shared.announced.lock().unwrap().extend(anns);
        let _ = self.shared.socket.send_multicast(&msg.encode());
    }

    /// Stop announcing everything (sends goodbye packets with TTL=0).
    pub fn withdraw(&self) {
        let announced: Vec<Announcement> = {
            let mut g = self.shared.announced.lock().unwrap();
            std::mem::take(&mut *g)
        };
        if announced.is_empty() {
            return;
        }
        let mut msg = packet::Message {
            id: 0,
            is_response: true,
            ..Default::default()
        };
        for a in &announced {
            let mut d = Vec::new();
            packet::encode_name(&mut d, &a.instance);
            msg.answers.push(packet::Record {
                name: a.service.clone(),
                rtype: TYPE_PTR,
                rclass: CLASS_IN,
                ttl: 0,
                rdata: d,
                ptr_target: None,
                srv: None,
            });
        }
        let _ = self.shared.socket.send_multicast(&msg.encode());
    }

    /// Browse `services` until `timeout` elapses; returns every instance seen.
    /// The query is re-sent once per second while browsing (RFC 6762 §5.2).
    pub fn browse(&self, services: &[&str], timeout: Duration) -> Vec<Discovered> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed);
        let (tx, rx): (Sender<Discovered>, Receiver<Discovered>) = std::sync::mpsc::channel();
        let svc_names: Vec<String> = services.iter().map(|s| service_names(s).1).collect();
        {
            let mut w = self.shared.waiters.lock().unwrap();
            w.push(Waiter { id, services: svc_names.clone(), sender: tx });
        }
        let query = self.browse_query(services);
        let _ = self.shared.socket.send_multicast(&query);
        let mut last_query = Instant::now();
        let mut found: BTreeMap<String, Discovered> = BTreeMap::new();
        let deadline = Instant::now() + timeout;
        while let Some(left) = deadline.checked_duration_since(Instant::now()) {
            if last_query.elapsed() >= Duration::from_secs(1) {
                let _ = self.shared.socket.send_multicast(&query);
                last_query = Instant::now();
            }
            match rx.recv_timeout(left.min(Duration::from_millis(200))) {
                Ok(d) => {
                    let key = format!("{}|{}|{}", d.instance, d.ip, d.port);
                    found.insert(key, d);
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        self.shared.waiters.lock().unwrap().retain(|w| w.id != id);
        found.into_values().collect()
    }

    fn browse_query(&self, services: &[&str]) -> Vec<u8> {
        let id = self.shared.next_id.fetch_add(1, Ordering::Relaxed) as u16;
        let mut msg = packet::Message {
            id,
            is_response: false,
            ..Default::default()
        };
        for svc in services {
            msg.questions.push(packet::Question {
                name: service_names(svc).1,
                qtype: TYPE_PTR,
                qclass: CLASS_IN,
            });
        }
        msg.encode()
    }
}

impl Drop for Mdns {
    fn drop(&mut self) {
        self.withdraw();
        self.shared.running.store(false, Ordering::Relaxed);
        let _ = self.shared.quit.send(());
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Normalize a service string to its full dotted name:
/// `_localsend._tcp` → `_localsend._tcp.local.` (also accepts full names).
fn service_names(svc: &str) -> (String, String) {
    let mut s = svc.trim().trim_end_matches('.').to_string();
    if !s.contains("._tcp") && !s.contains("._udp") {
        s.push_str("._tcp");
    }
    if !s.ends_with(".local") {
        s.push_str(".local");
    }
    s.push('.');
    (s.clone(), s)
}

fn parse_ipv4(s: &str) -> [u8; 4] {
    let mut out = [127, 0, 0, 1];
    for (i, part) in s.split('.').enumerate() {
        if let Ok(v) = part.parse::<u8>() {
            if i < 4 {
                out[i] = v;
            }
        }
    }
    out
}

/// The socket thread: receives packets, answers queries, feeds waiters.
fn socket_loop(shared: Arc<Shared>, quit: Receiver<()>) {
    // Accumulated discovery records (so split responses still resolve).
    let mut srvs: BTreeMap<String, (u16, String)> = BTreeMap::new();
    let mut txts: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    let mut addrs: BTreeMap<String, [u8; 4]> = BTreeMap::new();
    let mut ptr_targets: Vec<(String, String)> = Vec::new(); // (service, instance)

    while shared.running.load(Ordering::Relaxed) {
        match quit.try_recv() {
            Ok(()) => break,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        }
        let (data, _src_ip, _src_port) = match shared.socket.recv(300) {
            Some(x) => x,
            None => continue,
        };
        if std::env::var_os("KETRA_MDNS_DEBUG").is_some() {
            eprintln!(
                "[mdns {:?}] recv {} bytes from {:?}",
                std::thread::current().name().unwrap_or("?"),
                data.len(),
                _src_ip
            );
        }
        let msg = match packet::Message::decode(&data) {
            Some(m) => m,
            None => {
                if std::env::var_os("KETRA_MDNS_DEBUG").is_some() {
                    eprintln!("[mdns] DECODE FAILED for {} bytes: {:02x?}", data.len(), &data[..data.len().min(48)]);
                }
                continue;
            }
        };

        // ---- responder side: answer queries about our announcements ----
        if !msg.is_response && !msg.questions.is_empty() {
            let announced = shared.announced.lock().unwrap().clone();
            if !announced.is_empty() {
                let mut reply = packet::Message {
                    id: 0,
                    is_response: true,
                    ..Default::default()
                };
                let mut matched = false;
                for q in &msg.questions {
                    let qname = q.name.to_ascii_lowercase();
                    for a in &announced {
                        let hit_ptr = q.qtype == TYPE_PTR && qname == a.service.to_ascii_lowercase();
                        let hit_srv =
                            q.qtype == TYPE_SRV && qname == a.instance.to_ascii_lowercase();
                        let hit_txt =
                            q.qtype == TYPE_TXT && qname == a.instance.to_ascii_lowercase();
                        let hit_a = q.qtype == TYPE_A && qname == a.hostname.to_ascii_lowercase();
                        if hit_ptr {
                            matched = true;
                            let mut d = Vec::new();
                            packet::encode_name(&mut d, &a.instance);
                            reply.answers.push(packet::Record {
                                name: a.service.clone(),
                                rtype: TYPE_PTR,
                                rclass: CLASS_IN,
                                ttl: 4500,
                                rdata: d,
                                ptr_target: None,
                                srv: None,
                            });
                            reply.additional.push(packet::Record {
                                name: a.instance.clone(),
                                rtype: TYPE_SRV,
                                rclass: CLASS_IN,
                                ttl: 120,
                                rdata: packet::srv_rdata(0, 0, a.port, &a.hostname),
                                ptr_target: None,
                                srv: None,
                            });
                            reply.additional.push(packet::Record {
                                name: a.instance.clone(),
                                rtype: TYPE_TXT,
                                rclass: CLASS_IN,
                                ttl: 4500,
                                rdata: packet::txt_rdata(&a.txt),
                                ptr_target: None,
                                srv: None,
                            });
                            // The A record is required: browsers only complete
                            // an instance once SRV + A are both known.
                            reply.additional.push(packet::Record {
                                name: a.hostname.clone(),
                                rtype: TYPE_A,
                                rclass: CLASS_IN,
                                ttl: 120,
                                rdata: packet::a_rdata(a.ip),
                                ptr_target: None,
                                srv: None,
                            });
                        } else if hit_srv {
                            matched = true;
                            reply.answers.push(packet::Record {
                                name: a.instance.clone(),
                                rtype: TYPE_SRV,
                                rclass: CLASS_IN,
                                ttl: 120,
                                rdata: packet::srv_rdata(0, 0, a.port, &a.hostname),
                                ptr_target: None,
                                srv: None,
                            });
                        } else if hit_txt {
                            matched = true;
                            reply.answers.push(packet::Record {
                                name: a.instance.clone(),
                                rtype: TYPE_TXT,
                                rclass: CLASS_IN,
                                ttl: 4500,
                                rdata: packet::txt_rdata(&a.txt),
                                ptr_target: None,
                                srv: None,
                            });
                        } else if hit_a {
                            matched = true;
                            reply.answers.push(packet::Record {
                                name: a.hostname.clone(),
                                rtype: TYPE_A,
                                rclass: CLASS_IN,
                                ttl: 120,
                                rdata: packet::a_rdata(a.ip),
                                ptr_target: None,
                                srv: None,
                            });
                        }
                    }
                }
                if matched {
                    let _ = shared.socket.send_multicast(&reply.encode());
                }
            }
            continue;
        }

        // ---- browser side: accumulate records, complete instances ----
        if msg.is_response {
            for r in msg.all_records() {
                let key = r.name.to_ascii_lowercase();
                match r.rtype {
                    TYPE_PTR => {
                        if let Some(t) = r.ptr_target.clone() {
                            ptr_targets.push((r.name.clone(), t));
                        }
                    }
                    TYPE_SRV => {
                        if let Some((port, target)) = r.srv.clone() {
                            srvs.insert(key.clone(), (port, target));
                        }
                    }
                    TYPE_TXT => {
                        txts.insert(key.clone(), packet::parse_txt(&r.rdata));
                    }
                    TYPE_A => {
                        if r.rdata.len() == 4 {
                            let mut ip = [0u8; 4];
                            ip.copy_from_slice(&r.rdata);
                            addrs.insert(key.clone(), ip);
                        }
                    }
                    _ => {}
                }
            }
            // Resolve every known instance that has full SRV+A data.
            let waiters = shared.waiters.lock().unwrap().clone();
            for (service, instance) in &ptr_targets {
                let ikey = instance.to_ascii_lowercase();
                let Some((port, hostname)) = srvs.get(&ikey) else { continue };
                let Some(ip) = addrs.get(&hostname.to_ascii_lowercase()) else { continue };
                let txt = txts.get(&ikey).cloned().unwrap_or_default();
                let d = Discovered {
                    hostname: hostname.clone(),
                    ip: format!("{}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]),
                    port: *port,
                    txt,
                    service: service.clone(),
                    instance: instance.clone(),
                };
                for w in &waiters {
                    if w.services.iter().any(|s| s.eq_ignore_ascii_case(&service)) {
                        let _ = w.sender.send(d.clone());
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_encode_decode_roundtrip() {
        let mut txt = BTreeMap::new();
        txt.insert("alias".to_string(), "Ketra-Desktop".to_string());
        txt.insert("ketra".to_string(), "1".to_string());
        let mut msg = packet::Message {
            id: 42,
            is_response: true,
            ..Default::default()
        };
        msg.questions.push(packet::Question {
            name: "_localsend._tcp.local.".into(),
            qtype: TYPE_PTR,
            qclass: CLASS_IN,
        });
        msg.answers.push(packet::Record {
            name: "_localsend._tcp.local.".into(),
            rtype: TYPE_PTR,
            rclass: CLASS_IN,
            ttl: 4500,
            rdata: {
                let mut d = Vec::new();
                packet::encode_name(&mut d, "desk1._localsend._tcp.local.");
                d
            },
            ptr_target: None,
            srv: None,
        });
        msg.additional.push(packet::Record {
            name: "desk1._localsend._tcp.local.".into(),
            rtype: TYPE_SRV,
            rclass: CLASS_IN,
            ttl: 120,
            rdata: packet::srv_rdata(0, 0, 53317, "desk1.local."),
            ptr_target: None,
            srv: None,
        });
        msg.additional.push(packet::Record {
            name: "desk1._localsend._tcp.local.".into(),
            rtype: TYPE_TXT,
            rclass: CLASS_IN,
            ttl: 4500,
            rdata: packet::txt_rdata(&txt),
            ptr_target: None,
            srv: None,
        });
        msg.additional.push(packet::Record {
            name: "desk1.local.".into(),
            rtype: TYPE_A,
            rclass: CLASS_IN,
            ttl: 120,
            rdata: packet::a_rdata([192, 168, 1, 42]),
            ptr_target: None,
            srv: None,
        });
        let wire = msg.encode();
        let back = packet::Message::decode(&wire).expect("decode");
        assert_eq!(back.id, 42);
        assert!(back.is_response);
        assert_eq!(back.questions.len(), 1);
        assert_eq!(back.answers.len(), 1);
        assert_eq!(back.additional.len(), 3);
        // PTR target resolved via full-message decode:
        assert_eq!(
            back.answers[0].ptr_target.as_deref(),
            Some("desk1._localsend._tcp.local.")
        );
        // SRV (port, target) resolved:
        assert_eq!(back.additional[0].srv, Some((53317, "desk1.local.".to_string())));
        let txt_back = packet::parse_txt(&back.additional[1].rdata);
        assert_eq!(txt_back.get("alias").unwrap(), "Ketra-Desktop");
        assert_eq!(txt_back.get("ketra").unwrap(), "1");
        assert_eq!(back.additional[2].rdata, vec![192, 168, 1, 42]);
    }

    #[test]
    fn test_compression_pointer_decoding() {
        // Question name at offset 12; the answer name is a pointer (0xC0 0x0C).
        let mut wire: Vec<u8> = vec![0, 1, 0, 0, 0, 1, 0, 1, 0, 0, 0, 0];
        packet::encode_name(&mut wire, "desk1.local.");
        wire.extend_from_slice(&TYPE_A.to_be_bytes());
        wire.extend_from_slice(&CLASS_IN.to_be_bytes());
        // answer: name = compression pointer to offset 12
        wire.extend_from_slice(&[0xC0, 12]);
        wire.extend_from_slice(&TYPE_A.to_be_bytes());
        wire.extend_from_slice(&(CLASS_IN | CLASS_CACHE_FLUSH).to_be_bytes());
        wire.extend_from_slice(&120u32.to_be_bytes());
        wire.extend_from_slice(&2u16.to_be_bytes());
        wire.extend_from_slice(&[10, 20]);
        let m = packet::Message::decode(&wire).expect("decode with pointer");
        assert_eq!(m.questions[0].name, "desk1.local.");
        assert_eq!(m.answers[0].name, "desk1.local.");
        assert_eq!(m.answers[0].rdata, vec![10, 20]);
    }

    #[test]
    fn test_txt_rdata_roundtrip() {
        let mut txt = BTreeMap::new();
        txt.insert("a".to_string(), "1".to_string());
        txt.insert("long".to_string(), "x".repeat(200));
        let rdata = packet::txt_rdata(&txt);
        let back = packet::parse_txt(&rdata);
        assert_eq!(back.get("a").unwrap(), "1");
        assert!(back.get("long").unwrap().starts_with("xxx"));
    }

    #[test]
    fn test_service_names() {
        assert_eq!(
            service_names("_localsend._tcp"),
            ("_localsend._tcp.local.".into(), "_localsend._tcp.local.".into())
        );
        assert_eq!(
            service_names("_localsend._tcp.local."),
            ("_localsend._tcp.local.".into(), "_localsend._tcp.local.".into())
        );
        assert_eq!(service_names("_ketra._tcp.local").0, "_ketra._tcp.local.");
    }

    #[test]
    fn test_local_ip_is_dotted_quad() {
        let ip = local_ip();
        assert_eq!(ip.split('.').count(), 4, "{ip}");
        assert!(ip.split('.').all(|p| p.parse::<u8>().is_ok()), "{ip}");
    }

    /// Live multicast announce→browse test. Only runs when
    /// `KETRA_MDNS_LIVE=1` (multicast is unavailable in some sandboxes).
    #[test]
    fn test_live_announce_browse() {
        if std::env::var("KETRA_MDNS_LIVE").ok().as_deref() != Some("1") {
            return;
        }
        let a = Mdns::spawn().expect("mdns socket");
        let mut txt = BTreeMap::new();
        txt.insert("alias".to_string(), "ketra-test-a".to_string());
        txt.insert("ketra".to_string(), "1".to_string());
        a.announce(&["_ketra._tcp.local"], "ketra-test-a", 53317, &txt);
        let b = Mdns::spawn().expect("mdns socket 2");
        let found = b.browse(&["_ketra._tcp.local"], Duration::from_secs(3));
        assert!(
            found.iter().any(|d| d.alias() == "ketra-test-a"),
            "did not discover our own announcement: {found:?}"
        );
        a.withdraw();
    }
}
