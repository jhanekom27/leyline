#!/usr/bin/env bash
#
# Drives two real leyline peers plus Bob's received-files observer for
# docs/file-demo.tape. The driver creates a fixture only under Alice's
# isolated HOME, verifies Bob saved the same bytes, and reveals Bob's channel
# folder only after that verified transfer finishes.

set -o errexit
set -o nounset
set -o pipefail

readonly ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
readonly BINARY="$ROOT/target/release/leyline"
readonly SOCKET="leyline-vhs-file-demo"
readonly SESSION="leyline-file-demo"
readonly STATE_DIR="${TMPDIR:-/tmp}/leyline-vhs-file-demo-state"
readonly STATE_FILE="$STATE_DIR/workdir"
DRIVER_PID=""

tmux_cmd() {
  tmux -L "$SOCKET" "$@"
}

stop() {
  tmux_cmd kill-server 2>/dev/null || true
  if [[ -f "$STATE_FILE" ]]; then
    rm -rf "$(cat "$STATE_FILE")"
  fi
  rm -rf "$STATE_DIR"
}

pane_text() {
  tmux_cmd capture-pane -p -J -t "$1"
}

wait_for_pane() {
  local pane="$1"
  local pattern="$2"
  local attempts=0

  until pane_text "$pane" | grep -Eq "$pattern"; do
    attempts=$((attempts + 1))
    if ((attempts == 300)); then
      printf 'timed out waiting for %s in %s\n' "$pattern" "$pane" >&2
      exit 1
    fi
    sleep 0.1
  done
}

send() {
  tmux_cmd send-keys -t "$1" -l "$2"
  tmux_cmd send-keys -t "$1" Enter
}

