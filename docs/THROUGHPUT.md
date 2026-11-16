# Why the throughput is what it is

A fair question about this prototype: JAM's limits are supposed to be far above Solana's, so
why does one work-package settle only a few hundred orders? This note answers it with
measurements. Everything was measured on 16 November 2026 on a local `dev` testnet of
polkajam 0.1.29 (Gray Paper 0.8.0); `scripts/bench.sh` reproduces it.

## The short answer

1. **It is multi-core already.** Throughput is per work-package, and a service may have a
   work-package on every core in every block. Tested: two rounds submitted at once to the
   two cores of the test chain landed in the same block and were settled by a single
   accumulate invocation.
2. **It was wasteful, and partly still is.** The first working version spent 89,000 gas per
   order in accumulate. It now spends 28,600. The floor for this design is around 8,000.
3. **The bottleneck is the on-chain half, and that is how JAM is built.** JAM offers 341
   cores of stateless computation and well under one core of stateful computation. An
   exchange settles state. The Gray Paper's own estimate for a payments service is 3,000
   transactions per work-package, with a design much leaner than this one.
4. **JAM's advantage over Solana is in aggregate, not per market.** Per core, this service
   does about 48 orders a second; Solana does more than that for one busy market today.

## What the Gray Paper itself expects

The Gray Paper models a transfer service (section "Illustrating Performance"):

> A 12MB work-package can hold around 96k transactions at 128 bytes per transaction.
> However, a 48KB work-result could only encode around 6k account updates when each update
> is given as a pair of a 4 byte account index and 4 byte balance, resulting in a limit of
> 3k transactions per package, or 171k TPS in total.

Two things about that figure:

* **171k is the whole chain**: 3,000 per package, times 341 cores, every 6 seconds. Per core
  it is 500 a second.
* **It predates the current gas table.** In the source of version 0.8.0 that passage sits
  under a comment reading "Update from here". It assumes 6,000 account updates fit in
  accumulate's 10 million gas, which is 1,667 gas each. Under the 0.8.0 host-call prices one
  storage write alone costs 2,442 and a read 2,407. With one stored item per account, 10
  million gas buys at most about 4,000 blind writes or 2,000 read-modify-writes, before any
  computation at all.

The same section describes a second, "more sophisticated" model: also do the balance
updates in refine, against state witnesses, with accounts packed 1,024 to a page. That is
where its 1.4 million TPS figure comes from, and it is a different architecture from this
prototype (see "Design levels" below).

## Where the gas went

A round of 20 orders, by phase of accumulate (the `profile` build reports these):

| Phase | First profile | Now | What it is |
|---|---|---|---|
| decode | 80,507 | 81,226 | turning refine's record back into orders |
| prepare | 492,604 | 194,838 | loading each account, checking the order against it |
| price | 65,195 | 60,656 | funding at the limit price, finding the clearing price |
| allocate | 35,997 | 29,968 | funding at the clearing price, sharing the volume |
| settle | 137,397 | 73,780 | moving balances, spending nonces |
| flush | 92,286 | 113,534 | encoding and writing changed records (23 in the later run, 20 before) |

What was wrong, in order of size:

* **Every storage read was two host calls.** The SDK's read asks for the length, allocates,
  then reads. The service now declares the `read` host call itself and reads in one call
  into a reused buffer. Saved about 2,400 gas per account.
* **The account record was large and was copied repeatedly.** An account with no special
  rules still carried the full, empty policy structure inline (about 300 bytes), and Rust
  moved it several times on its way into the cache. Under a gas model that charges 25 per
  memory access that costs thousands of gas per account. The policy is now behind a pointer
  and the caches are sized ahead of a batch.
* **128-bit division on the fee path.** Fees were computed in 128-bit arithmetic, which on
  this machine is a library routine. They now stay in 64 bits whenever the numbers fit,
  which is nearly always.
* **A list that grew by one.** A buyer who receives an asset for the first time grows the
  account's balance list; that reallocated it. Room for it is now allocated when the account
  is decoded.

Not everything tried helped. Buffering multi-byte integers before appending them made
encoding slower, not faster, and was reverted. The numbers decide.

Measured end to end, without the profiling probes:

