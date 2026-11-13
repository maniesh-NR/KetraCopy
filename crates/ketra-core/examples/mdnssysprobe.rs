//! Minimal std-only replication of McastSocket::new()'s exact syscall
//! sequence, to bisect why looped multicast never wakes poll() in the
//! engine while an equivalent Python socket does.
//! Usage: mdnssysprobe [mode]
//!   mode 1 = exact ketra sequence (REUSEADDR, REUSEPORT, bind, join, TTL, LOOP)
//!   mode 2 = like 1 but without SO_REUSEPORT
//!   mode 3 = like 1 but join AFTER setting TTL/LOOP
//!   mode 4 = like 1 but bind AFTER join
use std::os::raw::{c_int, c_void};

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
            sin_family: 2, // AF_INET
            sin_port: port.to_be(),
            sin_addr: u32::from_ne_bytes(ip),
            sin_zero: [0; 8],
        }
    }
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: i16,
    revents: i16,
}

unsafe extern "C" {
    fn socket(domain: c_int, ty: c_int, protocol: c_int) -> c_int;
    fn setsockopt(fd: c_int, level: c_int, name: c_int, value: *const c_void, len: u32) -> c_int;
    fn bind(fd: c_int, addr: *const c_void, len: u32) -> c_int;
    fn sendto(fd: c_int, buf: *const c_void, len: usize, flags: c_int, addr: *const c_void, alen: u32) -> isize;
    fn recvfrom(fd: c_int, buf: *mut c_void, len: usize, flags: c_int, addr: *mut c_void, alen: *mut u32) -> isize;
    fn poll(fds: *mut PollFd, nfds: u64, timeout: c_int) -> c_int;
    fn close(fd: c_int) -> c_int;
}

const SOL_SOCKET: c_int = 1;
const SO_REUSEADDR: c_int = 2;
const SO_REUSEPORT: c_int = 15;
const IPPROTO_IP: c_int = 0;
const IP_ADD_MEMBERSHIP: c_int = 35;
const IP_MULTICAST_TTL: c_int = 33;
const IP_MULTICAST_LOOP: c_int = 34;
const POLLIN: i16 = 0x001;
const GROUP: [u8; 4] = [224, 0, 0, 251];
const PORT: u16 = 5353;

fn main() {
    let mode: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    unsafe {
        let fd = socket(2, 2, 0); // AF_INET, SOCK_DGRAM, 0
        assert!(fd >= 0, "socket() failed");
        let one: c_int = 1;
        setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, &one as *const _ as *const c_void, 4);
        if mode != 2 {
            setsockopt(fd, SOL_SOCKET, SO_REUSEPORT, &one as *const _ as *const c_void, 4);
        }
        let do_join = |fd: c_int| {
            let mreq: [u8; 8] = [GROUP[0], GROUP[1], GROUP[2], GROUP[3], 0, 0, 0, 0];
            let r = setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP, mreq.as_ptr() as *const c_void, 8);
            eprintln!("join -> {}", r);
        };
        match mode {
            4 => do_join(fd), // join before bind
            _ => {}
        }
        let a = SockAddrIn::new([0, 0, 0, 0], PORT);
        let br = bind(fd, &a as *const SockAddrIn as *const c_void, 16);
        eprintln!("bind -> {}", br);
        if mode == 3 {
            let ttl: c_int = 255;
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_TTL, &ttl as *const _ as *const c_void, 4);
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_LOOP, &one as *const _ as *const c_void, 4);
            do_join(fd);
        } else if mode == 4 {
            let ttl: c_int = 255;
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_TTL, &ttl as *const _ as *const c_void, 4);
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_LOOP, &one as *const _ as *const c_void, 4);
        } else {
            let mreq: [u8; 8] = [GROUP[0], GROUP[1], GROUP[2], GROUP[3], 0, 0, 0, 0];
            let r = setsockopt(fd, IPPROTO_IP, IP_ADD_MEMBERSHIP, mreq.as_ptr() as *const c_void, 8);
            eprintln!("join -> {}", r);
            let ttl: c_int = 255;
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_TTL, &ttl as *const _ as *const c_void, 4);
            setsockopt(fd, IPPROTO_IP, IP_MULTICAST_LOOP, &one as *const _ as *const c_void, 4);
        }

        // send a small multicast packet (an mDNS-like query)
        let pkt = [0u8; 35];
        let dst = SockAddrIn::new(GROUP, PORT);
        let sr = sendto(fd, pkt.as_ptr() as *const c_void, 35, 0, &dst as *const SockAddrIn as *const c_void, 16);
        eprintln!("sendto -> {}", sr);

        // poll + recv for 3s
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            let mut p = PollFd { fd, events: POLLIN, revents: 0 };
            let pr = poll(&mut p, 1, 300);
            eprintln!("poll -> {}", pr);
            if pr > 0 {
                let mut buf = [0u8; 9000];
                let mut from = SockAddrIn::new([0, 0, 0, 0], 0);
                let n = recvfrom(fd, buf.as_mut_ptr() as *mut c_void, 9000, 0, &mut from as *mut SockAddrIn as *mut c_void, &mut 16u32);
                eprintln!("recvfrom -> {} bytes", n);
                break;
            }
        }
        close(fd);
    }
}
