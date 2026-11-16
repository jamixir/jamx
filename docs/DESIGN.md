# JAMX: design

JAMX is a JAM service that implements the design argued for in
[*Bitcoin Wasn't Hacked. Binance Was.*](https://danicuki.substack.com/p/bitcoin-wasnt-hacked-binance-was):
an exchange in which no key controls more than one person's money.

This document says what was built, why it is shaped the way it is, and what it does not do.
Protocol references are to the Gray Paper, version 0.8.0 (the version implemented by
polkajam 0.1.29, against which everything here was run).

## 1. The invariant

> Go through every key in the system and ask what it can move. If any answer is
> "everybody's money", the design has failed.

Every decision below is in service of that test. Section 9 walks the list of keys and gives
the code that bounds each one.

## 2. What JAM gives us to work with

A JAM service has two entry points and they are very different machines.

| | Refine | Accumulate |
|---|---|---|
| Where it runs | in-core, on a few guarantors, then re-run by auditors | on-chain, by every validator |
| State | none (it may only look up preimages) | the service's key/value store |
| Budget per work-package (full chain) | 5,000,000,000 gas, up to 13.8 MB of input | 10,000,000 gas |
| Budget on the `dev` test chain | 1,000,000,000 gas | 10,000,000 gas (20,000,000 per block) |
| Output | at most 48 KiB per work-report | state changes |

(GP 0.8.0 appendix on definitions: `G_R`, `G_A`, `G_T`, `W_R`, `W_B`. The `dev` values are
what the node's `parameters` RPC reports.)

Three consequences shape everything:

1. **Signatures and proofs belong in refine.** One Ed25519 verification costs about 3.0
   million gas in the PVM (measured). Accumulate could afford three. Refine can afford about
   330 on the test chain and 1,650 on the full one.
2. **Anything that depends on balances belongs in accumulate**, because refine cannot see
   them. Refine therefore cannot know whether an order is funded.
3. **Both stages are narrow.** Everything refine wants accumulate to know must fit in 48
   KiB, and accumulate must do its part in 10 million gas. Under the provisional gas model of
   GP 0.8.0 (a memory access costs 25, a branch 20) that is tighter than it sounds; see
   [GAS.md](GAS.md) and [THROUGHPUT.md](THROUGHPUT.md).

## 3. Shape of the service

```
 wallet / bundler / carrier                 JAM core (refine)                    JAM chain (accumulate)
 --------------------------                 -----------------                    ----------------------
 payload = [ item, item, ... ]   ───────▶   for each item:                       for each record:
   signed operation                           verify signature      ─────▶         authorize, check nonce
   order batch                                verify, sort, match                  fund, price, settle
   Bitcoin headers                            check proof-of-work                  check difficulty, extend chain
   Bitcoin receipt                            hash tx, walk Merkle path            match root, depth, deal
   plain operation                            pass through                         apply if state allows
                                            output = [ record, record, ... ]     receipt (one status per record)
```

One service holds the ledger, the order book logic, the Bitcoin header chain and the deals.
That is deliberate. JAM services talk to one another only through deferred transfers with a
128-byte memo, which arrive in a later accumulation round; a swap that spans two services
cannot be atomic without one of them holding the other's asset in the meantime. Keeping the
legs of a trade in one state is what makes "settled all at once or not at all" true. (The
decomposition into several services, with a deposit step between them, is possible and is
listed under future work.)

Code layout:

* `crates/jamx-core`: all logic, `no_std`, written against a small `Host` trait. The same
  code runs in the PVM and in native tests.
  * `refine.rs` (stateless half), `accumulate.rs` (stateful half), `state.rs` (write-back
    cache with a rollback journal), `auction.rs`, `btc.rs`, `ops.rs` (wire formats),
    `types.rs` (records).
* `crates/jamx-service`: the PVM binary, sixty lines of glue to the JAM host calls.
* `crates/jamx-cli`: `jamx`, a wallet, bundler and carrier for demonstrations.
* `crates/jamx-pack`: turns the compiled ELF into a JAM code blob (see section 11).

## 4. Accounts: the custody job without a custodian

JAM has no built-in notion of an account that signs transactions, so the service defines
its own. An account is identified forever by the Ed25519 key that first controlled it. With
no further configuration, that key is the only thing that can act for it.

An account may then give itself rules (`Policy` in `types.rs`):

| Rule | What it does |
|---|---|
| more keys, with roles | an *owner* key can do anything; a *trader* key (a phone, a bot) can place and cancel orders and nothing else |
| guardian | another account that can **freeze** this one and lift the freeze, and can do nothing else |
| recovery | *m* of *n* accounts can install a new owner key, after a delay during which the owner can veto |
| daily limit | at most this much of an asset may leave per day, by transfer or by trade |
| security delay | every change that weakens the account waits this long before it applies |

The rule about delays is what makes the rest worth having. A thief who steals an owner key
would otherwise just remove the guardian and lift the limit. So `Policy::weakens` classifies
changes. These wait: adding or removing keys; replacing or removing a guardian; naming,
changing or removing a recovery group; raising or removing a limit; shortening the delay;
changing the length of a freeze in either direction. They are queued, visible on-chain, and
applied by a separate operation once the delay has passed; a freeze blocks their
application. Changes that only strengthen the account apply at once.

An account that adds its first guardian, limit or recovery group without having chosen a
delay is given one of a day. A protection that the owner key can undo in the next block
protects against nothing.

Design decisions here that are judgment calls:

* **The owner cannot lift a freeze early; the guardian can.** Whoever stole the key would
  lift it. To stop a guardian from locking an owner out for good, a freeze has a bounded
  length (thirty days at most) and cannot be renewed until an equal period has passed. The
  security delay must be shorter than that period, so a change that removes the guardian
  can always be queued and applied between two freezes.
* **A frozen account can still cancel queued changes, and nothing else.** Cancelling only
  keeps the account as it is. Without this, whoever controlled a freeze could hold the owner
  still until a queued change came due.
* **The owner can veto a recovery while not frozen.** That protects against colluding
  recovery guardians. It also means a thief holding the key could veto, unless the guardian
  has frozen the account. So a freeze guardian together with a recovery quorum can take an
  account over. That is the trust placed in "trusted friends", and its reach is one account.
* **Only an owner key or the guardian can freeze.** Because a frozen owner cannot veto a
  recovery, the power to freeze is a step towards the power to take the account, and a
  trading key must not have it.
* **Each guardian holds one vote for one key.** A recovery completes when *m* votes name the
  same key, so a single dissenting guardian cannot reset the others' count.

Replay protection is a sliding window of 256 nonces per account (`Account::nonce_use`), not a
strict counter, because orders are signed long before they trade and several bundlers may
carry the same account's operations in parallel. A signed operation also carries the last
slot at which it may be applied, and an operation whose signature and nonce were good spends
its nonce even if it then fails. Otherwise bytes that were refused once, or never submitted,
could be presented by anyone later, when they would succeed and mean something else.

**Everything an account owns is one storage item**: its balances in every asset, its nonce
window, its resting orders and its policy. A settlement that touches an account costs one
read and one write whatever it does.

**Only the account adds to that item.** An account holds an asset only once it has accepted
it, with an explicit operation or by signing an order that buys it, and it can accept at
most 64. A transfer, a mint or an issuer's flag aimed at an account that has not accepted
the asset is refused. This is the trust-line rule of Stellar and Algorand, and it is here
for the same reason: if anyone could make an account's record grow, anyone could make its
trades more expensive or, at the limit, stop it from being paid. (Refunds and releases of
money the account itself locked are exempt from the count.)

## 5. Assets: one key, one issuer

Fiat on a chain is somebody's liability, so the reachable property for the fiat leg is not
"one key, one person" but "one key, one issuer, visibly bounded". Anyone can create an asset
and becomes its issuer. An issuer can:

* **mint**, up to a cap. If the asset names attestors (auditors), the cap is the largest
  reserve figure that at least *threshold* of them vouch for (`Asset::mint_cap`). The issuer
  cannot be one of its own attestors. (Nothing stops it from controlling another account
  that is; who the attestors are is public, and it is for the market to judge them.)
* **allow or disallow** accounts that have accepted the asset, if it was created
  *permissioned* (a bank's token carries the bank's identity checks). A disallowed account
  can still send what it holds.
* **freeze or unfreeze** a holder's balance of that asset, if the asset was created
  *freezable*. Every use is appended to an on-chain log with the hash of a stated reason.
  A frozen balance cannot be dropped and accepted again to shed the mark.

What an issuer cannot do is as important: there is no operation that moves a holder's
balance to anyone, and none that touches any other asset. A frozen balance is still the
holder's. Redemption is the holder sending tokens to the issuer and the issuer burning them.

Whether the reserves exist is an off-chain fact. The code makes the issuer's powers public
and bounded; it cannot make a bank honest, and does not claim to.

## 6. The order book: a batch auction

An order is a signed message: account, nonce, side, quantity, limit price (an exact ratio of
quote atoms per base atom), expiry, the highest fee it accepts. It stays off-chain until some
bundler puts it in a batch. Anyone can bundle; the batch names the account that collects the
fee, capped at 1% by the code and further by each order.

**In refine** (`refine_batch`): every signature is checked; orders are sorted by price and
then by hash, so the bundler's ordering is erased; a second order with the same account and
nonce is dropped; orders that cannot cross anything in the batch are removed. What is left,
the *crossing book*, is the record.

**In accumulate** (`settle_batch`), where balances are visible:

1. each order is checked against state (key role, freeze, nonce or resting remainder,
   allowlists, issuer freeze, daily limit) and cut down to what its account can fund;
2. the clearing price is found over those funded quantities only (`auction::discover`):
   maximize volume, then minimize the unmatched remainder, then take the midpoint of the
   interval of prices that satisfy both;
3. the volume is shared by price priority, pro rata within the last price level
   (`auction::allocate`);
4. every buyer pays that one price and every seller receives it, in one atomic step, between
   the traders' own accounts.

**Rounding.** Amounts are whole atoms, so the price has to be rounded for each fill: up for
a buyer, down for a seller. The atoms in between go to the bundler with the fee (burned if
the bundler's account does not accept the quote asset, so that supply always equals the sum
of holdings). That can carry a fill past its order's limit by less than one atom. For a fill
worth a hundred quote atoms or more this is at most one part in a hundred and is accepted.
Below that, where one atom could be most of the fill, the rounding goes the trader's way
and the bundler makes up the atom from its own balance. Without this a bundler who was also
the counterparty could fill a resting order one atom at a time at a multiple of its limit.

**A round cannot be undone by one of its orders.** Everything that could make an order fail
at settlement is decided when the order is first checked against state, where refusing it
costs nobody else anything: whether its account could take delivery; whether its nonce and
the other nonces of the same account in the round can all be marked, in any order; whether
the account has room to remember it if it comes to rest. An order that fails any of these
is left out. Once the round has a price, the only failures left are the bundler's own (a
batch too large for its gas, a rounding shortfall it cannot cover).

**Why the price is fixed in accumulate.** The first version fixed the price in refine and
let accumulate only shrink the volume. A test then showed the flaw: one order with a good
price and no money behind it set the price and voided the round. A bundler can filter
unfunded orders from state it reads off-chain, but a trader can move funds in the same block,
so the exchange would be exactly as robust as the bundler is fast. With the price computed
over funded orders, **an order that cannot pay has no influence on anything**, and unfunded
orders are excluded before price discovery so they cannot even shift a midpoint. The cost is
a linear pass in accumulate over orders that arrive already sorted.

So "the matching engine runs in refine" is half true here, and the document should say so:
refine does the authentication, the sorting and the pruning, which is where the computation
is. The last step of matching, the one that needs balances, runs in accumulate.

Properties, each with tests (`tests/exchange.rs`):

* one price per round, within every traded order's limit;
* the result is independent of the order in which the bundler listed the orders;
* holdings equal supply after every round (checked after every submission in every test);
* an order never fills beyond its size, in one round or across rounds; a filled or cancelled
  order cannot be replayed;
* an unfunded, frozen, expired or disallowed order drops out and costs nobody anything;
* two orders of one account cannot count on the same money;
* a fill is never more than one part in a hundred past its order's limit.

What the bundler can still do: choose which orders go in a round. It cannot reorder for
advantage, cannot trade anyone beyond their limit, and cannot touch funds. Several bundlers
can submit rounds for the same market in the same block; each round clears on its own.

## 7. Assets that live elsewhere: deals

JAM cannot hold a bitcoin, and a Pix transfer is not a JAM object. Such assets are defined
as *external*: they have no balances here and trade only through **deals**
(`Deal` in `types.rs`), which are conditional payments between two people:

> `payer` locked `amount` of a token. `payee` must deliver the external asset to `delivery`
> and prove it; then the amount, and the payee's bond, go to the payee. If the time for that
> runs out, the payer is refunded and keeps the bond.

Deals are created by the same auction. In a market whose base asset is external, buyers
state a delivery destination and lock the quote token; sellers lock a bond (a parameter of
the asset, as in Bisq, to price the free option of walking away). After clearing, buyers and
sellers are paired in order and one deal is opened per pair. A pair that cannot become a
deal (too small, not fundable after rounding, an address already in use by an untagged
deal) is left out; both orders keep that quantity.

A deal ends in one of three ways (`Outcome`):

* **paid**: by proof (anyone carries it), by release from the payer (who has seen the payment
  and does not want to wait), or by attestation (oracle rails). The payee receives the
  locked amount and its bond.
* **unpaid**: time ran out with no proof, or the oracles said so. The payer is refunded and
  receives the bond.
* **void**: it could not be established either way. Each side gets back what it put in.

### 7.1 Bitcoin

The asset's creator supplies chain parameters and a checkpoint header; both are public and
can be checked against the real chain before anyone trusts the asset.

*Headers* (`refine_headers`, `add_headers`). Refine checks that each header links to the one
before and that its hash meets the target in its own difficulty field. Accumulate checks that
the field is the one consensus prescribes after its parent, including the retarget every
2016 blocks with Bitcoin's exact arithmetic and clamping (`btc::next_bits`), stores the
header with its cumulative work, and keeps a height index of the most-work chain, following
reorganisations.

Checking the difficulty field is not optional. Without it, anyone could append six easy
blocks containing a fake payment; they would be valid proof-of-work for the difficulty they
claim. `tests/bitcoin.rs::headers_with_the_wrong_difficulty_are_refused` is that attack.
Median-time-past and the two-hour future limit are not checked: under a most-work rule a
miner who distorts timestamps to lower difficulty still has to out-work the honest chain.

*Receipts* (`refine_proof`, `claim_payment`). Refine hashes the transaction, walks the Merkle
path to a root, reads the output, and checks that the block's coinbase transaction sits
under the same root at the same depth. Accumulate requires that root to be the one in a
header on the best chain, buried under the configured number of blocks, mined above the
height the chain had when the deal opened and inside the deal's payment window; the output
to pay the deal's script at least the agreed amount; the
transaction to carry the deal's tag in an `OP_RETURN` (unless the buyer waived it), and in
no case another deal's tag; and the output never to have settled a deal before.

*The deal's clock is Bitcoin's.* Bitcoin keeps its own time for time locks: a block's
*median time past*, the median of its timestamp and those of the ten blocks before it. It
never goes backwards and no single miner can push it far. A deal uses it. When the deal
opens, the service notes the JAM slot's wall-clock time and adds the asset's `pay_secs`:
that is the deal's limit. A payment counts if the median time past of its block is no later
than the limit. The deal can be closed as unpaid only when the block that lies
`confirmations + grace_blocks` below the best tip shown to the service is itself past the
limit by that clock; from then on no payment made in time can still turn up.

This is the lesson of the projects that tried it with wall-clock deadlines (see
[PRIOR-ART.md](PRIOR-ART.md)): if nobody carries the headers, a seller who paid on time can
be timed out. Here, without headers the clock does not move either. Whoever wants the refund
has to carry the headers in, and those are the same headers that prove a payment, had there
been one; the seller's receipt is good for as long as the deal is open. The seller's duty is
to present it within `grace_blocks` of Bitcoin time after it confirms.

A first version of this counted the window in blocks from the last block the service knew.
That was wrong, and it is worth saying why: whoever builds the batch chooses when a deal
opens, and could open it while the service's view of Bitcoin was hours behind, handing the
seller a window that had already passed. A window that starts at the JAM slot time and is
read off Bitcoin's clock does not depend on how recent the service's view is
(`tests/bitcoin.rs::a_stale_view_of_bitcoin_does_not_shorten_the_sellers_window`).

Two guards go with this. No deal opens until the service has seen eleven blocks past its
starting header, so that every median it uses is a full one. And each deal has a fallback
deadline in slots, far longer than the whole course should take: if the header chain has
stalled for that long, anyone can close the deal as void.

*Tags and addresses.* By default a payment must carry the deal's tag, a 32-byte value in an
`OP_RETURN` output. A buyer can waive that (`ORDER_NO_TAG`) so that a seller can pay from
somewhere that cannot add outputs, such as an exchange withdrawal. A payment that carries a
tag settles the deal it names and no other; and at most one deal open to untagged payments
exists per address at a time. Together these mean that a payment made for one deal cannot
be claimed for another, which would leave the first seller unable to prove anything. The
buyer who waives the tag takes a different risk: any payment of the amount to that address
inside the window settles the deal, so the address must be fresh.

A buyer's destination must be one of the standard output scripts (P2PKH, P2SH, P2WPKH,
P2WSH, P2TR). A seller is matched without seeing it, and must not be matched with a deal no
wallet can pay.

