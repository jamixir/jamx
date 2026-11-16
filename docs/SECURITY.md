# Security review

On 16 November 2026 the first working version of JAMX was given to a second reviewer (an AI
model, working from the code and the design document alone) with one instruction: find ways
to steal, to lock funds, or to stop the service, and write each as a test that passes while
the flaw exists. It found sixteen. All sixteen are fixed, and each test now asserts the
opposite: `crates/jamx-core/tests/review.rs`.

This file records what was wrong, because the mistakes are more instructive than the fixes.
It is not an audit. One reviewer, of the same kind as the author, looked for a few hours.

## What was found

Severity is the reviewer's. "Before" describes the first version.

### Theft or loss of a counterparty's funds

**F1, critical. A decoy deal used up the seller's payment.** A Bitcoin output can settle one
deal, ever. A buyer opened the real deal with the seller (tag required) and a tiny second
deal with an accomplice to the same address, tag waived. When the seller's payment
confirmed, the accomplice presented it for the second deal, which accepted any payment to
that address. The output was now marked as used; the seller's own claim was refused for
ever; at the deadline the buyer was refunded and took the seller's bond, with the bitcoin
already in hand.

*Fix.* A payment that carries a deal's tag settles that deal and no other
(`TagState::Other`). And at most one deal open to untagged payments exists per address at a
time, so the same trick cannot be played without tags.

**F2, high. Anyone could fill an account so that it could not be paid.** An issuer could set
a flag on any account for its own asset, which created a balance entry there, and an account
held a bounded number of entries. A few hundred throw-away tokens filled a victim's record.
After that the victim could not be credited any asset it did not already hold. A buyer did
this to a seller whose balance of the quote token was zero while the deal was open: her
valid proof failed at the payout, and he collected the refund and her bond. The same limit
let one account, by filling itself, undo a whole auction round.

*Fix.* Only the account adds entries to its own record: it must accept an asset before it
can be sent it (the trust-line rule of Stellar and Algorand). Issuer flags apply to accounts
that accepted the asset. An order from an account that could not take delivery is left out
of the round.

**F3, high. A stolen key installed its own recovery group, at once.** Changes that weaken an
account are supposed to wait out its security delay. Setting a recovery group where there
was none was classed as harmless. A thief named a sock-puppet as the only recovery guardian
with no delay; the recovery replaced every key and lifted the guardian's freeze. A second
form: with no guardian set, the thief appointed its own, queued the removal of the daily
limit, and kept the account frozen through the whole delay so that the owner could not
cancel the queued change.

*Fix.* Naming, changing or removing a recovery group always waits. A frozen account can
still cancel queued changes. A first protection brings a default delay of a day with it.

**F10, medium. A receipt for a transaction that never existed.** Refusing 64-byte
"transactions" stops an inner Merkle node from being passed off as a transaction. The
opposite was open: nothing fixed the depth of the tree, so a 64-byte transaction that
really was in a block could be read as an inner node, and an invented transaction "proved"
one level below. On mainnet this needs about 2^70 hash operations and a cooperating miner.

*Fix.* A receipt must show the block's coinbase under the same root at the same depth. All
transactions of a block sit at one depth, and only a coinbase looks like one.

### Abuse of an account's own rules

**F4, medium.** The length of a freeze is also the guardian's cool-down. Raising it was
applied at once: set to the maximum after any earlier freeze, it switched the guardian off
for good. Combined with a self-freeze it locked an account until the end of time.
*Fix.* Any change to it waits out the delay, and it is bounded at thirty days.

**F5, medium.** A trading key could freeze the account. A frozen owner cannot veto a
recovery, so a trading key plus the recovery quorum took the account.
*Fix.* Only an owner key or the guardian can freeze.

**F9, medium.** A failed operation kept its nonce and a signed operation never expired. A
guardian's redundant "unfreeze", refused at the time, was replayed by a thief months later
to lift a real freeze.
*Fix.* A failed operation spends its nonce; every signed operation carries an expiry slot.

### Stopping a round or the whole invocation

