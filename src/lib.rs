//! # chrome-cdp
//!
//! A Rust library for interacting with Chrome via DevTools Protocol.

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
