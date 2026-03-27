#!/bin/sh
set -e

# Always refresh bundled providers into the data volume. Extra user-created files are left alone.
mkdir -p /data/providers
echo "[entrypoint] Syncing bundled providers into /data/providers..."
cp /app/providers/*.yaml /data/providers/ 2>/dev/null || true
echo "[entrypoint] Done. $(find /data/providers/ -maxdepth 1 -type f -name '*.yaml' 2>/dev/null | wc -l) provider(s) present."

# Runtime dirs used by nginx when running as non-root.
mkdir -p /tmp/nginx/client_temp /tmp/nginx/proxy_temp /tmp/nginx/fastcgi_temp /tmp/nginx/uwsgi_temp /tmp/nginx/scgi_temp

# Hand off to supervisor (starts Xvfb, VNC/noVNC, rebarr, nginx)
exec /usr/bin/supervisord -c /etc/supervisord.conf