**F6, medium.** Two orders of one account with nonces a window apart: marking the far one
slid the window past the near one, whose marking then failed and undid the round.
**F7, medium.** In an external market an order was sized by the cost of its whole fill but
charged once per matched pair, each rounded up; a buyer cut down to exactly what it could
pay was one atom short on its second pair, and the round was undone, honest trades included.
*Fix for both.* Everything that can fail is decided when an order is first checked against
state, where leaving it out costs nobody else anything; a pair that cannot be funded is
skipped by itself.

**F11, medium. One work item spent everybody's gas.** JAM runs one accumulate invocation per
service per block, with the gas of all its work items pooled. A round of orders checked up
front that it could afford itself; a run of headers checked nothing. An item with the most
headers the format allows, declared with little gas, spent the pool and trapped. The items
behind it in that block were never applied.
*Fix.* Each item is held to the gas it declared, and every loop checks before each step that
enough is left to finish cleanly. See "Gas is per work item" in [DESIGN.md](DESIGN.md).

### Smaller things

| | Before | Now |
|---|---|---|
| F8 | A buyer naming a destination that cannot be paid collected the seller's bond. | Oracles can declare a deal void (nobody penalised); Bitcoin destinations must be standard scripts. |
| F12 | Rounding always went against the trader, unbounded relative to the fill: a bundler who was also the counterparty filled an order one atom at a time at a multiple of its limit. | A fill worth less than 100 quote atoms is rounded the trader's way and the bundler pays the atom. |
| F13 | A seventeenth resting order lost its remainder after its first fill. | It is not matched until there is room to remember it. |
| F14 | Cancelling a nonce beyond the window reported success and cancelled nothing. | It reports failure; `BumpNonce` cancels that far ahead. |
| F15 | An issuer could name itself as its own attestor. | Refused. |
| F16 | A checkpoint with near-maximal total work made a light client that never advanced. | Refused at creation; total work is added with overflow checks. |

## What the review changed in the design

Three rules came out of it that were not in the first version, and they are the ones to
keep in mind when changing the code:

1. **Nobody but its owner can make an account's record grow.**
2. **A round, once priced, can be undone only by its builder's own failure.** Every order is
   judged alone, before the price exists.
3. **A work item spends its own gas and nobody else's.**

The change from deadlines in JAM slots to deadlines kept by Bitcoin's own clock was made at
the same time for a different reason (the history of earlier projects; see
[PRIOR-ART.md](PRIOR-ART.md)), and it removes a class of timing attacks on the seller. It
also shows how easily such a thing goes wrong. Its first form counted blocks from the last
one the service knew, and the author found the hole only on re-reading: whoever builds the
batch could open the deal on a stale view of Bitcoin and leave the seller no time at all.
The window now starts at the JAM slot time and is read off Bitcoin's median time past
(DESIGN.md, section 7.1).

## What is known and not fixed

* **A trading key can lose money on purpose.** It can only place and cancel orders, but
  orders at absurd prices against an accomplice move value. The daily limit bounds it.
* **A buyer who waives the tag trusts the address.** Any payment of the right size to it
  settles the deal, whoever made it, if it was mined after the last block the service knew
  when the deal opened and inside the deal's window. If the service's view was stale at
  that moment, "after" reaches back that far. The address must be a fresh one.
* **A bundler chooses what enters a round.** It can leave orders out. It cannot reorder,
  misprice or touch funds, and anyone can be a bundler.
* **A seller must present the receipt in time.** After the payment confirms there are
  `grace_blocks` of Bitcoin time (a parameter of the asset) before the deal can be closed
  against the seller.
* **The light client does not check block timestamps.** It follows the most-work chain and
  takes timestamps as they are. A deal's clock is their median over eleven blocks, which
  one miner cannot move far; a majority of the hash rate could, and could also do worse.
* **Deep reorganisations.** The light client follows forks up to 144 blocks deep. A deeper
  one stalls it; deals then end void at their fallback deadline.
* **Whoever defines an external asset chooses its parameters.** One confirmation, or a
  fallback deadline of minutes, makes an asset that is dangerous to trade. The parameters
  are public and fixed; nothing vets them.
* **The gas reserves are estimates.** They were set from measurements on the interpreter
  backend with a gas model the Gray Paper marks as provisional. If real costs exceed them,
  an oversized item could still trap; it would lose its own work, and that of the items
  behind it in the block.
* **Storage is not charged for.** Any key can create a small account record. A deployment
  needs deposits.
* **No independent audit.** See the first paragraph.
