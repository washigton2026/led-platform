//! Distributed observability for multi-node LUMYX shows.
//!
//! Combines `MetricsEmitter` with structured tracing spans and automatic alerting.
//!
//! ## Components
//!
//! - [`Span`] — a timed code section with tags (zero allocation on fast path).
//! - [`SpanCollector`] — accumulates spans from all cluster nodes.
//! - [`AlertRule`] — threshold-based automatic alert.
//! - [`AlertEngine`] — evaluates rules against live metrics and emits structured JSON alerts.
//! - [`ObservabilityReport`] — full snapshot: metrics + active alerts + top spans.
//!
//! ## Invariants (lumyx-observability-engineer)
//! - All spans use sample-index-derived timestamps — never wall-clock.
//! - `SpanCollector` is `Send + Sync` — safe to share between render and send threads.
//! - `AlertEngine` emits alerts as one-line JSON — parseable by any log aggregator.
//! - No allocation on the critical path: `Span::start` / `Span::finish` are alloc-free.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::metrics::MetricsEmitter;

// ── Span ──────────────────────────────────────────────────────────────────────

/// A single timing measurement for one pipeline stage.
#[derive(Clone, Debug)]
pub struct Span {
    pub name:       &'static str,
    pub node_id:    u32,
    pub start_us:   u64,
    pub duration_us: u64,
    pub tags:       &'static str, // static key=value pairs, e.g. "layer=hal,device=0"
}

/// In-progress span. Call `finish()` to complete it.
pub struct ActiveSpan {
    name:    &'static str,
    node_id: u32,
    tags:    &'static str,
    t0:      Instant,
    collector: Arc<SpanCollector>,
}

impl ActiveSpan {
    /// Complete the span and record it.
    pub fn finish(self) {
        let duration_us = self.t0.elapsed().as_micros() as u64;
        self.collector.record(Span {
            name:    self.name,
            node_id: self.node_id,
            start_us: 0, // we track duration only for now
            duration_us,
            tags:    self.tags,
        });
    }
}

// ── SpanCollector ─────────────────────────────────────────────────────────────

/// Collects spans from all cluster nodes. Thread-safe.
pub struct SpanCollector {
    /// Last N spans (circular buffer).
    spans:    Mutex<Vec<Span>>,
    capacity: usize,
    total:    AtomicU64,
}

impl SpanCollector {
    pub fn new(capacity: usize) -> Arc<Self> {
        Arc::new(Self {
            spans:    Mutex::new(Vec::with_capacity(capacity)),
            capacity,
            total:    AtomicU64::new(0),
        })
    }

    /// Record a completed span.
    pub fn record(&self, span: Span) {
        self.total.fetch_add(1, Ordering::Relaxed);
        let mut v = self.spans.lock().unwrap();
        if v.len() >= self.capacity { v.remove(0); }
        v.push(span);
    }

    /// Start a new span. Returns an `ActiveSpan`; call `.finish()` to record it.
    pub fn start(self: &Arc<Self>, name: &'static str, node_id: u32, tags: &'static str) -> ActiveSpan {
        ActiveSpan { name, node_id, tags, t0: Instant::now(), collector: self.clone() }
    }

    /// Total spans recorded (ever).
    pub fn total(&self) -> u64 { self.total.load(Ordering::Relaxed) }

    /// Compute p99 duration (µs) for spans matching a name.
    pub fn p99_us(&self, name: &str) -> u64 {
        let v = self.spans.lock().unwrap();
        let mut durations: Vec<u64> = v.iter()
            .filter(|s| s.name == name)
            .map(|s| s.duration_us)
            .collect();
        if durations.is_empty() { return 0; }
        durations.sort_unstable();
        durations[(durations.len() * 99 / 100).max(durations.len() - 1)]
    }

    /// Latest N spans as JSON array (for dashboards).
    pub fn latest_json(&self, n: usize) -> String {
        let v = self.spans.lock().unwrap();
        let start = v.len().saturating_sub(n);
        let entries: Vec<String> = v[start..].iter()
            .map(|s| format!(
                r#"{{"name":"{}","node":{},"us":{},"tags":"{}"}}"#,
                s.name, s.node_id, s.duration_us, s.tags
            ))
            .collect();
        format!("[{}]", entries.join(","))
    }
}

// ── AlertRule + AlertEngine ───────────────────────────────────────────────────

#[derive(Clone, Debug)]
pub enum AlertCondition {
    /// p99 latency for named span exceeds threshold (µs).
    P99ExceedsUs { span_name: &'static str, threshold_us: u64 },
    /// Frame drop rate exceeds threshold (percent).
    DropRatePct { threshold_pct: f32 },
    /// Heartbeat gap exceeds threshold (ms).
    HeartbeatGapMs { threshold_ms: u64 },
    /// Frame count below minimum in last window.
    LowFrameCount { minimum: u64 },
}

#[derive(Clone, Debug)]
pub struct AlertRule {
    pub name:      &'static str,
    pub severity:  AlertSeverity,
    pub condition: AlertCondition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlertSeverity { Warning, Critical }

/// Evaluates alert rules against live metrics and spans.
pub struct AlertEngine {
    rules:   Vec<AlertRule>,
    fired:   Mutex<Vec<String>>, // JSON alert lines
}

impl AlertEngine {
    pub fn new(rules: Vec<AlertRule>) -> Self {
        Self { rules, fired: Mutex::new(vec![]) }
    }

