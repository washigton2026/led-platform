//! Prometheus exposition for [`MetricsEmitter`] — text format 0.0.4 plus a
//! minimal std-only HTTP endpoint (`GET /metrics`).
//!
//! ## Invariants (lumyx-observability-engineer)
//! - Rendering a scrape NEVER touches the frame hot path: it only reads the
//!   emitter's atomics/histogram, same cost as `snapshot_json`.
//! - The HTTP server is one thread, one connection at a time — a scrape
//!   endpoint, not a web server. Prometheus scrapes every 15–60s; that's the
//!   entire load profile.
//! - Metric names follow Prometheus conventions: `lumyx_` prefix, `_total`
//!   suffix on counters, base units (seconds) on latency gauges.

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpListener};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::metrics::MetricsEmitter;

/// Render the standard LUMYX metric set in Prometheus text format.
pub fn prometheus_text(emitters: &[Arc<MetricsEmitter>]) -> String {
    let mut out = String::with_capacity(1024);
    out.push_str("# HELP lumyx_frames_total Frames sent to devices.\n");
    out.push_str("# TYPE lumyx_frames_total counter\n");
    for e in emitters {
        out.push_str(&format!(
            "lumyx_frames_total{{node=\"{}\"}} {}\n",
            e.name(),
            e.frame_count()
        ));
    }
    out.push_str("# HELP lumyx_drops_total Frames dropped before reaching a device.\n");
    out.push_str("# TYPE lumyx_drops_total counter\n");
    for e in emitters {
        out.push_str(&format!(
            "lumyx_drops_total{{node=\"{}\"}} {}\n",
            e.name(),
            e.drop_count()
        ));
    }
    out.push_str("# HELP lumyx_beats_total Audio beats detected.\n");
    out.push_str("# TYPE lumyx_beats_total counter\n");
    for e in emitters {
        out.push_str(&format!(
            "lumyx_beats_total{{node=\"{}\"}} {}\n",
            e.name(),
            e.beat_count()
        ));
    }
    out.push_str("# HELP lumyx_frame_latency_seconds Frame send latency (quantiles from HDR-lite histogram).\n");
    out.push_str("# TYPE lumyx_frame_latency_seconds summary\n");
    for e in emitters {
        let name = e.name();
        out.push_str(&format!(
            "lumyx_frame_latency_seconds{{node=\"{name}\",quantile=\"0.5\"}} {}\n",
            e.p50_us() as f64 / 1e6
        ));
        out.push_str(&format!(
            "lumyx_frame_latency_seconds{{node=\"{name}\",quantile=\"0.99\"}} {}\n",
            e.p99_us() as f64 / 1e6
        ));
    }
    out
}

/// Handle to a running metrics endpoint. Dropping it does NOT stop the server;
/// call [`MetricsServer::stop`] (tests) or let it live for the process (shows).
pub struct MetricsServer {
    pub addr: SocketAddr,
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl MetricsServer {
    /// Ask the accept loop to exit and join the thread.
    pub fn stop(mut self) {
        self.stop.store(true, Ordering::Relaxed);
        // Unblock accept() with a dummy connection.
        let _ = std::net::TcpStream::connect(self.addr);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Serve `GET /metrics` on `addr` (use port 0 to pick a free port; the real
/// bound address is in the returned handle). One thread, sequential accepts.
pub fn serve_metrics(
    emitters: Vec<Arc<MetricsEmitter>>,
    addr: SocketAddr,
) -> std::io::Result<MetricsServer> {
    let listener = TcpListener::bind(addr)?;
    let bound = listener.local_addr()?;
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();

    let handle = std::thread::Builder::new()
        .name("lumyx-metrics".into())
        .spawn(move || {
            for conn in listener.incoming() {
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(mut stream) = conn else { continue };
                // Read the request line; we only serve GET /metrics.
                let mut buf = [0u8; 512];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let (status, body) = if req.starts_with("GET /metrics") {
                    ("200 OK", prometheus_text(&emitters))
                } else {
                    ("404 Not Found", String::from("not found\n"))
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Type: text/plain; version=0.0.4\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            }
        })?;

    Ok(MetricsServer { addr: bound, stop, handle: Some(handle) })
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::BufRead;

    fn emitter_with_data(name: &'static str) -> Arc<MetricsEmitter> {
        let e = Arc::new(MetricsEmitter::new(name));
        e.record_frame(1000);
        e.record_frame(2000);
        e.record_drop();
        e.record_beat();
        e
    }

    #[test]
    fn text_format_has_help_type_and_values() {
        let e = emitter_with_data("node-a");
        let text = prometheus_text(&[e]);
        assert!(text.contains("# HELP lumyx_frames_total"));
        assert!(text.contains("# TYPE lumyx_frames_total counter"));
        assert!(text.contains("lumyx_frames_total{node=\"node-a\"} 2"));
        assert!(text.contains("lumyx_drops_total{node=\"node-a\"} 1"));
        assert!(text.contains("lumyx_beats_total{node=\"node-a\"} 1"));
        assert!(text.contains("quantile=\"0.99\""));
    }

    #[test]
    fn latency_is_reported_in_seconds() {
        let e = Arc::new(MetricsEmitter::new("s"));
        e.record_frame(1_000_000); // 1s
        let text = prometheus_text(&[e]);
        let line = text
            .lines()
            .find(|l| l.contains("quantile=\"0.5\""))
            .expect("p50 line");
        let v: f64 = line.rsplit(' ').next().unwrap().parse().unwrap();
        assert!((0.5..=2.0).contains(&v), "1s recorded → ~1.0 exposed, got {v}");
    }

    #[test]
    fn multiple_nodes_render_one_series_each() {
        let a = emitter_with_data("node-a");
        let b = emitter_with_data("node-b");
        let text = prometheus_text(&[a, b]);
        assert!(text.contains("node=\"node-a\""));
        assert!(text.contains("node=\"node-b\""));
        // HELP/TYPE headers appear once per metric, not per node
        assert_eq!(text.matches("# TYPE lumyx_frames_total").count(), 1);
    }

    #[test]
    fn http_endpoint_serves_metrics_and_404() {
        let e = emitter_with_data("http-node");
        let server =
            serve_metrics(vec![e], "127.0.0.1:0".parse().unwrap()).expect("bind");
        let addr = server.addr;

        // GET /metrics → 200 with our series
        let mut s = std::net::TcpStream::connect(addr).unwrap();
        write!(s, "GET /metrics HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut resp = String::new();
        std::io::BufReader::new(&mut s).read_to_string(&mut resp).unwrap();
        assert!(resp.starts_with("HTTP/1.1 200"), "got: {}", &resp[..40.min(resp.len())]);
        assert!(resp.contains("lumyx_frames_total{node=\"http-node\"} 2"));

        // GET /other → 404
        let mut s2 = std::net::TcpStream::connect(addr).unwrap();
        write!(s2, "GET /other HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
        let mut line = String::new();
        std::io::BufReader::new(&mut s2).read_line(&mut line).unwrap();
        assert!(line.starts_with("HTTP/1.1 404"));

        server.stop();
    }
}
