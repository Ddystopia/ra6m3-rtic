use crate::socket::PicoRuntime;

pub struct Timer;

/// picoserve 0.18 drives its timeouts through `Timer<Runtime>` using
/// `picoserve::time::Duration` (milliseconds). We back it with embassy-time,
/// which rides the same SysTick as the rtic monotonic — mirroring picoserve's
/// own `EmbassyTimer`.
impl picoserve::Timer<PicoRuntime> for Timer {
    async fn delay(&self, duration: picoserve::time::Duration) {
        embassy_time::Timer::after_millis(duration.as_millis()).await;
    }

    async fn run_with_timeout<F: core::future::Future>(
        &self,
        duration: picoserve::time::Duration,
        future: F,
    ) -> Result<F::Output, picoserve::time::TimeoutError> {
        embassy_time::with_timeout(
            embassy_time::Duration::from_millis(duration.as_millis()),
            future,
        )
        .await
        .map_err(|embassy_time::TimeoutError| picoserve::time::TimeoutError)
    }
}