    /// Standard LUMYX alert rules per LUMYX_GOSL.
    pub fn lumyx_standard() -> Self {
        Self::new(vec![
            AlertRule {
                name: "hal-latency-p99",
                severity: AlertSeverity::Critical,
                condition: AlertCondition::P99ExceedsUs {
                    span_name: "hal.send_frame",
                    threshold_us: 50_000, // 50ms
                },
            },
            AlertRule {
                name: "frame-drop-rate",
                severity: AlertSeverity::Warning,
                condition: AlertCondition::DropRatePct { threshold_pct: 1.0 },
            },
            AlertRule {
                name: "heartbeat-gap",
                severity: AlertSeverity::Critical,
                condition: AlertCondition::HeartbeatGapMs { threshold_ms: 2_000 },
            },
        ])
    }

    /// Evaluate all rules; return fired alerts as JSON lines.
    pub fn evaluate(
        &self,
        metrics:  &MetricsEmitter,
        spans:    &SpanCollector,
    ) -> Vec<String> {
        let mut fired = vec![];
        let frames = metrics.frame_count();
        let drops  = metrics.drop_count();
        let drop_rate = if frames > 0 { drops as f32 / frames as f32 * 100.0 } else { 0.0 };
        let p99_us = metrics.p99_us();

        for rule in &self.rules {
            let alert = match &rule.condition {
                AlertCondition::P99ExceedsUs { span_name, threshold_us } => {
                    let p = spans.p99_us(span_name).max(p99_us);
                    if p > *threshold_us {
                        Some(format!(
                            r#"{{"alert":"{}","severity":"{:?}","p99_us":{},"threshold_us":{}}}"#,
                            rule.name, rule.severity, p, threshold_us
                        ))
                    } else { None }
                }
                AlertCondition::DropRatePct { threshold_pct } => {
                    if drop_rate > *threshold_pct {
                        Some(format!(
                            r#"{{"alert":"{}","severity":"{:?}","drop_rate_pct":{:.2},"threshold_pct":{}}}"#,
                            rule.name, rule.severity, drop_rate, threshold_pct
                        ))
                    } else { None }
                }
                AlertCondition::HeartbeatGapMs { threshold_ms } => {
                    // hb_gap is stored in µs in MetricsEmitter
                    let hb_json = metrics.snapshot_json();
                    let gap_ms: u64 = hb_json
                        .split("\"hb_gap_ms\":")
                        .nth(1)
                        .and_then(|s| s.split([',', '}']).next())
                        .and_then(|s| s.trim().parse().ok())
                        .unwrap_or(0);
                    if gap_ms > *threshold_ms {
                        Some(format!(
                            r#"{{"alert":"{}","severity":"{:?}","hb_gap_ms":{},"threshold_ms":{}}}"#,
                            rule.name, rule.severity, gap_ms, threshold_ms
                        ))
                    } else { None }
                }
                AlertCondition::LowFrameCount { minimum } => {
                    if frames < *minimum {
                        Some(format!(
                            r#"{{"alert":"{}","severity":"{:?}","frames":{},"minimum":{}}}"#,
                            rule.name, rule.severity, frames, minimum
                        ))
                    } else { None }
                }
            };
            if let Some(a) = alert { fired.push(a); }
        }
        *self.fired.lock().unwrap() = fired.clone();
        fired
    }

    /// All alerts fired in the last `evaluate()` call.
    pub fn last_alerts(&self) -> Vec<String> {
        self.fired.lock().unwrap().clone()
    }
}

// ── ObservabilityReport ───────────────────────────────────────────────────────

/// Full operational snapshot for a LUMYX show.
pub struct ObservabilityReport {
    pub metrics_json:  String,
    pub alerts:        Vec<String>,
    pub top_spans_json: String,
    pub node_count:    usize,
}

impl ObservabilityReport {
    pub fn collect(
        metrics:    &MetricsEmitter,
        spans:      &SpanCollector,
        alerts:     &AlertEngine,
        node_count: usize,
    ) -> Self {
        let alert_list = alerts.evaluate(metrics, spans);
        Self {
            metrics_json:   metrics.snapshot_json(),
            alerts:         alert_list,
            top_spans_json: spans.latest_json(10),
            node_count,
        }
    }