Attacks considered, each with a test:

| Attack | Defence |
|---|---|
| mine easy blocks containing a fake payment | difficulty field must match consensus |
| pass an inner Merkle node off as a transaction | 64-byte "transactions" are refused |
| read a real 64-byte transaction as an inner node and "prove" an invented one beneath it | the coinbase fixes the depth of the tree |
| reuse one payment for two deals | each (txid, output) settles one deal, ever |
| open a decoy deal and have the seller's payment presented for it | a tagged payment settles only its own deal; one untagged deal per address |
| claim with a payment made before the deal | proving block must be above the deal's start height |
| claim with an unrelated payment to the same address | deal tag in the transaction |
| present the proof from a branch that lost | height index follows the most-work chain |
| let the deal time out although the seller paid | the deal runs on Bitcoin's clock; no headers, no timeout |
| open the deal on a stale view of Bitcoin, so that the window has already passed | the window starts at the JAM slot time, not at the last block known |
| name a destination nobody can pay, to collect the bond | standard scripts only |

The header and receipt code is tested against real mainnet data: twelve headers across the
retarget at height 840,672 and transactions from block 840,673 with its coinbase
(`tests/fixtures/btc_mainnet.txt`), including a full deal released by a real transaction.

### 7.2 Oracle rails (Pix)

