//! Bonjour announcing through macOS's mDNSResponder (`dns_sd.h`, in
//! libSystem), on a host name of the bridge's own.
//!
//! Why not the Mac's own host name, as `DNSServiceRegister` does by default:
//! that name resolves to every address the Mac has, IPv6 ones (link-local,
//! ULA) included, and Tobii SDK 3 knows only IPv4. Given such a host it never
//! connects, and its `createEyeTracker` never returns either: Tobii's own
//! plug-ins call it on the main thread, so the app freezes. An X2-60 has one
//! IPv4 address and nothing else. So the bridge announces
//! `<host>.local.` with one A record per interface, holding that
//! interface's IPv4 address (`127.0.0.1` for local clients), and points the
//! service at it.
//!
//! Self-contained (only `libc`), so it can be tried apart from the bridge.

use std::ffi::{CString, c_char, c_void};
use std::thread;

type DNSServiceRef = *mut c_void;
type DNSRecordRef = *mut c_void;
type RecordReply = extern "C" fn(DNSServiceRef, DNSRecordRef, u32, i32, *mut c_void);
type RegisterReply = extern "C" fn(
    DNSServiceRef,
    u32,
    i32,
    *const c_char,
    *const c_char,
    *const c_char,
    *mut c_void,
);

const K_SHARE_CONNECTION: u32 = 0x4000;
const K_UNIQUE: u32 = 0x20;
/// `kDNSServiceInterfaceIndexLocalOnly`: seen by this machine's clients only.
const K_LOCAL_ONLY: u32 = u32::MAX;
const K_TYPE_A: u16 = 1;
const K_CLASS_IN: u16 = 1;

