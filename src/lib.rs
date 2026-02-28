//! # chrome-cdp
//!
//! A Rust library for interacting with Chrome via DevTools Protocol.

mod browser;
mod connection;
mod error;
mod page;

pub use browser::{BrowserManager, CdpBrowser};
pub use connection::CdpConnection;
pub use error::{Error, Result};
pub use page::CdpPage;

/// Returns the library version
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_returns_version() {
        assert_eq!(version(), "0.1.0");
    }
}
