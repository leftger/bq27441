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
///
/// This crate is `no_std` and single-threaded by nature, so the missing
/// `Send` bound that rustc warns about for `async fn` in public traits
/// doesn't apply here.
#[allow(async_fn_in_trait)]
pub trait DelayMsAsync {
    /// Await at least `ms` milliseconds.
    async fn delay_ms(&mut self, ms: u32);
}

#[cfg(feature = "async")]
impl DelayMsAsync for BusyWait {
    async fn delay_ms(&mut self, _ms: u32) {}
}