#[allow(non_snake_case)]
unsafe extern "C" {
    fn DNSServiceCreateConnection(sdRef: *mut DNSServiceRef) -> i32;
    fn DNSServiceRegisterRecord(
        sdRef: DNSServiceRef,
        recordRef: *mut DNSRecordRef,
        flags: u32,
        interfaceIndex: u32,
        fullname: *const c_char,
        rrtype: u16,
        rrclass: u16,
        rdlen: u16,
        rdata: *const c_void,
        ttl: u32,
        callBack: RecordReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceRegister(
        sdRef: *mut DNSServiceRef,
        flags: u32,
        interfaceIndex: u32,
        name: *const c_char,
        regtype: *const c_char,
        domain: *const c_char,
        host: *const c_char,
        port: u16,
        txtLen: u16,
        txtRecord: *const c_void,
        callBack: RegisterReply,
        context: *mut c_void,
    ) -> i32;
    fn DNSServiceProcessResult(sdRef: DNSServiceRef) -> i32;
}

extern "C" fn record_reply(_: DNSServiceRef, _: DNSRecordRef, _: u32, err: i32, _: *mut c_void) {
    if err != 0 {
        eprintln!("bonjour: host record error {err}");
    }
}

extern "C" fn register_reply(
    _: DNSServiceRef,
    _: u32,
    err: i32,
    _: *const c_char,
    _: *const c_char,
    _: *const c_char,
    _: *mut c_void,
) {
    if err != 0 {
        eprintln!("bonjour: service registration error {err}");
    }
}

/// This machine's IPv4 addresses that mDNS can be answered on, with the
/// interface index to announce each on.
fn ipv4_interfaces() -> Vec<(u32, [u8; 4], String)> {
    let mut out = Vec::new();
    let mut ifap: *mut libc::ifaddrs = std::ptr::null_mut();
    // SAFETY: getifaddrs fills `ifap` with a list we only read, then free
    // with freeifaddrs; every pointer is checked for null before use.
    unsafe {
        if libc::getifaddrs(&raw mut ifap) != 0 {
            return out;
        }
        let mut p = ifap;
        while !p.is_null() {
            let ifa = &*p;
            p = ifa.ifa_next;
            if ifa.ifa_addr.is_null() || i32::from((*ifa.ifa_addr).sa_family) != libc::AF_INET {
                continue;
            }
            let flags = ifa.ifa_flags as i32;
            let up = flags & libc::IFF_UP != 0 && flags & libc::IFF_RUNNING != 0;
            let loopback = flags & libc::IFF_LOOPBACK != 0;
            // mDNS runs on multicast-capable, non-point-to-point links (no
            // VPN tunnels).
            let mdns = flags & libc::IFF_MULTICAST != 0 && flags & libc::IFF_POINTOPOINT == 0;
            if !up || !(loopback || mdns) {
                continue;
            }
            let sin = &*ifa.ifa_addr.cast::<libc::sockaddr_in>();
            let ip = sin.sin_addr.s_addr.to_ne_bytes();
            let name = std::ffi::CStr::from_ptr(ifa.ifa_name)
                .to_string_lossy()
                .into_owned();
            let index = if loopback {
                K_LOCAL_ONLY
            } else {
                libc::if_nametoindex(ifa.ifa_name)
            };
            if index != 0 {
                out.push((index, ip, name));
            }
        }
        libc::freeifaddrs(ifap);
    }
    out
}

/// Announce `instance` (of type `regtype`, e.g. `_tetserver._tcp`) on
/// `port`, on host `<host>.local.` whose only addresses are this machine's
/// IPv4 ones. Lasts as long as the process. Returns the addresses announced
/// (as "interface address" strings), for the log.
pub fn register(
    instance: &str,
    regtype: &str,
    host: &str,
    port: u16,
    txt: &[u8],
) -> Result<Vec<String>, String> {
    let c = |s: &str| CString::new(s).map_err(|e| e.to_string());
    let fullname = c(&format!("{host}.local."))?;
    let (c_instance, c_regtype) = (c(instance)?, c(regtype)?);
    let txt_len = u16::try_from(txt.len()).map_err(|_| "TXT record too long".to_owned())?;

    let addresses = ipv4_interfaces();
    if addresses.is_empty() {
        return Err("no IPv4 address to announce".into());
    }

    let mut conn: DNSServiceRef = std::ptr::null_mut();
    // SAFETY: plain out-parameter; on success `conn` is a live connection to
    // mDNSResponder, which is never deallocated: the records and the service
    // registered on it must last as long as the process.
    let err = unsafe { DNSServiceCreateConnection(&raw mut conn) };
    if err != 0 {
        return Err(format!("DNSServiceCreateConnection error {err}"));
    }

    let mut announced = Vec::new();
    for (index, ip, name) in &addresses {
        let mut rec: DNSRecordRef = std::ptr::null_mut();
        // SAFETY: `conn` is live, `fullname` and `ip` outlive the call (the
        // client library copies them into its request), the callback is a
        // valid `extern "C"` function.
        let err = unsafe {
            DNSServiceRegisterRecord(
                conn,
                &raw mut rec,
                K_UNIQUE,
                *index,
                fullname.as_ptr(),
                K_TYPE_A,
                K_CLASS_IN,
                4,
                ip.as_ptr().cast(),
                120,
                record_reply,
                std::ptr::null_mut(),
            )
        };
        if err == 0 {
            announced.push(format!("{name} {}.{}.{}.{}", ip[0], ip[1], ip[2], ip[3]));
        }
    }
    if announced.is_empty() {
        return Err("mDNSResponder refused every host record".into());
    }

    let mut svc = conn; // shares `conn`
    // SAFETY: as above; `svc` starts as a copy of `conn`, as
    // kDNSServiceFlagsShareConnection requires.
    let err = unsafe {
        DNSServiceRegister(
            &raw mut svc,
            K_SHARE_CONNECTION,
            0,
            c_instance.as_ptr(),
            c_regtype.as_ptr(),
            std::ptr::null(),
            fullname.as_ptr(),
            port.to_be(),
            txt_len,
            txt.as_ptr().cast(),
            register_reply,
            std::ptr::null_mut(),
        )
    };
    if err != 0 {
        return Err(format!("DNSServiceRegister error {err}"));
    }

    // Read mDNSResponder's replies (and report errors) for as long as the
    // process lives.
    let conn_addr = conn as usize;
    thread::spawn(move || {
        // SAFETY: the connection is never deallocated, so it stays valid.
        while unsafe { DNSServiceProcessResult(conn_addr as DNSServiceRef) } == 0 {}
    });
    Ok(announced)
}
