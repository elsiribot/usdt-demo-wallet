# USDT Demo Wallet — dev tasks. Run inside `nix develop` (or with direnv).

# Build the wasm bundle into dist/
build:
    trunk build

# Release-optimized bundle
build-release:
    trunk build --release

# Local dev server with autoreload
serve:
    trunk serve

# Type-check the wasm target without producing a bundle
check:
    cargo check --target wasm32-unknown-unknown --bin usdt-wallet-web

# Lint
clippy:
    cargo clippy --target wasm32-unknown-unknown --bin usdt-wallet-web -- -D warnings

# Format
fmt:
    cargo fmt

# Remove build artifacts
clean:
    cargo clean
    rm -rf dist
