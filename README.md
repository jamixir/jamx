# JAMX

A non-custodial exchange as a [JAM](https://graypaper.com) service: a working prototype of
the design in
[*Bitcoin Wasn't Hacked. Binance Was.*](https://danicuki.substack.com/p/bitcoin-wasnt-hacked-binance-was)

No key in it controls more than one person's money.

**Status: prototype.** It runs end to end on a local polkajam testnet (polkajam 0.1.29, Gray
Paper 0.8.0) and has a test suite. It has been through one adversarial review, whose sixteen
findings are fixed ([docs/SECURITY.md](docs/SECURITY.md)), but not an independent audit; it
has no fee or spam model, and leaves out parts of the article's design.
[docs/DESIGN.md](docs/DESIGN.md) lists exactly what is and is not there.

## What it does

| In the article | In the code |
|---|---|
| An order book with no operator | Signed orders, bundled by anyone into batch auctions. Signatures, sorting and matching in refine; funding, the single clearing price and atomic settlement in accumulate. |
| The fiat custodian never touches the crypto | Issuer tokens: an issuer can mint up to a cap set by independent attestors, keep an allowlist, and freeze its own token with every use logged. It has no operation that moves a holder's balance. |
| Buying bitcoin: pay on proof | A Bitcoin light client in the service (proof-of-work in refine, consensus difficulty and most-work chain in accumulate) and conditional payments released by a Merkle receipt, with a bond the seller loses for walking away. A deal keeps time by Bitcoin's own clock: a seller who paid on time cannot be timed out because nobody carried the headers. |
| Pix with an oracle or arbiter | The same conditional payment, decided by a named oracle set whose only power is a verdict on that one payment: arrived, did not, or cannot be made. |
| An account model as forgiving as a bank's | Per-account rules: second-device keys that can trade but not withdraw, a friend who can freeze but not spend, social recovery, daily limits, and a delay on every change that weakens the account. An account holds only assets it has accepted. |
| The upgrade key belongs to governance or to no one | The administrator and the upgrade delay are compiled into the code; upgrades are announced on-chain and wait; the role can be renounced. |

## Try it

Requirements: Rust (stable, with the `rust-src` component), the polkajam 0.8.0 release
unpacked under `../tools/`, and `polkatool` 0.37 (`cargo install polkatool --version 0.37.0
--root ../tools/cargo`).

```sh
scripts/testnet.sh start     # six validators and an RPC node, on this machine
scripts/demo.sh              # deploys a fresh instance and walks through the whole design
scripts/testnet.sh stop
```

The demo takes about fifteen minutes; every step is a real work-package. It shows the mint
cap refusing an over-issue, an auction round clearing at one price, bitcoin bought with
reais against a local regtest chain (including a receipt refused for too few confirmations,
and a deal that can be timed out only once Bitcoin's own blocks have been carried in), a
Pix escrow released by an oracle and another timing out, a stolen key held to its daily
limit and then frozen by a friend, a lost key replaced by guardians, an issuer freeze on the
public record, and an upgrade that must wait.

By hand:

```sh
export JAMT=../tools/polkajam-v0.8.0-linux-x86_64/jamt JAMX_SERVICE=<id> PATH=$PWD/target/release:$PATH
jamx create-token bank BRL
jamx accept alice 1          # an account holds only what it has accepted
jamx mint bank 1 alice 100000
jamx accept bob 1
jamx transfer alice 1 bob 2500
jamx show account bob
jamx --help
```

Names such as `alice` are development keys derived from the name. They are for
demonstrations only.

## Tests

```sh
cargo test --release -p jamx-core
```

130 tests. The scenario tests (`crates/jamx-core/tests/`) push real payloads through the
real refine and accumulate code against an in-memory host, and after every submission check
that, for every asset, what accounts hold plus what deals have locked equals the supply.
The Bitcoin code is also tested against real mainnet headers and transactions.
`tests/review.rs` holds the attacks found by the adversarial review, each now failing.

## Layout

```
crates/jamx-core      all logic, no_std; refine.rs and accumulate.rs are the two halves
crates/jamx-service   the PVM binary: entry points and host bindings
crates/jamx-cli       `jamx`: wallet, bundler, carrier of headers and receipts
crates/jamx-pack      packs the compiled service into a JAM (GP 0.8.0) code blob
scripts/              build-service.sh, testnet.sh, demo.sh, bench.sh, fetch-btc-fixtures.py
docs/DESIGN.md        what was built and why, what is trusted, what is missing
docs/SECURITY.md      the adversarial review: sixteen flaws, their fixes, what remains open
docs/GAS.md           measured costs and what they imply for throughput
docs/THROUGHPUT.md    why a package settles a few hundred orders, and what would raise it
docs/PRIOR-ART.md     what happened to earlier attempts at each piece, and the lessons
docs/ROADMAP.md       why exchanges still dominate, and what is missing in technology, market and law
docs/explainer.html   the idea in plain words, with diagrams, and an honest "why JAM?"
docs/explainer-src/   its source: `python3 build.py template.html ../explainer.html`
docs/demo-transcript.txt   output of a full run of scripts/demo.sh
```

## Notes

* Built against the published JAM SDK crates (`jam-pvm-common`, `jam-types` 0.1.29) and
  `polkavm-linker` 0.37 as dependencies. The published `jam-pvm-build` does not yet target
  GP 0.8.0, hence `jamx-pack`.
* The polkajam nodes run the PVM interpreter by default here, which needs no kernel
  settings. See the comment in `scripts/testnet.sh` for the compiler backend.

Licensed under Apache-2.0.
