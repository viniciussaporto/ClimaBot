# /etc/profile.d/climabot.sh - climabot shortcuts + SSH login banner
# Installed by deploy/install.sh (source: deploy/shell/climabot.sh). Edit there, not in /etc.
# Bash-only, interactive-only. Also sourced from /etc/bash.bashrc (non-login shells).
[ -n "${BASH_VERSION:-}" ] || return 0
case $- in *i*) ;; *) return 0 ;; esac
# /etc/profile (profile.d) and /etc/bash.bashrc both source this in a login shell: run once per shell
[ -n "${CB_PROFILE_DONE:-}" ] && return 0
CB_PROFILE_DONE=1

# Single source of truth: name | description | command
CB_CMDS=(
  "climabot-status|Whether the bot is running, and its containers|climabot status"
  "climabot-logs|Follow the bot's logs|climabot logs"
  "climabot-update|Pull, rebuild and restart the bot (also nightly at 05:30 UTC)|climabot update"
  "climabot-start|Pull, rebuild and start the bot|climabot start"
  "climabot-stop|Stop the bot and its stack|climabot stop"
  "climabot-help|Show this list|cb_banner"
)

cb_state() {
  local running
  running=$(docker ps --format '{{.Names}}' 2>/dev/null)
  if grep -qx 'climabot-climabot-1' <<< "$running"; then
    printf '\033[1;32mbot running\033[0m (%s %s)' \
      "$(git -C /root/climabot-rust rev-parse --abbrev-ref HEAD 2>/dev/null || echo '?')" \
      "$(git -C /root/climabot-rust log -1 --format=%h 2>/dev/null || echo '?')"
  else
    printf '\033[1;31mno bot running\033[0m'
  fi
}

cb_banner() {
  local name desc cmd
  printf '\n  \033[1mclimabot ops\033[0m  %s\n\n' "$(cb_state)"
  for entry in "${CB_CMDS[@]}"; do
    IFS='|' read -r name desc cmd <<< "$entry"
    printf '  \033[1;32m%-25s\033[0m %s\n' "$name" "$desc"
  done
  echo
}

for entry in "${CB_CMDS[@]}"; do
  IFS='|' read -r name desc cmd <<< "$entry"
  [ -n "$cmd" ] && alias "$name=$cmd"
done
unset entry name desc cmd

# Banner only on SSH login shells (not every tmux pane / subshell)
if shopt -q login_shell && [ -n "${SSH_CONNECTION:-}" ]; then
  cb_banner
fi
