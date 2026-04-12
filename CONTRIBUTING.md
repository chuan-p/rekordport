# Contributing

Thanks for taking a look at `rekordport`.

## Before Opening A PR

- Keep changes focused and easy to review.
- If a change affects conversion behavior, include a short note about expected user impact.
- Prefer adding or updating tests when touching the Rust conversion path.

## Local Setup

```bash
npm install
```

Run the desktop app:

```bash
npm run tauri dev
```

Run checks:

```bash
npm run check
```

Run Rust tests directly:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

## Notes

- The Tauri app is the main product surface.
- The Python script is still useful as a CLI scanner and reference implementation.
- Sidecars are optional in development if `sqlcipher` and `ffmpeg` are already available in `PATH`.
