//! Per-thread periodic sampler for hot-path call sites.

use std::cell::Cell;

thread_local! {
    static SAMPLE_CTR: Cell<u64> = const { Cell::new(0) };
}

/// True exactly once per `mask + 1` calls. `mask` must be `2^n - 1` (e.g. [`SAMPLE_1_IN_8192`]).
/// Checks the pre-increment counter value, so the very first call on a thread always samples.
#[inline]
pub fn should_sample(mask: u64) -> bool {
    SAMPLE_CTR.with(|c| {
        let v = c.get();
        c.set(v.wrapping_add(1));
        v & mask == 0
    })
}

/// Mask for 1-in-8192 sampling: pass to [`should_sample`].
pub const SAMPLE_1_IN_8192: u64 = 8192 - 1;

#[cfg(test)]
mod tests {
    use super::*;

    /// should_sample: fires on fresh thread's first call, again one period later,
    /// exactly once per period-length run of consecutive calls, whatever the start offset.
    // @lat: [[tests#Sampling#Fires on first call and every period after]]
    #[test]
    fn should_sample_fires_first_call_and_every_period_after() {
        const MASK: u64 = SAMPLE_1_IN_8192;
        const PERIOD: u64 = MASK + 1;

        std::thread::spawn(|| {
            assert!(
                should_sample(MASK),
                "first call on fresh thread must sample"
            );

            let stray = (0..PERIOD - 1).filter(|_| should_sample(MASK)).count();
            assert_eq!(stray, 0, "no sample strictly between period boundaries");

            assert!(
                should_sample(MASK),
                "sample fires again exactly one period later"
            );

            // any PERIOD consecutive calls cross exactly one boundary
            let hits = (0..PERIOD).filter(|_| should_sample(MASK)).count();
            assert_eq!(hits, 1, "exactly one sample per PERIOD consecutive calls");
        })
        .join()
        .unwrap();
    }
}
