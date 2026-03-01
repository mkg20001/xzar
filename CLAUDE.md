# Development

Use `nix develop --command` for all cargo commands (provides PostgreSQL libs and other dependencies).

# Testing

Run integration tests with PostgreSQL:
```
nix run .#integration-tests
```

This sets up a temporary PostgreSQL instance and runs all interop tests.