type_text() {
  local pane="$1"
  local text="$2"
  local index

  for ((index = 0; index < ${#text}; index++)); do
    tmux_cmd send-keys -t "$pane" -l "${text:index:1}"
    sleep 0.08
  done
}

type_and_submit() {
  type_text "$1" "$2"
  sleep 0.7
  tmux_cmd send-keys -t "$1" Enter
}

paste_ticket_and_submit() {
  local pane="$1"
  local ticket="$2"

  type_text "$pane" '/join '
  sleep 0.4
  tmux_cmd send-keys -t "$pane" -l "$ticket"
  sleep 0.6
  tmux_cmd send-keys -t "$pane" Enter
}

clipboard_ticket() {
  if command -v pbpaste >/dev/null; then
    pbpaste
  elif command -v wl-paste >/dev/null; then
    wl-paste --no-newline
  elif command -v xclip >/dev/null; then
    xclip -selection clipboard -o
  else
    printf 'no supported clipboard reader found (need pbpaste, wl-paste, or xclip)\n' >&2
    return 1
  fi
}

clear_clipboard() {
  if command -v pbcopy >/dev/null; then
    printf '' | pbcopy
  elif command -v wl-copy >/dev/null; then
    wl-copy
  elif command -v xclip >/dev/null; then
    xclip -selection clipboard
  fi
}

wait_for_ticket() {
  local attempts=0
  local ticket

  until ticket="$(clipboard_ticket | tr -d '\r\n')" \
    && [[ "$ticket" =~ ^leyline[a-z0-9]+$ ]]; do
    attempts=$((attempts + 1))
    if ((attempts == 300)); then
      printf 'timed out waiting for Alice'\''s invite ticket in the clipboard\n' >&2
      exit 1
    fi
    sleep 0.1
  done
  printf '%s\n' "$ticket"
}

write_observer() {
  local workdir="$1"
  local observer="$workdir/observer.sh"

  cat >"$observer" <<'EOF'
#!/usr/bin/env bash
set -o errexit
set -o nounset
set -o pipefail

readonly DESTINATION="$1"
readonly FIXTURE="demo-note.txt"

clear
printf "Preparing Bob's received-files view...\n"
read -r

clear
printf "Bob's received files (#demo)\n\n"
if [[ -f "$DESTINATION/$FIXTURE" ]]; then
  printf 'unexpectedly found %s before the transfer\n' "$FIXTURE"
else
  printf 'empty: %s is not present yet\n' "$FIXTURE"
fi
printf '\nwaiting for Bob to run /save...\n'
read -r

clear
printf "Bob's received files (#demo)\n\n"
if [[ ! -f "$DESTINATION/$FIXTURE" ]]; then
  printf 'expected %s after the transfer, but it is missing\n' "$FIXTURE" >&2
  exit 1
fi
printf 'saved copy:\n'
ls -lh "$DESTINATION"
exec tail -f /dev/null
EOF
  chmod +x "$observer"
}

drive() {
  local workdir="$1"
  local alice="$SESSION:0.0"
  local bob="$SESSION:0.1"
  local observer="$SESSION:0.2"
  local fixture="$workdir/alice/share/demo-note.txt"
  local destination="$workdir/bob/Downloads/leyline/demo/demo-note.txt"
  local ticket

  tmux_cmd respawn-pane -k -t "$alice" "env HOME=$workdir/alice $BINARY"
  tmux_cmd respawn-pane -k -t "$bob" "env HOME=$workdir/bob $BINARY"
  wait_for_pane "$alice" 'you:'
  wait_for_pane "$bob" 'you:'

  type_and_submit "$bob" '/join demo'
  wait_for_pane "$bob" '#demo'
  type_and_submit "$bob" '/invite'
  ticket="$(wait_for_ticket)"

  paste_ticket_and_submit "$alice" "$ticket"
  wait_for_pane "$alice" '1 peer'
  wait_for_pane "$bob" '1 peer'
  tmux_cmd select-pane -t "$alice" -T Alice
  tmux_cmd select-pane -t "$bob" -T Bob
  type_and_submit "$alice" '/nick Alice'
  type_and_submit "$bob" '/nick Bob'
  send "$alice" '/hints'
  send "$bob" '/hints'
  sleep 1

  tmux_cmd send-keys -t "$observer" Enter
  wait_for_pane "$observer" 'not present yet'

  type_and_submit "$alice" '/send ~/share/demo-note.txt'
  wait_for_pane "$bob" 'demo-note\.txt'
  sleep 1
  type_and_submit "$bob" '/save'
  wait_for_pane "$bob" 'saved demo-note\.txt to'
  cmp -s "$fixture" "$destination"

  tmux_cmd send-keys -t "$observer" Enter
  wait_for_pane "$observer" 'saved copy:'
}

start() {
  local workdir
  local observer
  local observer_command

  if [[ ! -x "$BINARY" ]]; then
    printf 'build the release binary first: cargo build --release\n' >&2
    exit 1
  fi

  stop
  mkdir -p "$STATE_DIR"
  mkdir -p "$ROOT/guide/src/assets"
  clear_clipboard
  workdir="$(mktemp -d "${TMPDIR:-/tmp}/leyline-vhs-file-demo.XXXXXX")"
  printf '%s\n' "$workdir" >"$STATE_FILE"
  mkdir -p "$workdir/alice/share" "$workdir/bob"
  cat >"$workdir/alice/share/demo-note.txt" <<'EOF'
This small note was shared by Alice and saved by Bob.
EOF
  write_observer "$workdir"

  tmux_cmd new-session -d -s "$SESSION" -x 180 -y 42 -n files "env PS1='› ' bash --noprofile --norc"
  tmux_cmd split-window -d -v -l 10 -t "$SESSION:0"
  tmux_cmd split-window -d -h -t "$SESSION:0.0"
  tmux_cmd set-option -t "$SESSION" status off
  tmux_cmd set-option -t "$SESSION" automatic-rename off
  tmux_cmd set-option -t "$SESSION" allow-rename off
  tmux_cmd set-option -t "$SESSION" pane-border-status top
  tmux_cmd set-option -t "$SESSION" pane-border-format ' #{pane_title} '
  tmux_cmd select-pane -t "$SESSION:0.0" -T Alice
  tmux_cmd select-pane -t "$SESSION:0.1" -T Bob
  tmux_cmd select-pane -t "$SESSION:0.2" -T "Bob's received files"
  tmux_cmd send-keys -t "$SESSION:0.0" 'clear; printf "Alice is starting leyline...\n"' Enter
  tmux_cmd send-keys -t "$SESSION:0.1" 'clear; printf "Bob is waiting to join...\n"' Enter
  observer="$workdir/observer.sh"
  printf -v observer_command 'bash %q %q' "$observer" "$workdir/bob/Downloads/leyline/demo"
  tmux_cmd respawn-pane -k -t "$SESSION:0.2" "$observer_command"

  drive "$workdir" >"$workdir/driver.log" 2>&1 &
  DRIVER_PID="$!"
}

render() {
  start
  trap stop EXIT
  cd "$ROOT"
  vhs docs/file-demo.tape
  if ! wait "$DRIVER_PID"; then
    cat "$(cat "$STATE_FILE")/driver.log" >&2
    return 1
  fi
}

case "${1:-}" in
  start) start ;;
  stop) stop ;;
  render) render ;;
  *)
    printf 'usage: %s {start|stop|render}\n' "$0" >&2
    exit 64
    ;;
esac
