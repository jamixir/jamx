#!/usr/bin/env bash
# JAMX end to end on a local polkajam testnet.
#
# Deploys a fresh instance of the service and walks through the design of
# "Bitcoin Wasn't Hacked. Binance Was.": a bank that issues a token and nothing else, an
# order book with no operator, bitcoin bought with reais without anyone holding both legs,
# a Pix escrow whose arbiter can reach one trade, accounts with a freeze-only friend and
# social recovery, and an upgrade key that has to announce itself and wait.
#
#   scripts/testnet.sh start && scripts/demo.sh
#
# Takes about ten minutes: every step is a real work package, refined in-core and
# accumulated on-chain by six validators.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TOOLS="${JAMX_TOOLS:-$ROOT/../tools}"
RUN="${JAMX_RUN:-$ROOT/../run}"
export JAMT="${JAMT:-$TOOLS/polkajam-v0.8.0-linux-x86_64/jamt}"
export JAMX_DIR="$RUN/.jamx-demo"
export PATH="$ROOT/target/release:$PATH"

bold() { printf '\n\033[1m== %s ==\033[0m\n' "$*"; }
note() { printf '   %s\n' "$*"; }
# Run a command that must succeed.
run() { printf '\033[2m$ jamx %s\033[0m\n' "$*"; jamx "$@"; }
# Run a command the service must refuse.
refused() {
    printf '\033[2m$ jamx %s\033[0m   (must be refused)\n' "$*"
    if out=$(jamx "$@" 2>&1); then echo "$out"; echo "UNEXPECTED: this should have been refused"; exit 1; fi
    # It must be the service that said no, not a tool that failed.
    if ! grep -q 'accumulated at slot' <<<"$out"; then echo "$out"; echo "UNEXPECTED: the command failed before reaching the service"; exit 1; fi
    grep -v '^error:' <<<"$out" || true
}

bold "Build"
(cd "$ROOT" && cargo build --release -q -p jamx-cli)
rm -rf "$JAMX_DIR"
ADMIN=$(jamx id admin)
# Governance is compiled in: this code hash commits to its administrator and to the delay
# every upgrade must wait (here 10 slots, one minute, so that the demo can show it).
# (`logging` makes the service report its gas use in the node log; leave it out in earnest.)
JAMX_FEATURES="${JAMX_FEATURES-logging}" JAMX_ADMIN=$ADMIN JAMX_UPGRADE_DELAY=10 "$ROOT/scripts/build-service.sh" \
    "$ROOT/crates/jamx-service" "$RUN/jamx.jam" --min-stack-size 262144 2>&1 | tail -1

bold "Deploy"
# Every work item must bring at least the gas its own receipt costs.
SERVICE=$("$JAMT" create-service --raw --min-item-gas 100000 "$RUN/jamx.jam" 50000000 2>/dev/null | tail -1)
export JAMX_SERVICE=$SERVICE
note "service id $SERVICE; waiting for its code to be finalized"
sleep 24
run show config

BRL=1; USD=2; BTC=3; PIX=4

bold "1. The last bank: an issuer that holds reserves and nothing else"
note "The bank's token is permissioned (holding it means passing the bank's checks) and"
note "freezable (with every use logged). Two independent auditors cap what can exist."
run create-token bank BRL --permissioned --freezable --attestor auditor1 --attestor auditor2 --threshold 2
run -q attest auditor1 $BRL 10000000
run -q attest auditor2 $BRL 8000000
note "An account holds only what it has accepted: each customer opens the relationship,"
note "and the bank then approves it."
for who in bob alice carol dave; do run -q accept $who $BRL; done
for who in bob alice carol dave; do run -q issuer bank $BRL allow $who; done
run -q mint bank $BRL bob 5000000
run -q mint bank $BRL alice 1000000
run -q mint bank $BRL carol 1000000
run submit
run show asset $BRL
note "The auditors vouch for 8,000,000 between them; 7,000,000 exist. The bank cannot mint past that:"
refused mint bank $BRL bob 1500000
note "And nobody without the bank's approval can receive its token:"
refused transfer bob $BRL stranger 100

