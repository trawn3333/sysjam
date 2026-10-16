#!/bin/bash
# Screenshot the TUI in tmux and render it to docs/styles/NAME.png (needs tmux, python3-pil, fonttools;
# set SYSJAM_EMOJI_FONT to a monochrome Noto Emoji TTF for the emoji glyphs).
# usage: tools/shot.sh NAME WAIT_SECONDS "sysjam args" [tmux keys | sleep:S ...]
#   tools/shot.sh country 55 "--seed 3 --style country"
#   tools/shot.sh tui 3 "--seed 3 --style country" sleep:38 Up sleep:0.5 Up sleep:2 Left
# LOAD=75 adds synthetic CPU/disk activity for that many seconds so the band wakes up.
name=$1; wait=$2; args=$3; shift 3
s=shot_$name
if [ -n "$LOAD" ]; then
  for i in 1 2 3 4 5; do timeout "$LOAD" sh -c 'while :; do :; done' & done
  timeout "$LOAD" sh -c 'while :; do dd if=/dev/zero of=/tmp/sysjam-load bs=1M count=64 conv=fsync 2>/dev/null; sleep 1; done' &
fi
tmux kill-session -t "$s" 2>/dev/null
tmux -f /dev/null new-session -d -s "$s" -x ${COLS:-140} -y ${ROWS:-46} "env -u NO_COLOR TERM=xterm-256color COLORTERM=truecolor ./target/release/sysjam --null $args; sleep 100"
sleep 2
for k in "$@"; do
  case $k in sleep:*) sleep "${k#sleep:}";; *) tmux send-keys -t "$s" "$k";; esac
done
sleep "$wait"
mkdir -p docs/styles
tmux capture-pane -t "$s" -p -e -N > "/tmp/$name.ans"
tmux capture-pane -t "$s" -p > "/tmp/$name.txt"
tmux send-keys -t "$s" q; sleep 0.5; tmux send-keys -t "$s" q
tmux kill-session -t "$s" 2>/dev/null
python3 tools/ansi2png.py "/tmp/$name.ans" "docs/styles/$name.png" ${COLS:-140} ${ROWS:-46}
wait
rm -f /tmp/sysjam-load
