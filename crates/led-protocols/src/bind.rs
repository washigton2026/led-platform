//! Where a sending socket binds **locally** — the one place in this crate that decides what
//! "nothing was declared" means.
//!
//! ## Why this module exists
//!
//! Every sender in this crate used to write `UdpSocket::bind("0.0.0.0:0")` inline. That is not
//! a neutral default: it hands the choice of **egress interface** to the routing table, and on
//! a host with more than one address that reaches the target — the LUMYX bench host is
//! dual-homed on a single subnet (`en0` WiFi and `en7` Ethernet, both `192.168.2.0/24`) — the
//! routing table, not the operator, decides which cable the show goes down. A profile that
//! declares `OutputInterface::Ethernet` could not be honoured, because nothing below it could
//! express *which* local address to send from.
//!
//! ## What a bind can and cannot do
//!
//! - **Unicast** — binding a local address fixes the datagram's **source address**, and with it
//!   the interface the packet leaves by. This is the case the daemon and the player use, and it
//!   is the one the tests prove on the wire.
//! - **Multicast** — the egress interface is chosen by `IP_MULTICAST_IF` (or the route), *not*
//!   by the bind. `std::net::UdpSocket` does not expose `IP_MULTICAST_IF`, so this crate does
//!   **not** offer a bind parameter for multicast senders: accepting one would suggest a
//!   guarantee it could not keep. Named here rather than left for someone to discover.
//!
//! ## The default is still the wildcard, and that is deliberate
//!
//! `None` keeps the previous behaviour byte for byte. Declaring a source is opt-in because the
//! single-homed host — every CI runner, most rigs — has exactly one answer and the routing
//! table already knows it.

use std::net::{Ipv4Addr, SocketAddr, UdpSocket};

/// Open a sending UDP socket, bound to `bind` when the caller declared one.
///
/// `None` is the wildcard `0.0.0.0:0`: any interface, any ephemeral port — the egress is the
/// routing table's choice. `Some(local)` binds exactly `local`; port `0` there still means "any
/// port", so `Some(192.168.2.163:0)` is the usual production form: *this interface, whatever
/// port*.
///
/// A bind that fails (address not on this host, port already taken) is an **error**, never a
/// silent fall back to the wildcard — falling back would send the show down a cable the
/// operator did not choose and report success.
pub fn bind_sender(bind: Option<SocketAddr>) -> std::io::Result<UdpSocket> {
    match bind {
        Some(local) => UdpSocket::bind(local),
        None => UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn none_is_the_wildcard_and_some_is_exactly_what_was_asked() {
        let auto = bind_sender(None).unwrap();
        assert!(auto.local_addr().unwrap().ip().is_unspecified(), "None = 0.0.0.0");

        let sock = bind_sender(Some("127.0.0.1:0".parse().unwrap())).unwrap();
        let local = sock.local_addr().unwrap();
        assert_eq!(local.ip(), "127.0.0.1".parse::<std::net::IpAddr>().unwrap());
        assert_ne!(local.port(), 0, "porta 0 pedida = porta efémera atribuída");
    }

    /// Um endereço que não existe neste host é **erro**, não um regresso ao wildcard.
    #[test]
    fn an_address_this_host_does_not_have_is_an_error() {
        // 203.0.113.0/24 é TEST-NET-3 (RFC 5737): nunca atribuído a uma interface real.
        let r = bind_sender(Some("203.0.113.7:0".parse().unwrap()));
        assert!(r.is_err(), "bind num endereço inexistente tem de falhar, nunca cair no wildcard");
    }
}
