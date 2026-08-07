# Audido

![crates.io](https://img.shields.io/crates/v/audido-tui.svg)
![docs.rs](https://docs.rs/audido-tui/badge.svg)
![license](https://img.shields.io/badge/license-GPL--3.0-blue.svg)
![rust](https://img.shields.io/badge/rust-1.70%2B-orange.svg)

Audido is a terminal-based audio player (TUI) written in Rust. It provides a local audio player, queue management, and a foundation for DSP-based audio processing.

**Key Features**
- Local audio playback
- Queue management
- Browse local files from the TUI
- Extensible DSP pipeline (EQ, normalization, pitch shifting, etc.)

## Install

Prerequisites:
- Rust toolchain (recommended via `rustup`)
- `cargo` available on PATH

Clone the repository:

```bash
git clone https://github.com/haidarptrw/audido.git
cd audido
```

Install the Rust toolchain (if needed):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup install stable
rustup default stable
```

## Build

Build the workspace (release):

```bash
cargo build --workspace --release
```

Build just the TUI binary (debug):

```bash
cargo build -p audido-tui
```

## Run

Run the TUI in debug mode:

```bash
cargo run -p audido-tui
```

Run the release binary:

```bash
cargo run --release -p audido-tui
# or run the built binary directly
./target/release/audido-tui
```

## Development

- To iterate quickly use `cargo run -p audido-tui`.
- Use `cargo test` to run tests for workspace crates (if any).
- The `audido-core` crate contains the DSP and audio engine code.

## Configuration & Notes

- The project uses a workspace layout. The main interactive binary lives in the `audido-tui` crate.
- License: GPL-3.0-or-later (see [LICENSE](LICENSE))

## Contributors

Thanks to everyone who contributed. If your name or avatar is missing, open a PR to add yourself.

 - **haidarptrw** — https://github.com/haidarptrw  

If you want to add more contributors automatically, run:

```bash
git shortlog -sne --all
```

## Contributing

Contributions welcome — please open issues or PRs. For large changes, open an issue first to discuss the approach.

## License

This project is licensed under the GNU General Public License v3 (GPL-3.0-or-later) — see the [LICENSE](LICENSE) file for details.
