#!/usr/bin/env bash
# Start, stop or inspect a local polkajam dev testnet (6 validators + 1 RPC node).
#
#   scripts/testnet.sh start    # RPC on ws://localhost:19800
#   scripts/testnet.sh stop
#   scripts/testnet.sh status
#
# The PVM runs with the interpreter backend, which needs no special kernel settings. For
# the faster compiler backend run `sudo sysctl -w vm.unprivileged_userfaultfd=1` once and
# export POLKAVM_BACKEND=compiler.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
POLKAJAM_DIR="${POLKAJAM_DIR:-$ROOT/../tools/polkajam-v0.8.0-linux-x86_64}"
RUN="${JAMX_RUN:-$ROOT/../run}"
mkdir -p "$RUN"
PIDFILE="$RUN/testnet.pid"

running() { [ -f "$PIDFILE" ] && kill -0 "$(cat "$PIDFILE")" 2>/dev/null; }
rpc_up() { (exec 3<>/dev/tcp/127.0.0.1/19800) 2>/dev/null; }

case "${1:-status}" in
start)
    if running; then echo "testnet already running (pid $(cat "$PIDFILE"))"; exit 0; fi
    if rpc_up; then echo "something else is already listening on port 19800"; exit 1; fi
    cd "$RUN"
    POLKAVM_BACKEND="${POLKAVM_BACKEND:-interpreter}" setsid nohup \
        "$POLKAJAM_DIR/polkajam-testnet" > "$RUN/testnet.log" 2>&1 < /dev/null &
    echo $! > "$PIDFILE"
    for _ in $(seq 1 60); do rpc_up && break; sleep 1; done
    rpc_up || { echo "the RPC did not come up; see $RUN/testnet.log"; exit 1; }
    echo "testnet running (pid $(cat "$PIDFILE")), RPC ws://localhost:19800, log $RUN/testnet.log"
    ;;
stop)
    if running; then
        kill -TERM -- "-$(cat "$PIDFILE")" 2>/dev/null || kill "$(cat "$PIDFILE")" 2>/dev/null || true
        sleep 1
        pkill -f "$POLKAJAM_DIR/polkajam" 2>/dev/null || true
        echo "testnet stopped"
    else
        echo "testnet not running"
    fi
    rm -f "$PIDFILE"
    ;;
status)
    if running; then echo "running (pid $(cat "$PIDFILE"))"; else echo "not running"; fi
    rpc_up && echo "RPC port 19800 is open" || echo "RPC port 19800 is closed"
    ;;
*) echo "usage: $0 start|stop|status"; exit 2 ;;
esac
