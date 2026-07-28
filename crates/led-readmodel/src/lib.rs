//! # led-readmodel — the read-only snapshot the operator UI polls
//!
//! Aggregates already-confirmed sources into one serialisable view:
//! - [`DeviceStatus`] (`led-core`) — per-controller connectivity/frames,
//! - [`HealthStatus`] (`led-protocols`) — heartbeat health (Ok/Warning/Critical),
//! - [`DiscoveryResult`] (`led-protocols`) — pre-show ArtPoll (responded/missing),
//! - a small [`MetricsView`] filled from `led-hal`'s `MetricsEmitter` by the caller.
//!
//! **Read-only by construction.** Nothing here mutates engine state, sends a command, or
//! touches the render/send hot path — it is exactly the surface ADR-0013/0014 lets the UI
//! consume (the UI reads this; the daemon owns the engine).
//!
//! JSON is **hand-rolled** to match the workspace convention (`MetricsEmitter::snapshot_json`,
//! `Provenance::to_json`); the workspace has no `serde` dependency and this crate stays
//! std-only. Field names are stable — the UI depends on them.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use led_core::DeviceStatus;
use led_protocols::{DiscoveryResult, HealthStatus};

/// A minimal metrics view for the UI. Filled from `led-hal`'s `MetricsEmitter` by whoever
/// assembles the snapshot (kept as plain fields so this crate needs no dependency on the HAL).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct MetricsView {
    pub fps: u64,
    pub p50_us: u64,
    pub p99_us: u64,
    pub drops: u64,
    pub hb_gap_ms: u64,
}

/// One controller's read-only status, tagged with its stable device id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeviceView {
    pub id: u16,
    pub status: DeviceStatus,
}

/// The whole read-only snapshot the UI polls. Read-only by construction.
#[derive(Clone, Debug)]
pub struct ReadModel {
    pub devices: Vec<DeviceView>,
    pub health: HealthStatus,
    pub metrics: MetricsView,
    pub discovery: Option<DiscoveryResult>,
}

impl ReadModel {
    /// Serialise to one JSON object (hand-rolled, std-only). Stable field names.
    pub fn to_json(&self) -> String {
        let health = match self.health {
            HealthStatus::Ok => "ok",
            HealthStatus::Warning => "warning",
            HealthStatus::Critical => "critical",
        };
        let devices = self
            .devices
            .iter()
            .map(|d| {
                format!(
                    r#"{{"id":{},"connected":{},"frames_sent":{},"last_send_ms":{}}}"#,
                    d.id, d.status.connected, d.status.frames_sent, d.status.last_send_ms
                )
            })
            .collect::<Vec<_>>()
            .join(",");
        let m = &self.metrics;
        let discovery = match &self.discovery {
            None => "null".to_string(),
            Some(d) => format!(
                r#"{{"responded":[{}],"missing":[{}]}}"#,
                ip_list(&d.responded),
                ip_list(&d.missing)
            ),
        };
        format!(
            r#"{{"health":"{health}","devices":[{devices}],"metrics":{{"fps":{},"p50_us":{},"p99_us":{},"drops":{},"hb_gap_ms":{}}},"discovery":{discovery}}}"#,
            m.fps, m.p50_us, m.p99_us, m.drops, m.hb_gap_ms
        )
    }
}

fn ip_list(v: &[Ipv4Addr]) -> String {
    v.iter()
        .map(|ip| format!("\"{ip}\""))
        .collect::<Vec<_>>()
        .join(",")
}

// ── Read-only localhost serve ───────────────────────────────────────────────────