| | Gas in accumulate | Per order | Orders per 10M package |
|---|---|---|---|
| First working version (20 orders) | 1,782,971 | 89,000 | 110 |
| After the first optimisation (20 orders) | 1,086,905 | 54,000 | 185 |
| After the second (200 orders in one package) | 5,532,048 | 27,700 | 360 |
| Now, with per-item gas checks (200 orders) | 5,711,699 | 28,600 | 350 |

Refine for that 200-order package: 605,613,706 gas, 3.03 million per order, turning 28,485
bytes of signed orders into a 10,277-byte record (51 bytes per order).

The last column divides 10 million by the cost per order. What a package can carry in
practice is less, about 290: the service keeps gas in reserve at every step so that it can
always finish or undo cleanly ([GAS.md](GAS.md)). A round of 286 orders settled
(7,833,981 gas); a round of 300 was refused whole with an "out of gas" status after
spending 1.3 million on decoding it, and the invocation did not trap. The last row costs 3%
more per order than the one above it: that is the price of the checks which make a greedy
work item fail alone instead of taking the block's other items with it.

## Multi-core, measured

Two markets, 20 orders each, submitted at the same moment with `jamx --core 0 batch ...` and
`jamx --core 1 batch ...`:

```
core 0:  accumulated at slot 9848919: ok    cleared at 1911/20
core 1:  accumulated at slot 9848919: ok    cleared at 911/20
node:    jamx accumulate: 2 items, 1581367 gas
```

Same block, one accumulate invocation, each round priced on its own. Nothing in the service
is tied to a core, and rounds for the same market can run on several cores at once (each is
its own auction; an order included in two of them fills once).

So capacity scales with the cores the service buys, up to the block's accumulate budget:

| Chain | Cores | Block accumulate budget | Orders per block at 290 per package | Per second |
|---|---|---|---|---|
| `dev` test chain | 2 | 20,000,000 | 580 | about 100 |
| Full JAM | 341 | 3,500,000,000 | about 99,000 | about 16,000 |

The full-chain row would mean buying every core in every block, which nobody will. It is an
upper bound, not a forecast.

## Compared with Solana, in the same units

"JAM's limits are higher than Solana's" is true of total computation and false of stateful
computation, and an exchange is mostly the second.

| | Solana | JAM |
|---|---|---|
| Stateless computation | none separate | 341 cores, 6 s each per block |
| Stateful computation per block | 100M compute units per 0.4 s block | 3.5 billion gas per 6 s block: about 3.5 s of one core |
| Limit for one hot piece of state | 12M compute units per writable account per block | 10 million gas per work-report, any number of reports |
| Block time | 0.4 s | 6 s, and 12 to 18 s to settlement |

(Solana figures: block limit from the Solana Foundation's upgrade page; the per-account
limit is quoted there as unchanged. The JAM column is from the Gray Paper's constants and
its statement that a work-result "is then entitled to 10ms" of accumulation.)

A design that settles every order with a read and a write on-chain is using JAM's scarce
resource and ignoring its abundant one. The numbers above are the result.

## Design levels

| Level | What changes | Orders per package | Status |
|---|---|---|---|
| 0 | first working version | 110 | superseded |
| 1 | cheaper reads, smaller records, 64-bit arithmetic | 290 (350 without the safety reserves) | **this code** |
| 2 | fixed-layout account records read in place, no heap in the hot path | about 1,000 (floor near 8,000 gas per order: one read, one write, little else) | not done; an engineering task |
| 3 | balances packed many to a storage item, addressed by a 32-bit index | several thousand, until the 48 KiB output binds (about 3,000 at 16 bytes per update) | not done; changes the account model |
| 4 | refine works against a state witness and accumulate applies net changes | the Gray Paper's "sophisticated" model; limited by how well state partitions | not done; a different architecture, with rounds sequenced against one another |

Levels 2 and 3 keep everything the article cares about. Level 4 gives up the property that
any number of independent rounds can land in the same block without coordination, because
each must build on the state the previous one left.

## What this does not fix

* **Latency.** Six-second blocks and 12 to 18 seconds to settlement are properties of JAM,
  not of this code. A market maker who requotes many times a second has no use for them.
  Batch auctions are the right mechanism for that cadence, and the wrong venue for
  high-frequency quoting.
* **The gas model is provisional.** The memory cost of 25 is marked in the Gray Paper as a
  temporary placeholder, and the maintainers have said every number in the model may
  change. A cheaper memory access would lift all of these figures at once; a dearer one
  would make levels 2 and 3 necessary rather than optional.
