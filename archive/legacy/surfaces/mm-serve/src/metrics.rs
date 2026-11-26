//! In-process Prometheus metrics.
//!
//! The console has no metrics-server dependency; this module keeps the small
//! set of counters and latency histograms the inspector needs and renders the
//! Prometheus text exposition directly. All names are stable operator API.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

const BUCKETS_MS: [u64; 8] = [5, 10, 25, 50, 100, 250, 500, 1_000];

#[derive(Debug, Default)]
struct Histogram {
    buckets: Mutex<[u64; BUCKETS_MS.len() + 1]>,
    sum: AtomicU64,
    count: AtomicU64,
}

impl Histogram {
    fn observe(&self, value_ms: u64) {
        self.sum.fetch_add(value_ms, Ordering::Relaxed);
        self.count.fetch_add(1, Ordering::Relaxed);
        let mut buckets = self.buckets.lock().expect("metrics histogram lock");
        for (index, bound) in BUCKETS_MS.iter().enumerate() {
            if value_ms <= *bound {
                buckets[index] = buckets[index].saturating_add(1);
            }
        }
        buckets[BUCKETS_MS.len()] = buckets[BUCKETS_MS.len()].saturating_add(1);
    }

    fn render(&self, name: &str, help: &str, out: &mut String) {
        out.push_str(&format!("# HELP {name} {help}\n"));
        out.push_str(&format!("# TYPE {name} histogram\n"));
        let buckets = self.buckets.lock().expect("metrics histogram lock");
        for (index, bound) in BUCKETS_MS.iter().enumerate() {
            out.push_str(&format!(
                "{name}_bucket{{le=\"{bound}\"}} {}\n",
                buckets[index]
            ));
        }
        out.push_str(&format!(
            "{name}_bucket{{le=\"+Inf\"}} {}\n",
            buckets[BUCKETS_MS.len()]
        ));
        out.push_str(&format!(
            "{name}_sum {}\n{name}_count {}\n",
            self.sum.load(Ordering::Relaxed),
            self.count.load(Ordering::Relaxed)
        ));
    }
}

/// Process-wide counters.
#[derive(Debug, Default)]
pub struct Metrics {
    http_requests_total: AtomicU64,
    http_errors_total: AtomicU64,
    http_latency_ms: Histogram,
    status_frames_total: AtomicU64,
    status_recompute_ms: Histogram,
    prompt_unavailable_total: AtomicU64,
    budget_denied_total: AtomicU64,
    automation_emissions_total: AtomicU64,
    plan_external_starts_total: AtomicU64,
    plan_failures_total: AtomicU64,
}

impl Metrics {
    /// Record one HTTP response and its wall time.
    pub fn record_request(&self, status: u16, elapsed_ms: u64) {
        self.http_requests_total.fetch_add(1, Ordering::Relaxed);
        if status >= 500 {
            self.http_errors_total.fetch_add(1, Ordering::Relaxed);
        }
        self.http_latency_ms.observe(elapsed_ms);
    }

    /// Record one published status revision and its recompute time.
    pub fn record_status(&self, elapsed_ms: u64) {
        self.status_frames_total.fetch_add(1, Ordering::Relaxed);
        self.status_recompute_ms.observe(elapsed_ms);
    }

    /// Record a fail-closed prompt build.
    pub fn prompt_unavailable(&self) {
        self.prompt_unavailable_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record a budget denial.
    pub fn budget_denied(&self) {
        self.budget_denied_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one delivered automation utterance.
    pub fn automation_emission(&self) {
        self.automation_emissions_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one plan-monitor external start.
    pub fn plan_external_start(&self) {
        self.plan_external_starts_total
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record one failed plan step.
    pub fn plan_failure(&self) {
        self.plan_failures_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Render the Prometheus text exposition.
    pub fn prometheus(&self) -> String {
        let mut out = String::new();
        out.push_str("# HELP metamind_http_requests_total HTTP responses served.\n");
        out.push_str("# TYPE metamind_http_requests_total counter\n");
        out.push_str(&format!(
            "metamind_http_requests_total {}\n",
            self.http_requests_total.load(Ordering::Relaxed)
        ));
        out.push_str("# HELP metamind_http_errors_total HTTP 5xx responses.\n");
        out.push_str("# TYPE metamind_http_errors_total counter\n");
        out.push_str(&format!(
            "metamind_http_errors_total {}\n",
            self.http_errors_total.load(Ordering::Relaxed)
        ));
        self.http_latency_ms.render(
            "metamind_http_request_duration_ms",
            "HTTP request wall time.",
            &mut out,
        );
        out.push_str("# HELP metamind_status_frames_total Published status revisions.\n");
        out.push_str("# TYPE metamind_status_frames_total counter\n");
        out.push_str(&format!(
            "metamind_status_frames_total {}\n",
            self.status_frames_total.load(Ordering::Relaxed)
        ));
        self.status_recompute_ms.render(
            "metamind_status_recompute_duration_ms",
            "Status view recompute time.",
            &mut out,
        );
        for (name, help, value) in [
            (
                "metamind_prompt_unavailable_total",
                "Fail-closed prompt builds.",
                self.prompt_unavailable_total.load(Ordering::Relaxed),
            ),
            (
                "metamind_budget_denied_total",
                "Budget reservations denied.",
                self.budget_denied_total.load(Ordering::Relaxed),
            ),
            (
                "metamind_automation_emissions_total",
                "Automation utterances delivered.",
                self.automation_emissions_total.load(Ordering::Relaxed),
            ),
            (
                "metamind_plan_external_starts_total",
                "Plan-monitor external runs started.",
                self.plan_external_starts_total.load(Ordering::Relaxed),
            ),
            (
                "metamind_plan_failures_total",
                "Plan-monitor step failures.",
                self.plan_failures_total.load(Ordering::Relaxed),
            ),
        ] {
            out.push_str(&format!("# HELP {name} {help}\n# TYPE {name} counter\n"));
            out.push_str(&format!("{name} {value}\n"));
        }
        out
    }
}