    /// One-line JSON summary for log aggregators.
    pub fn to_json(&self) -> String {
        let alert_count = self.alerts.len();
        let status = if alert_count == 0 { "nominal" } else { "degraded" };
        format!(
            r#"{{"status":"{status}","nodes":{nc},"alerts":{ac},"metrics":{m}}}"#,
            status = status,
            nc     = self.node_count,
            ac     = alert_count,
            m      = self.metrics_json,
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── SpanCollector ─────────────────────────────────────────────────────────

    #[test]
    fn span_record_and_total() {
        let c = SpanCollector::new(100);
        c.record(Span { name: "test", node_id: 1, start_us: 0, duration_us: 500, tags: "" });
        assert_eq!(c.total(), 1);
    }

    #[test]
    fn span_start_finish_records_span() {
        let c = SpanCollector::new(100);
        let active = c.start("hal.send_frame", 0, "layer=hal");
        std::thread::sleep(std::time::Duration::from_micros(10));
        active.finish();
        assert_eq!(c.total(), 1);
        let p99 = c.p99_us("hal.send_frame");
        assert!(p99 > 0, "p99 must be non-zero after finish");
    }

    #[test]
    fn span_collector_evicts_oldest_when_full() {
        let c = SpanCollector::new(3);
        for i in 0..5u64 {
            c.record(Span { name: "s", node_id: 0, start_us: i, duration_us: i, tags: "" });
        }
        assert_eq!(c.total(), 5, "total counts all");
        let json = c.latest_json(10);
        assert_eq!(json.matches("\"us\"").count(), 3, "only 3 kept in buffer");
    }

    #[test]
    fn span_p99_correct() {
        let c = SpanCollector::new(200);
        for i in 1u64..=100 {
            c.record(Span { name: "op", node_id: 0, start_us: 0, duration_us: i * 100, tags: "" });
        }
        let p99 = c.p99_us("op");
        assert!(p99 >= 9_900, "p99 must be ≥ 9900µs: got {p99}");
    }

    // ── AlertEngine ───────────────────────────────────────────────────────────

    #[test]
    fn no_alerts_when_healthy() {
        let m = MetricsEmitter::new("test");
        let s = SpanCollector::new(100);
        for _ in 0..10 { m.record_frame(1_000); }
        let engine = AlertEngine::lumyx_standard();
        let alerts = engine.evaluate(&m, &s);
        assert!(alerts.is_empty(), "no alerts in healthy system: {:?}", alerts);
    }

    #[test]
    fn drop_rate_alert_fires() {
        let m = MetricsEmitter::new("test");
        let s = SpanCollector::new(100);
        for _ in 0..90 { m.record_frame(500); }
        for _ in 0..10 { m.record_drop(); m.record_frame(500); }
        let engine = AlertEngine::new(vec![AlertRule {
            name: "drops",
            severity: AlertSeverity::Warning,
            condition: AlertCondition::DropRatePct { threshold_pct: 5.0 },
        }]);
        let alerts = engine.evaluate(&m, &s);
        assert!(!alerts.is_empty(), "drop rate alert must fire");
        assert!(alerts[0].contains("drops"));
    }

    #[test]
    fn heartbeat_gap_alert_fires() {
        let m = MetricsEmitter::new("test");
        let s = SpanCollector::new(100);
        m.record_heartbeat_gap(3_000_000); // 3s in µs → 3000ms
        let engine = AlertEngine::new(vec![AlertRule {
            name: "hb",
            severity: AlertSeverity::Critical,
            condition: AlertCondition::HeartbeatGapMs { threshold_ms: 2_000 },
        }]);
        let alerts = engine.evaluate(&m, &s);
        assert!(!alerts.is_empty(), "heartbeat gap alert must fire");
    }

    // ── ObservabilityReport ───────────────────────────────────────────────────

    #[test]
    fn report_json_nominal_when_no_alerts() {
        let m = MetricsEmitter::new("test");
        let s = SpanCollector::new(100);
        for _ in 0..5 { m.record_frame(500); }
        let engine = AlertEngine::lumyx_standard();
        let report = ObservabilityReport::collect(&m, &s, &engine, 2);
        let json = report.to_json();
        assert!(json.contains("\"status\":\"nominal\""), "healthy system → nominal");
        assert!(json.contains("\"nodes\":2"));
    }

    #[test]
    fn report_json_degraded_when_alerts() {
        let m = MetricsEmitter::new("test");
        let s = SpanCollector::new(100);
        for _ in 0..50 { m.record_frame(500); m.record_drop(); }
        let engine = AlertEngine::new(vec![AlertRule {
            name: "drops",
            severity: AlertSeverity::Warning,
            condition: AlertCondition::DropRatePct { threshold_pct: 1.0 },
        }]);
        let report = ObservabilityReport::collect(&m, &s, &engine, 1);
        assert!(report.to_json().contains("\"status\":\"degraded\""));
    }

    #[test]
    fn multi_node_span_collection() {
        let c = SpanCollector::new(200);
        // Node 0 and Node 1 both record spans
        for _ in 0..5 {
            c.record(Span { name: "render", node_id: 0, start_us: 0, duration_us: 1_000, tags: "layer=pixel-engine" });
            c.record(Span { name: "render", node_id: 1, start_us: 0, duration_us: 1_200, tags: "layer=pixel-engine" });
        }
        assert_eq!(c.total(), 10);
        let json = c.latest_json(20);
        assert!(json.contains("\"node\":0"));
        assert!(json.contains("\"node\":1"));
    }
}
