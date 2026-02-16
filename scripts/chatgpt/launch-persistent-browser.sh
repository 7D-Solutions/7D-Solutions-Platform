#!/bin/bash
# Launch persistent browser for worker to connect to

PROFILE_DIR=".browser-profiles/chatgpt-persistent"
CDP_PORT=9222

# Create profile dir
mkdir -p "$PROFILE_DIR"

# Launch Chromium with remote debugging
/Applications/Google\ Chrome.app/Contents/MacOS/Google\ Chrome \
  --remote-debugging-port=$CDP_PORT \
  --user-data-dir="$(pwd)/$PROFILE_DIR" \
  --no-first-run \
  --no-default-browser-check \
  "https://chatgpt.com" &

echo "Browser launched on port $CDP_PORT"
echo "Profile: $PROFILE_DIR"
echo "Please login to ChatGPT in the browser window"