bold "2. The order book with no operator"
note "A second issuer's dollar token trades against the real. Orders are signed off-chain;"
note "anyone can bundle them. One round, one price."
run -q create-token usbank USDX
run -q accept alice $USD
run -q accept carol $USD
run -q mint usbank $USD alice 4000
run -q mint usbank $USD carol 3000
run submit
run order bob   buy  $USD $BRL 3000 5.60
run order dave  buy  $USD $BRL 2000 5.40     # dave has no reais: his order will simply not count
run order alice sell $USD $BRL 2500 5.30
run order carol sell $USD $BRL 2000 5.50
run orders $USD $BRL
note "The bundler is nobody special: it lists orders and collects the fee the orders allow."
run -q accept bundler $BRL
run -q issuer bank $BRL allow bundler
run submit
run batch $USD $BRL --builder bundler --fee-bps 10
run show account bob
run show account alice
run show account bundler
note "Bundling the same signed orders again does nothing: they are spent."

bold "3. Buying bitcoin with reais, with nobody holding both legs"
note "A local Bitcoin regtest chain stands in for Bitcoin. JAM never connects to it:"
note "headers and receipts are carried in and checked by arithmetic."
run btc init demo
note "A deal keeps time by Bitcoin's own clock, not JAM's: the seller has two hours, read off"
note "the timestamps of Bitcoin's blocks, to get the payment mined. (Each block of this local"
note "chain is dated ten minutes after the one before, however fast it is mined here.)"
run create-btc bundler BTC --chain demo --confirmations 3 --pay-secs 7200 --grace-blocks 2 --bond-bps 1000 --min-deal 1000
run show asset $BTC
note "The service opens deals once it has seen ten blocks past its starting header:"
run btc mine demo 10
run btc relay demo $BTC
note "Bob wants 0.001 BTC at R\$ 600,000 per coin, delivered to his own wallet. Carol has the coins."
run order bob   buy  $BTC $BRL 100000 0.6 --deliver bob-wallet
run order carol sell $BTC $BRL 100000 0.6
run batch $BTC $BRL --builder bundler
run show deals
note "Bob's reais are locked in the deal with Carol's bond. Carol now pays from her own wallet."
run btc pay demo 1
note "JAM has not seen that block yet, so the receipt means nothing:"
refused btc claim demo 1
run btc relay demo $BTC
note "One confirmation is not three:"
refused btc claim demo 1
run btc mine demo 2
run btc relay demo $BTC
run show chain $BTC
note "Three blocks deep. Anyone can carry the receipt; the reais go to Carol."
run btc claim demo 1
run show deals
run show account carol
note "Next, a seller who never pays. Nothing on JAM's clock can time the deal out: only"
note "Bitcoin's own blocks can, and whoever wants the refund has to carry them in. Those are"
note "the same headers that would have proved a payment, had there been one."
run order bob   buy  $BTC $BRL 50000 0.6 --deliver bob-wallet
run order carol sell $BTC $BRL 50000 0.6
run batch $BTC $BRL --builder bundler
run show deals
refused timeout 2
note "Bitcoin moves on by more than two hours of its own time. Until someone carries the"
note "headers in, the service knows nothing of it:"
run btc mine demo 20
refused timeout 2
run btc relay demo $BTC
run timeout 2
note "Bob has his reais back, with Carol's bond:"
run show account bob

