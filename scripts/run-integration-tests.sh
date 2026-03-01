#!/usr/bin/env bash
set -euo pipefail

# Integration test runner that sets up a temporary PostgreSQL instance
# with socket authentication and runs the xzar-server integration tests.
#
# Usage: run-integration-tests.sh [CARGO_TEST_ARGS...]
#
# All arguments are passed directly to cargo test.
# The script must be run from the xzar-rust project directory.

PROJECT_ROOT="$PWD"

if [[ ! -f "$PROJECT_ROOT/Cargo.toml" ]]; then
    echo "Error: Cargo.toml not found in $PROJECT_ROOT"
    echo "Please run this script from the xzar-rust project directory."
    exit 1
fi

# Capture arguments for cargo test
CARGO_TEST_ARGS=("$@")

# Create temporary directory for PostgreSQL
PGDATA=$(mktemp -d -t xzar-test-pg.XXXXXX)
SOCKET_DIR="$PGDATA/socket"

cleanup() {
    echo "Cleaning up..."
    if [[ -f "$PGDATA/postmaster.pid" ]]; then
        pg_ctl -D "$PGDATA" stop -m immediate 2>/dev/null || true
    fi
    rm -rf "$PGDATA"
    echo "Done."
}

trap cleanup EXIT

echo "Setting up PostgreSQL in $PGDATA..."

# Initialize the database cluster
initdb -D "$PGDATA" --auth=trust --no-locale --encoding=UTF8 >/dev/null

# Create socket directory after initdb
mkdir -p "$SOCKET_DIR"

# Configure PostgreSQL for socket-only connections
cat >> "$PGDATA/postgresql.conf" <<EOF
listen_addresses = ''
unix_socket_directories = '$SOCKET_DIR'
logging_collector = off
log_destination = 'stderr'
EOF

# Start PostgreSQL
echo "Starting PostgreSQL..."
pg_ctl -D "$PGDATA" -l "$PGDATA/postgres.log" -o "-k $SOCKET_DIR" start >/dev/null

# Wait for PostgreSQL to be ready
attempts=0
while [ $attempts -lt 30 ]; do
    if pg_isready -h "$SOCKET_DIR" -q; then
        break
    fi
    attempts=$((attempts + 1))
    sleep 0.1
done

if ! pg_isready -h "$SOCKET_DIR" -q; then
    echo "PostgreSQL failed to start. Log:"
    cat "$PGDATA/postgres.log"
    exit 1
fi

echo "PostgreSQL is ready."

# Create test database
createdb -h "$SOCKET_DIR" xzar_test

# Set DATABASE_URL for the tests
export DATABASE_URL="postgres://?host=$SOCKET_DIR&dbname=xzar_test"

echo "Running integration tests..."
echo "DATABASE_URL=$DATABASE_URL"
echo

cd "$PROJECT_ROOT"

# Run the tests
# --test-threads=1 ensures tests don't interfere with each other
cargo test -p xzar-server --test integration_test --features test_harness -- --test-threads=1 "${CARGO_TEST_ARGS[@]}"

echo
echo "All tests passed!"