/// Handle to a running read-model endpoint. Mirrors `led-hal`'s `MetricsServer`:
/// dropping it does NOT stop the server; call [`ReadModelServer::stop`] (tests) or let it
/// live for the process (shows).
#[derive(Debug)]
pub struct ReadModelServer {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ReadModelServer {
    /// Ask the accept loop to exit and join the thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::net::TcpStream::connect(self.addr); // unblock accept()
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Serve `GET /` → the current read-model as JSON on `addr`. One thread, sequential accepts
/// (mirrors `led-hal::serve_metrics`). `source` produces the current snapshot per request.
///
/// **Security (`/security`):** refuses to bind a **non-loopback** address by default — the UI
/// read channel is same-host only; LAN access requires the authenticated path (ADR-0014).
/// Never binds `0.0.0.0`.
pub fn serve_readmodel<F>(addr: SocketAddr, source: F) -> std::io::Result<ReadModelServer>
where
    F: Fn() -> ReadModel + Send + 'static,
{
    if !addr.ip().is_loopback() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "read-model serve refuses a non-loopback bind (use 127.0.0.1); LAN needs ADR-0014 auth",
        ));
    }
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    let handle = std::thread::Builder::new()
        .name("lumyx-readmodel".into())
        .spawn(move || {
            for conn in listener.incoming() {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = conn else { continue };
                // Read the request headers — they may span multiple TCP segments.
                let mut raw = Vec::new();
                let mut buf = [0u8; 512];
                while !raw.windows(4).any(|w| w == b"\r\n\r\n") && raw.len() < 8192 {
                    match stream.read(&mut buf) {
                        Ok(0) => break,
                        Ok(n) => raw.extend_from_slice(&buf[..n]),
                        Err(_) => break,
                    }
                }
                let req = String::from_utf8_lossy(&raw);
                let (status, body) = if req.starts_with("GET / ")
                    || req.starts_with("GET /readmodel")
                {
                    ("200 OK", source().to_json())
                } else {
                    ("404 Not Found", String::from("not found\n"))
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write); // clean FIN after the response
            }
        })?;

    Ok(ReadModelServer { addr: bound, stop, handle: Some(handle) })
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn dev(id: u16, connected: bool, frames: u64) -> DeviceView {
        DeviceView {
            id,
            status: DeviceStatus { connected, frames_sent: frames, last_send_ms: 0 },
        }
    }

    #[test]
    fn empty_snapshot_serialises_with_stable_shape() {
        let rm = ReadModel {
            devices: vec![],
            health: HealthStatus::Ok,
            metrics: MetricsView::default(),
            discovery: None,
        };
        let j = rm.to_json();
        assert!(j.contains(r#""health":"ok""#), "{j}");
        assert!(j.contains(r#""devices":[]"#), "{j}");
        assert!(j.contains(r#""discovery":null"#), "{j}");
        assert!(j.contains(r#""fps":0"#) && j.contains(r#""p99_us":0"#), "{j}");
        assert!(braces_balanced(&j), "unbalanced JSON: {j}");
    }

    #[test]
    fn health_variants_map_to_stable_strings() {
        for (h, want) in [
            (HealthStatus::Ok, "ok"),
            (HealthStatus::Warning, "warning"),
            (HealthStatus::Critical, "critical"),
        ] {
            let rm = ReadModel {
                devices: vec![],
                health: h,
                metrics: MetricsView::default(),
                discovery: None,
            };
            assert!(rm.to_json().contains(&format!(r#""health":"{want}""#)));
        }
    }

    #[test]
    fn devices_metrics_and_discovery_are_serialised() {
        let rm = ReadModel {
            devices: vec![dev(0, true, 42), dev(7, false, 0)],
            health: HealthStatus::Warning,
            metrics: MetricsView { fps: 44, p50_us: 120, p99_us: 4100, drops: 3, hb_gap_ms: 800 },
            discovery: Some(DiscoveryResult {
                responded: vec![Ipv4Addr::new(192, 168, 2, 156)],
                missing: vec![Ipv4Addr::new(192, 168, 2, 157)],
            }),
        };
        let j = rm.to_json();
        assert!(j.contains(r#""id":0,"connected":true,"frames_sent":42"#), "{j}");
        assert!(j.contains(r#""id":7,"connected":false"#), "{j}");
        assert!(j.contains(r#""p99_us":4100"#) && j.contains(r#""drops":3"#), "{j}");
        assert!(j.contains(r#""responded":["192.168.2.156"]"#), "{j}");
        assert!(j.contains(r#""missing":["192.168.2.157"]"#), "{j}");
        assert!(braces_balanced(&j), "unbalanced JSON: {j}");
    }

    fn sample() -> ReadModel {
        ReadModel {
            devices: vec![dev(0, true, 5)],
            health: HealthStatus::Ok,
            metrics: MetricsView::default(),
            discovery: None,
        }
    }

    fn http_get(addr: std::net::SocketAddr, path: &str) -> String {
        use std::net::TcpStream;
        let mut s = TcpStream::connect(addr).unwrap();
        let req = format!("GET {path} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).unwrap(); // single write — don't split the request line
        // Accumulate what the server sent; tolerate a RST that may follow the response.
        let mut resp = Vec::new();
        let mut buf = [0u8; 1024];
        loop {
            match s.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => resp.extend_from_slice(&buf[..n]),
                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionReset => break,
                Err(e) => panic!("read: {e}"),
            }
        }
        String::from_utf8_lossy(&resp).into_owned()
    }

    #[test]
    fn serve_returns_json_on_root_and_404_elsewhere() {
        let server =
            serve_readmodel("127.0.0.1:0".parse().unwrap(), sample).expect("bind loopback");
        let addr = server.addr;

        let ok = http_get(addr, "/");
        assert!(ok.starts_with("HTTP/1.1 200 OK"), "{ok}");
        assert!(ok.contains("application/json"), "{ok}");
        assert!(ok.contains(r#""health":"ok""#), "{ok}");

        let nf = http_get(addr, "/nope");
        assert!(nf.starts_with("HTTP/1.1 404 Not Found"), "{nf}");

        server.stop();
    }

    #[test]
    fn serve_refuses_non_loopback_bind() {
        // /security: never bind a non-loopback address by default.
        let err = serve_readmodel("0.0.0.0:0".parse().unwrap(), sample).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::PermissionDenied);
    }

    /// Cheap structural sanity (the workspace has no serde to parse with): braces/brackets balance.
    fn braces_balanced(s: &str) -> bool {
        let mut curly = 0i32;
        let mut square = 0i32;
        for c in s.chars() {
            match c {
                '{' => curly += 1,
                '}' => curly -= 1,
                '[' => square += 1,
                ']' => square -= 1,
                _ => {}
            }
            if curly < 0 || square < 0 {
                return false;
            }
        }
        curly == 0 && square == 0
    }
}