An external asset can instead name a set of oracle accounts and a threshold. An oracle's only
operation is to give one of three verdicts on one deal: the payment arrived; it did not; or
the destination the buyer named cannot be paid at all. Enough votes for a verdict close the
deal as paid, unpaid or void. The third verdict exists because the seller is matched without
seeing the destination: without it, a buyer who named a Pix key that does not exist would
collect the bond of everyone matched with it. An oracle cannot name a recipient, change an
amount, or touch anything outside the deal. In the usual case no oracle is needed: the payer
confirms receipt. On this rail the deadline is in JAM slots.

## 8. Governance and upgrades

Whoever can change a service's code can do anything. So:

* the administrator account and the upgrade delay are **compiled into the code**
  (`JAMX_ADMIN`, `JAMX_UPGRADE_DELAY`). The code hash commits to them, and nobody can race to
  claim the role after deployment. With no administrator the code is final.
* an upgrade is **announced** on-chain with the hash of the new code, and the service asks
  the chain to hold that code as a preimage, so anyone can fetch and read it;
* it can be applied only after the delay, only if the code is actually available, by anyone;
* the administrator can hand the role over or give it up for good.

The administrator has no other power: it cannot move funds, freeze accounts or alter assets.

## 9. What is still trusted

| Who holds it | What it can move | Where the bound is in the code |
|---|---|---|
| a user's key | that user's account | `Account::authorized`; everything in `apply_op` acts on the signer's own account |
| a trader key | orders of one account, within its daily limits (it can trade badly on purpose, up to those limits) | `Perm::Trade` |
| a freeze guardian | nothing; can stop one account for a bounded time | `Op::Freeze`, `Op::Unfreeze` |
| recovery guardians (*m* of *n*) | the keys of one account, after a public delay | `Op::RecoveryApprove`, `PlainOp::RecoveryExec` |
| an issuer | supply of its own asset up to the attested cap; a freeze flag on its own asset in accounts that accepted it, logged | `Op::Mint`, `Op::IssuerSet`, `Asset::mint_cap` |
| an attestor | one of *n* figures that bound one issuer's supply | `Op::Attest` |
| the bundler | nothing; chooses which orders enter a round, keeps the rounding and makes it up on very small fills | `refine_batch` sorts, `settle_batch` prices |
| whoever carries headers and receipts | nothing; a false one fails the check | `refine_headers`, `refine_proof`, `add_headers`, `claim_payment` |
| an oracle | the outcome of one deal (paid, unpaid or void) | `Op::OracleAttest` |
| whoever defines an external asset | nothing afterwards; its parameters (confirmations, windows, bond, checkpoint) are fixed and public, and a bad choice harms those who trade it | `create_asset` |
| the administrator | the code, after a public delay | `Op::GovProposeUpgrade`, `PlainOp::GovExecUpgrade` |
| JAM's validators | everything, as with any chain | consensus, auditing |

