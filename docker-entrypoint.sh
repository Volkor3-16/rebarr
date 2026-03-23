#!/bin/sh
set -e

# First-run setup: seed default providers if the volume directory is empty
if [ ! -d "/data/providers" ] || [ -z "$(find /data/providers/ -maxdepth 1 -type f -name '*.yaml' 2>/dev/null)" ]; then
    echo "[entrypoint] No providers found in /data/providers — seeding defaults..."
    mkdir -p /data/providers
    cp /app/providers/*.yaml /data/providers/ 2>/dev/null || true
    echo "[entrypoint] Done. $(find /data/providers/ -maxdepth 1 -type f -name '*.yaml' 2>/dev/null | wc -l) provider(s) installed."
else
    echo "[entrypoint] Providers directory already populated — skipping seed."
fi

# Hand off to the rebarr binary
exec ./rebarr "$@"