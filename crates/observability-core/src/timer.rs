//! Gated RAII timer: clock read only pays when gate is on.

use std::time::Instant;

use crate::gate::metrics_enabled;

/// RAII latency timer. Records elapsed time into `metric`'s histogram on drop, in seconds,
/// the base unit Prometheus and OpenTelemetry expect; nanoseconds stay inside the hdr path.
/// `None` variant means gate was off at start: no clock read, no record on drop.
pub struct Timer(Option<(Instant, &'static str)>);

/// Start a timer for `metric` iff [`metrics_enabled`], else skip the clock read entirely.
#[inline]
pub fn timer(metric: &'static str) -> Timer {
    if metrics_enabled() {
        Timer(Some((Instant::now(), metric)))
    } else {
        Timer(None)
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some((start, metric)) = self.0 {
            metrics::histogram!(metric).record(start.elapsed().as_secs_f64());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::set_metrics_enabled;

    /// Timer skips clock read when gate off, captures start + metric name when on.
    // @lat: [[tests#Runtime gate#Timer skips clock when gate off, captures when on]]
    #[test]
    fn timer_skips_clock_off_gate_captures_on_gate() {
        let _state = crate::test_state();
        set_metrics_enabled(false);
        assert!(
            timer("x").0.is_none(),
            "off gate: no clock read, no capture"
        );

        set_metrics_enabled(true);
        assert!(
            timer("x").0.is_some(),
            "on gate: start instant + metric name captured"
        );

        set_metrics_enabled(false);
    }
}
