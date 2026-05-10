#!/usr/bin/env bash

set -euo pipefail

python3 tool/audio_test_server.py --port 8765 --directory test-assets &
server_pid="$!"
trap 'kill "$server_pid" 2>/dev/null || true' EXIT

server_ready=0
for _ in {1..20}; do
  if curl --fail --silent --output /dev/null http://127.0.0.1:8765/generated/long-30s.wav; then
    server_ready=1
    break
  fi
  sleep 1
done

if [[ "$server_ready" -ne 1 ]]; then
  echo "Audio test server did not become ready"
  exit 1
fi

adb reverse tcp:8765 tcp:8765

PATH="$PATH:$HOME/.pub-cache/bin" \
CI=true \
patrol test \
  --target patrol_test/native_audio_controls_test.dart \
  --device emulator-5554 \
  --no-label \
  --show-flutter-logs
