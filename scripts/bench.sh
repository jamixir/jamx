#!/usr/bin/env bash
# Measure what an auction round costs on a running local testnet.
#
#   scripts/bench.sh [orders-per-side]      (default 10; needs scripts/testnet.sh start)
#
# Deploys a fresh instance built with the `profile` feature, funds 2N traders, submits one
# round of 2N orders and prints the gas spent in each phase of accumulate, as reported by
# the service itself through the node log. The probes themselves cost gas; for the true
# total run with JAMX_FEATURES=logging, which reports only the two totals.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${JAMX_TOOLS:-$ROOT/../tools}"
RUN="${JAMX_RUN:-$ROOT/../run}"
N="${1:-10}"
export JAMT="${JAMT:-$TOOLS/polkajam-v0.8.0-linux-x86_64/jamt}"
export JAMX_DIR="$RUN/.jamx-bench"
export PATH="$ROOT/target/release:$PATH"

(cd "$ROOT" && cargo build --release -q -p jamx-cli)
rm -rf "$JAMX_DIR"
JAMX_FEATURES="${JAMX_FEATURES-profile}" "$ROOT/scripts/build-service.sh" "$ROOT/crates/jamx-service" "$RUN/jamx-profile.jam" \
    --min-stack-size 262144 2>&1 | tail -1
SERVICE=$("$JAMT" create-service --raw --min-item-gas 100000 "$RUN/jamx-profile.jam" 50000000 2>/dev/null | tail -1)
export JAMX_SERVICE=$SERVICE
echo "service $SERVICE; waiting for its code to be finalized"
sleep 24

jamx -q create-token bank QUOTE >/dev/null
jamx -q create-token bank BASE >/dev/null
jamx -q accept bundler 1 >/dev/null
jamx submit >/dev/null
# Fund in chunks: a payload of signed operations is bounded by refine gas (3M per
# signature). Each trader first accepts the asset it is about to be sent.
i=1
while [ "$i" -le "$N" ]; do
    for _ in $(seq 1 25); do
        [ "$i" -le "$N" ] || break
        jamx -q accept "buyer$i" 1 >/dev/null
        jamx -q mint bank 1 "buyer$i" 100000000 >/dev/null
        jamx -q accept "seller$i" 2 >/dev/null
        jamx -q mint bank 2 "seller$i" 1000000 >/dev/null
        i=$((i + 1))
    done
    jamx submit >/dev/null
done
for i in $(seq 1 "$N"); do
    jamx order "buyer$i" buy 2 1 $((50 + i)) "$((1000 + i))/10" >/dev/null
    jamx order "seller$i" sell 2 1 $((40 + i)) "$((900 + i))/10" >/dev/null
done
MARK=$(wc -l < "$RUN/testnet.log")
# A round too large for its gas is refused whole ("out of gas"); report the gas either way.
jamx batch 2 1 --builder bundler --fee-bps 10 || true
echo
echo "gas, as reported by the service (first validator's log):"
tail -n +"$MARK" "$RUN/testnet.log" | grep -E '^node0: .*(profile:|jamx (refine|accumulate))' \
    | sed -E 's/^node0: [0-9-]+ [0-9:.]+ \S+ INFO +\S+ +//' | sed 's/^/  /'
