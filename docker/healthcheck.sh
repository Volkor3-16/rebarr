#!/bin/sh
set -eu

# X display should be up
if [ ! -S /tmp/.X11-unix/X99 ]; then
  echo "xvfb socket missing"
  exit 1
fi

# noVNC/websockify should serve vnc.html
if ! wget -q -T 3 -O - http://127.0.0.1:16080/vnc.html >/dev/null 2>&1; then
  echo "novnc unavailable"
  exit 1
fi

# Rebarr should answer its system endpoint
if ! wget -q -T 3 -O - http://127.0.0.1:18000/api/system >/dev/null 2>&1; then
  echo "rebarr unavailable"
  exit 1
fi
