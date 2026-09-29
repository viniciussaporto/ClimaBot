#!/usr/bin/env bash
# Installs the `climabot` management command and its aliases, and sets up the
# compose project for the JavaScript bot. Safe to run again.
#
# Usage (as root, from the Rust checkout): ./deploy/install.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
JS_STACK_DIR=${CLIMABOT_JS_STACK_DIR:-/root/climabot-js}
RC=${CLIMABOT_ALIAS_FILE:-/root/.bashrc}

install -m 0755 "$here/climabot" /usr/local/bin/climabot
echo "Installed /usr/local/bin/climabot"

install -d -m 0755 "$JS_STACK_DIR"
install -m 0644 "$here/javascript/docker-compose.yml" "$JS_STACK_DIR/docker-compose.yml"
if [[ ! -f "$JS_STACK_DIR/.env" ]]; then
    install -m 0600 /dev/null "$JS_STACK_DIR/.env"
    echo "Created $JS_STACK_DIR/.env: fill in TOKEN, CLIENT_ID and OPENCAGEAPIKEY"
fi
echo "Installed $JS_STACK_DIR/docker-compose.yml"

marker="# climabot aliases"
if ! grep -qF "$marker" "$RC" 2>/dev/null; then
    cat >>"$RC" <<'EOF'

# climabot aliases
alias climabot-start-rust='climabot start-rust'
alias climabot-start-javascript='climabot start-javascript'
alias climabot-update='climabot update'
alias climabot-stop='climabot stop'
alias climabot-status='climabot status'
EOF
    echo "Added aliases to $RC (open a new shell or run: . $RC)"
fi