There is no key in that list whose answer is "everybody's money", except the validator set
itself, which is the security assumption of the whole chain.

## 10. Failure handling

* **One record, all or nothing.** `Ctx` journals the first change to any record inside an
  instruction and restores it if the instruction fails. The one thing a failed signed
  operation leaves behind is its spent nonce (section 4).
* **One work item, all or nothing.** After each work item's records are applied, changes are
  flushed and the host is asked to checkpoint. If the host later refuses a write (the
  service's balance no longer covers the storage deposit) the invocation panics, and JAM
  discards everything since the last checkpoint.
* **Gas is per work item.** JAM runs one accumulate invocation per service per block and
  pools the gas of all the work items in it. The service holds each item to the accumulate
  gas it declared (`Session::work`). Before each record, and before each step of every loop
  (an order, a pair, a header), the gas left to the item is compared with what finishing
  cleanly would take: writing the records changed so far, undoing the record in progress,
  the receipt. If it is short, the record fails with `OutOfGas` and the item's remaining
  records are skipped. A run of headers stops where its gas ends and keeps what it accepted.
  An item that brings less than its own receipt costs is ignored (the service should be
  created with a matching minimum item gas, so that the chain refuses such items outright).
  The effect is that a greedy or oversized item fails alone and cannot spend the gas of the
  items behind it.
* **Receipts.** A ring of the last 64 receipts is kept in state, keyed by the hash of the
  payload, one status per record, so a wallet can learn what happened to what it sent.

The first version was reviewed adversarially, and sixteen ways to break it were found and
fixed. [SECURITY.md](SECURITY.md) lists them; each is a test in `tests/review.rs`.

## 11. Building for GP 0.8.0

The published `jam-pvm-build` tool stops at GP 0.7.2, and polkajam 0.1.29 implements 0.8.0
(host calls renumbered, a new `grow_heap` call, a new gas model). The service is built with
the published SDK crates at 0.1.29 as ordinary dependencies, linked with `polkavm-linker`
0.37, and packed into the JAM blob format by `crates/jamx-pack`, a hundred-line tool that
calls the published `jam-program-blob-common` library. `scripts/build-service.sh` does all
three steps.

## 12. Not done

Each of these is in the article and not in the code.

* **Private balances.** Balances and orders are public. The article calls zero-knowledge
  balance proofs "the condition of existence" for a bank's participation; nothing here
  addresses it. A proof verifier would live in refine.
* **Ethereum and the other chains with finality certificates.** No light client for any
  chain but Bitcoin-style proof-of-work, and no outbound bridge. Accumulate does not yet
  yield a commitment for BEEFY to carry.
* **Bitcoin inside JAM** (collateralised vaults, BitVM). Not needed for trading, not built.
* **Trades with both legs outside JAM** (bitcoin for Pix, with a deposit from each side).
  The deal structure has one external leg.
* **Clearing between issuers at par.** Two banks' real tokens are two assets; they trade
  against each other in the order book but there is no par-conversion facility.
* **Lightning.** Deals wait for confirmations.
* **Fees for the service itself and spam control.** The service's own balance pays for
  storage; nothing charges users for the state they create. A real deployment needs a
  deposit per account and per deal.
* **Pruning.** Old headers and used-payment markers are kept forever.
* **Several services.** See section 3.
* **Throughput.** A few hundred orders per work-package at present, on any number of cores
  at once. [THROUGHPUT.md](THROUGHPUT.md) explains where the limit comes from and what would
  raise it.
* **Orders pegged to a reference price, cancels applied first.** Lessons from earlier
  projects; see [PRIOR-ART.md](PRIOR-ART.md). (The third lesson, deadlines kept by
  Bitcoin's clock instead of JAM's, is implemented: section 7.1.)
* **An independent audit.** One adversarial review by a model is not one.

## 13. Open questions for the author

1. Is the split described in section 6 acceptable for the article's claim that the order
   book is "computed in parallel"? Authentication and sorting are; final pricing is not,
   and cannot be without giving refine a view of balances (a state witness, which is the
   road to much higher throughput and a much larger design).
2. The freeze and recovery rules in section 4 embed choices about who is trusted with what.
   They are defensible, not obviously right.
3. The bond for the off-chain side is a fixed percentage set by whoever defines the external
   asset. Bisq lets each offer choose.
4. Midpoint pricing splits the surplus evenly. Exchanges with a reference price would break
   ties toward it; there is no reference price in a stateless refine, and using the last
   on-chain price in accumulate is possible now that pricing happens there.
