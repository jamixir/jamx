# Measured costs

All figures were measured on 16 November 2026 on a local `dev` testnet of polkajam 0.1.29
(Gray Paper 0.8.0), six validators, PVM interpreter backend. Gas is deterministic, so the
backend does not change these numbers; it only changes how long they take in wall time.
They come from the service's own `gas` host call (build with `JAMX_FEATURES=logging` or
`profile` to print them to the node log).

## The gas model matters more than the host calls

GP 0.8.0 replaced "one gas per instruction" with a model of a CPU pipeline. In its cost
tables (appendix A, "Gas Cost Tables") a memory load or store has a latency of 25 cycles and
an ordinary branch 20; the text marks the memory figure as a temporary placeholder. A storage
`read` costs 2,407 gas plus a little per byte, and a `write` 2,442.

So one storage access costs about as much as a hundred memory accesses, and ordinary Rust
(heap allocation, copies, maps keyed by 32-byte arrays, formatting) is expensive in
accumulate. The first working version spent 89,000 gas per order, everything included. Two
rounds of measurement-led changes brought that to 27,700; the checks added after the
security review (see below) put it at 28,600. [THROUGHPUT.md](THROUGHPUT.md) has the
breakdown by phase and what each change bought. Even one line of logging with formatted
numbers costs roughly 20,000.

## Refine

| Work | Gas |
|---|---|
| Fixed overhead of an invocation | about 3,000 |
| One signed operation (Ed25519 `verify_strict`) | 3,000,000 |
| One order in a batch (signature, hash, sort) | 3,030,000 |
| One Bitcoin receipt (transaction of a few hundred bytes, short Merkle path) | 100,000 |

Budget: 1,000,000,000 on the `dev` chain, 5,000,000,000 on the full chain. That is about 330
signatures per work-package here and 1,650 on the full chain.

## Accumulate

| Work | Gas |
|---|---|
| Starting an invocation (loading configuration) | 12,000 to 25,000 |
| A simple signed operation (transfer, mint), computation | 21,000 to 26,000 |
| ... plus loading an account not yet in the cache | about 5,000 |
| Writing one changed record at the end | 4,600 |
| An auction round, per order, everything included | 28,600 |
| One Bitcoin header added to the chain, everything included | about 127,000 |

Measured on a round of 200 orders in one work-package: 5,711,699 gas for the whole
accumulate invocation, and 605,613,706 for refine.

Budget: 10,000,000 per work-report (`G_A`), on both the `dev` chain and the full chain.

## Each work item spends its own gas

JAM accumulates all of a service's work items in a block in one invocation and pools their
gas. The service divides it again: each item may spend what it declared and no more
(`Session::work`). Before every record, and before every step of a loop, it checks that
what is left to the item covers finishing cleanly:

| Kept back for | Gas |
|---|---|
| the item's receipt and checkpoint | 40,000 |
| writing each record changed so far | 6,000 each |
| undoing each record changed by the instruction in progress | 2,000 each |
| the next step of a loop (an order, a pair of a deal, a header) | 60,000 |

A record that would go past this fails with `OutOfGas` and leaves no trace; a run of headers
stops where its gas ends and keeps what it accepted. The reserves are deliberately larger
than the measured costs (a write is 4,600, not 6,000), so a full item ends with gas unspent.
That is the price of never running dry in the middle: an invocation that does traps, and
the items behind it in the block are lost with it.

These checks, and the other safeguards added after the review, cost about 900 gas per order
together (the difference between 27,700 and 28,600).

## What that means

* **About 290 orders per work-package.** At 28,600 gas each, 10 million would pay for 350.
  The reserves above take the rest. A round of 286 orders settled for 7,833,981 gas
  (27,400 each: the fixed costs spread thinner); one of 300 was refused whole with
  `OutOfGas` after 1.3 million gas of decoding, as intended, without a trap.
* **Accumulate is the bottleneck, by a factor of about five** on the full chain: refine could
  authenticate 1,650 orders in the time accumulate can settle 290. On the `dev` chain, with
  a fifth of the refine budget, the two are level.
* The 48 KiB limit on the work output is not binding yet: an order record is about 51 bytes,
  so 290 of them are 15 KiB.
* A run of Bitcoin headers is bounded the same way, and more tightly than expected: an item
  given 150 headers accepted 76 of them for 9,665,015 gas and stopped cleanly, which is
  about 127,000 gas per header. Bitcoin makes 144 blocks a day, so the header chain needs
  two work items a day. Most of that cost is not the two storage writes per header but the
  service's own generic record cache (an ordered map keyed by byte strings, which this gas
  model makes expensive); accounts were moved off it for the same reason, headers not yet.
* Several work-packages per block is the way to more volume today: the block-wide limit is
  3,500,000,000 gas on the full chain, 350 times the per-report limit. Tested with two
  rounds on the two cores of the `dev` chain: same block, one invocation.

## What would move the numbers

1. **The gas model itself.** The memory cost is declared provisional, and the maintainers
   have said that every number in the model may still change. These measurements are a
   snapshot.
2. **A flatter account record.** Decoding and re-encoding the variable-length account record
   is still a large part of the per-order cost. A fixed layout read in place should bring an
   order near 8,000 gas, most of it the one read and one write that this design cannot avoid.
3. **A state witness in refine.** If refine were given the balances it needs, with a proof
   against a state root that accumulate checks, the whole auction could run in refine and
   accumulate would only apply net balance changes. That is a rollup-style design: much
   higher throughput, at the price of sequencing every round against the previous one. It is
   the natural next step and is not attempted here.

## Reproducing

```sh
scripts/testnet.sh start
scripts/bench.sh 10                          # 20 orders, gas by phase
JAMX_FEATURES=logging scripts/bench.sh 100   # 200 orders, true totals
```
