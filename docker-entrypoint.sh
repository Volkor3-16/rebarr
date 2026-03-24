#!/bin/sh
set -e

# First-run setup: seed default providers if the volume directory is empty
if [ ! -d "/data/providers" ] || [ -z "$(ls -A /data/providers/ 2>/dev/null)" ]; then
    echo "[entrypoint] No providers found in /data/providers — seeding defaults..."
    mkdir -p /data/providers
    cp /app/providers/*.yaml /data/providers/ 2>/dev/null || true
    echo "[entrypoint] Done. $(find /data/providers/ -maxdepth 1 -type f -name '*.yaml' 2>/dev/null | wc -l) provider(s) installed."
else
    echo "[entrypoint] Providers directory already populated — skipping seed."
fi

# Runtime dirs used by nginx when running as non-root.
mkdir -p /tmp/nginx/client_temp /tmp/nginx/proxy_temp /tmp/nginx/fastcgi_temp /tmp/nginx/uwsgi_temp /tmp/nginx/scgi_temp

# Hand off to supervisor (starts Xvfb, VNC/noVNC, rebarr, nginx)
exec /usr/bin/supervisord -c /etc/supervisord.conf
