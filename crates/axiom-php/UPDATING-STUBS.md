# Updating PHP Runtime Stubs

The canonical updater command is:

```powershell
cargo run --locked -p axiom-php --bin axiom-php-stub-updater
```

The command is manual and may access the network. No Python installation is required.
Normal Cargo builds and tests remain offline.
The updater reads `phpstorm-stubs.lock`, downloads the pinned archive, validates SHA-256, extracts
to a temporary directory outside the repository, and atomically replaces `embedded/runtime-stubs.json`.
