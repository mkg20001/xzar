# xzar

A pinning-based Nix binary cache server with web UI.

## Action for github actions

```
  - uses: mkg20001/xzar/action@main
    with:
      install-nix: true  # only if Nix isn't already available
```

## Server

### Configuration

Copy the example configuration and edit it:

```bash
cp config.example.yaml config.yaml
```

Update `config.yaml` with your settings:

- `db.connection` - PostgreSQL connection string (e.g., `postgres://user@/dbname?host=/run/postgresql` for socket auth)
- `storage` - Path to store NAR files
- `tokens` - Upload tokens (use `hashed` for SHA-512 hashed tokens or `plain` for plaintext)
- `signingKey` - Generate with `nix-store --generate-binary-cache-key KEY-NAME priv.pem pub.pem`

### Running

```bash
# Build and run
nix develop --command cargo watch -- cargo run --bin xzar-server

# Or with a custom config path
XZAR_CONFIG=/path/to/config.yaml nix develop --command cargo watch -- cargo run --bin xzar-server
```

## Web UI

The web UI allows managing pins through a browser interface.

### Building

```bash
cd ui

# Install dependencies
npm install

# Build TailwindCSS (one-time)
npm run tailwind

# Or watch for changes during development
npm run tailwind:watch
```

### Running

```bash
# Install dioxus-cli if needed
cargo install dioxus-cli

# Run the development server
cd ui && dx serve
```

The UI will be available at `http://localhost:8080`.

## Development

Use `nix develop --command` for all cargo commands (provides PostgreSQL libs and other dependencies).

### Testing

Run integration tests with PostgreSQL:

```bash
nix run .#integration-tests
```

This sets up a temporary PostgreSQL instance and runs all interop tests.