bold "4. Pix, with an arbiter who can reach one trade"
note "Alice sells dollar tokens for reais sent by Pix. Her tokens wait in the deal; Dave pays"
note "by Pix and posts a bond. The oracle only says whether the Pix arrived."
run -q accept dave $USD
run -q mint usbank $USD dave 500
run -q create-oracle bundler PIX --oracle pix-oracle --bond-bps 500 --window 16 --min-deal 100
run submit
note "Market PIX/USDX: 5,600 centavos (R\$ 56) for 10 dollar tokens, i.e. 5 token-cents per 28 centavos."
run order alice buy  $PIX $USD 5600 5/28 --deliver alice-pix-key
run order dave  sell $PIX $USD 5600 5/28
run batch $PIX $USD --builder bundler
run show deals
note "A stranger's word is worth nothing; the oracle's settles this one deal:"
refused oracle-attest stranger 3 paid
run oracle-attest pix-oracle 3 paid
run show account dave
note "Next, a buyer who never sends the Pix. After the deadline anyone can close the deal:"
run order alice buy  $PIX $USD 2800 5/28 --deliver alice-pix-key
run order dave  sell $PIX $USD 2800 5/28
run batch $PIX $USD --builder bundler
refused timeout 4
note "waiting for the deadline (about 100 seconds)"
run wait 17
run timeout 4
note "Alice has her tokens back, plus Dave's bond:"
run show account alice

bold "5. Custody without a custodian: a friend who can freeze but not spend"
note "A freeze lasts 20 slots here and weakening changes wait 6 (in earnest: days). An account"
note "that adds a protection without choosing a delay is given one of a day."
run -q policy alice freeze-slots 20
run -q policy alice delay 6
run -q policy alice guardian friend
run -q policy alice limit $BRL 50000
run -q accept thief $BRL
run -q issuer bank $BRL allow thief
run submit
run show account alice
note "Suppose Alice's key is stolen. The daily limit holds the thief to R\$ 500:"
run transfer alice $BRL thief 50000
refused transfer alice $BRL thief 1
note "The thief tries to lift the limit and get rid of the friend. Both changes are queued, not applied:"
run -q policy alice limit $BRL none
run -q policy alice guardian none
run submit
run show account alice
note "The friend freezes the account. He cannot move a centavo of it; he can stop everything:"
refused --key friend transfer alice $BRL friend 1
run freeze friend alice
refused transfer alice $BRL thief 1
refused policy-exec alice 0
note "Frozen, the account can still do one thing: drop what the thief queued."
run policy-cancel alice 1
run policy-cancel alice 0
run show account alice

bold "6. Social recovery: the account stays, the key changes"
note "(On a second account, so the freeze above keeps holding.)"
run policy bob recovery --threshold 2 --delay 6 guardian1 guardian2 guardian3
note "Bob loses his key. Two of his three guardians vouch for a new one."
run -q recover guardian1 bob bob-new-phone
run -q recover guardian2 bob bob-new-phone
run submit
run show account bob
refused recovery-exec bob
note "waiting out the recovery delay (about 40 seconds)"
run wait 7
run recovery-exec bob
note "Same account, same balance, new key. The old key is nobody now:"
refused transfer bob $BRL carol 1
run --key bob-new-phone transfer bob $BRL carol 1000

bold "7. The issuer's freeze button, on the public record"
run issuer bank $BRL freeze carol --reason "court order 123/2026"
refused transfer carol $BRL bob 1
run show asset $BRL
note "The bank can stop its own liability from moving. It has no operation that takes it,"
note "and no power at all over Carol's dollar tokens:"
run transfer carol $USD bob 10

bold "8. The upgrade key announces itself and waits"
cp "$RUN/jamx.jam" "$RUN/jamx-next.jam"; printf 'x' >> "$RUN/jamx-next.jam"
refused propose-upgrade stranger "$RUN/jamx-next.jam"
run propose-upgrade admin "$RUN/jamx-next.jam"
run show config
note "Announced in public state. Not even the administrator can apply it before the delay:"
refused upgrade-exec
run cancel-upgrade admin
note "And the role can be given up for good, leaving code nobody can change:"
run set-admin admin none
refused propose-upgrade admin "$RUN/jamx-next.jam"
run show config

bold "Done"
run show receipts
note "Every key in that walk-through could move one person's money, or none."
