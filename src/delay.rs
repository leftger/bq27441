//! Optional delay hooks for config-mode polling on `no_std` targets.

/// Blocking delay used while polling config-mode flags.
pub trait DelayMs {
    /// Block for at least `ms` milliseconds.
    fn delay_ms(&mut self, ms: u32);
}

/// Poll immediately without inserting delays between attempts.
#[derive(Debug, Clone, Copy, Default)]
pub struct BusyWait;

impl DelayMs for BusyWait {
    fn delay_ms(&mut self, _ms: u32) {}
}

#[cfg(feature = "async")]
/// Async delay used while polling config-mode flags.
pub trait DelayMsAsync {
    /// Await at least `ms` milliseconds.
    fn delay_ms(&mut self, ms: u32) -> impl core::future::Future<Output = ()> + '_;
}

#[cfg(feature = "async")]
impl DelayMsAsync for BusyWait {
    fn delay_ms(&mut self, _ms: u32) -> impl core::future::Future<Output = ()> + '_ {
        async move {}
    }
}
