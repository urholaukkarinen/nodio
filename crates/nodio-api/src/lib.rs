#![deny(clippy::all)]
use nodio_core::Context;
use parking_lot::RwLock;
use std::sync::Arc;

#[cfg(target_os = "windows")]
use nodio_win32::Win32Context as PlatformContext;

pub fn create_nodio_context() -> Arc<RwLock<dyn Context>> {
    PlatformContext::new()
}
