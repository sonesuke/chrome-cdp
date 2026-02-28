# chrome-cdp

A Rust library for interacting with Chrome via the Chrome DevTools Protocol (CDP).

## Features

- **Browser Management**: Launch and manage Chrome/Chromium processes with CDP enabled
- **Page Automation**: Navigate pages, evaluate JavaScript, wait for elements
- **WebSocket Connection**: Direct WebSocket communication with CDP
- **Error Handling**: Comprehensive error types for debugging
- **Auto-cleanup**: Browser manager with inactivity-based cleanup

## Usage

```rust
use chrome_cdp::{BrowserManager, CdpPage};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create a browser manager
    let manager = BrowserManager::new(
        None,  // Use default Chrome path
        true,  // headless mode
        false, // debug mode
        vec![], // additional Chrome args
    );

    // Get or launch browser
    let browser = manager.get_browser().await?;

    // Create a new page
    let ws_url = browser.new_page().await?;
    let page = CdpPage::new(&ws_url).await?;

    // Navigate to a URL
    page.goto("https://example.com").await?;

    // Evaluate JavaScript
    let title = page.evaluate("document.title").await?;
    println!("Page title: {}", title);

    Ok(())
}
```

## Installation

Add this to your `Cargo.toml`:

```toml
[dependencies]
chrome-cdp = "0.1"
```

## Configuration

### Environment Variables

- `CHROME_BIN` - Path to Chrome/Chromium executable (optional)
- `CI` - Automatically detected to add sandbox-disabling flags

### Chrome Args

Additional Chrome arguments can be passed via `BrowserManager::new()`:

```rust
let manager = BrowserManager::new(
    None,
    true,
    false,
    vec!["--disable-gpu".to_string(), "--window-size=1920,1080".to_string()],
);
```

## Documentation

See [AGENTS.md](./AGENTS.md) for development guidelines.

## Development

### Prerequisites

- Rust toolchain (latest stable)
- [mise](https://mise.jdx.dev/) for task management and git hooks

### Running Tasks

```bash
mise run test          # Run tests
mise run clippy        # Run clippy with auto-fix
mise run fmt           # Format code
mise run lint          # Run all linters with auto-fix
mise run check         # Run cargo check
```

### Git Hooks

Pre-commit hook automatically fixes formatting and clippy issues:

```bash
mise generate git-pre-commit  # Install the hook
```

The hook runs: `fmt` → `clippy` → `test`

Skip temporarily: `git commit --no-verify`

### CI

GitHub Actions runs the same checks on push/PR:
- Format check (`cargo fmt -- --check`)
- Clippy (`cargo clippy -- -D warnings`)
- Tests (`cargo test`)

## License

MIT
