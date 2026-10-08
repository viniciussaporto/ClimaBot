#!/usr/bin/env bash
# Installs the `climabot` management command, its shell shortcuts and the
# nightly update timer. Safe to run again.
#
# Usage (as root, from the Rust checkout): ./deploy/install.sh
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
RC=${CLIMABOT_ALIAS_FILE:-/root/.bashrc}

# Copy then rename, so a `climabot` process that is running this very
# update keeps reading its old file.
install -m 0755 "$here/climabot" /usr/local/bin/.climabot.new
mv -f /usr/local/bin/.climabot.new /usr/local/bin/climabot
echo "Installed /usr/local/bin/climabot"

"$here/grafana/install.sh"

# Nightly update timer (05:30 UTC).
if command -v systemctl >/dev/null && [[ -d /run/systemd/system ]]; then
    install -m 0644 "$here/systemd/climabot-update.service" "$here/systemd/climabot-update.timer" /etc/systemd/system/
    systemctl daemon-reload
    systemctl enable --quiet climabot-update.timer
    # Start the timer, but don't restart it while its own update is running.
    systemctl is-active --quiet climabot-update.timer || systemctl start climabot-update.timer
    echo "Installed climabot-update.timer (next: $(systemctl show climabot-update.timer -p NextElapseUSecRealtime --value))"
fi

# Shell shortcuts (climabot-*) and the SSH login banner.
install -m 0644 "$here/shell/climabot.sh" /etc/profile.d/.climabot.sh.new
mv -f /etc/profile.d/.climabot.sh.new /etc/profile.d/climabot.sh
# Non-login interactive shells (e.g. `bash -i`, tmux panes) read /etc/bash.bashrc instead.
line='[ -r /etc/profile.d/climabot.sh ] && . /etc/profile.d/climabot.sh'
grep -qxF "$line" /etc/bash.bashrc || echo "$line" >>/etc/bash.bashrc
# Earlier versions put the aliases in $RC; they now live in the profile script.
if grep -q '^# climabot aliases$' "$RC" 2>/dev/null; then
    sed -i '/^# climabot aliases$/,/^alias climabot-status=/d' "$RC"
    echo "Moved the climabot aliases from $RC to /etc/profile.d/climabot.sh"
fi
echo "Installed /etc/profile.d/climabot.sh (shortcuts + login banner)"
