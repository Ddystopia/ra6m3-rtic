use rtic_monotonics::{Monotonic, fugit::Duration};

pub struct Timer;

impl picoserve::Timer for Timer {
    type Duration = Duration<u32, 1, 1000>;

    type TimeoutError = ();

    async fn run_with_timeout<F: core::future::Future>(
        &mut self,
        duration: Self::Duration,
        future: F,
    ) -> Result<F::Output, Self::TimeoutError> {
        let future = async { Ok(future.await) };
        let delay = async { Err(crate::Mono::delay(duration).await) };
        futures_lite::future::or(future, delay).await
    }
}
