# What earlier attempts teach

Every piece of JAMX has been tried before somewhere else. This note looks at what happened
to those attempts and what JAMX should do differently because of it.

Research date: 16 November 2026. Volumes and TVL are from DefiLlama's public API on that
date, aggregated to months. The BTC Relay figures are a tally of the contract's transaction
list on Blockscout, made for this note. Three labels are used throughout: **fact** (sourced),
**claim** (the project's own statement) and **inference** (reasoning, mine or the
researcher's). Sources are at the end.

Limits of the research: Interlay's governance site no longer resolves, so its wind-down post
is reconstructed from search excerpts and from other parachains' governance pages; some
pages (Blockworks, Medium) could not be fetched; DefiLlama's derivatives data is paywalled.

## Summary

| Project | What it was | Outcome | Main cause |
|---|---|---|---|
| Phoenix | on-chain order book, Solana | technical success, commercial failure; now "Phoenix Legacy" | public quotes lost to private, oracle-driven quoting |
| Serum | on-chain order book, Solana | dead within a week in November 2022 | upgrade key held by FTX; suspicion alone was enough |
| OpenBook | community fork of Serum | rescue succeeded, venue did not | nobody funded it; it was plumbing for another protocol |
| Interlay | collateralised-vault bitcoin on Polkadot | shut down July 2026 | over-collateralisation paid for with token inflation; demand under 100 BTC |
| BTC Relay | Bitcoin light client on Ethereum | dormant since early 2018 | nobody was paid to carry headers, and nobody needed them |
| Atomiq | bitcoin swaps on proof, Solana and Starknet | offline since August 2026 | small team, attacks on off-chain infrastructure |
| CoW Protocol | batch auctions, Ethereum | success, billions a month | routes to liquidity that already exists |
| Hyperliquid | order book on its own chain | success | a chain whose ordering rule favours makers |
| Bisq | peer-to-peer bitcoin for fiat | alive for ten years, small | every trade needs deposits, patience and sometimes a human |

## 1. Phoenix

A fully on-chain limit order book on Solana, by Ellipsis Labs, with atomic settlement and
no crank. Raised about $44M across three rounds (seed, a $20M Series A led by Paradigm in
April 2024, and $21M led by Haun Ventures in October 2024).

**Facts.** Monthly spot volume peaked at $9.58B in March 2024, was $3.12B in January 2025,
$0.21B in October 2025, and about $1M in October 2026. In October 2024 the same team
launched SolFi, a proprietary market maker with private quotes; by January 2025 SolFi did
$39.6B against Phoenix's $3.1B. In December 2025 Ellipsis announced a perpetuals product and
said the spot exchange "will transition to Phoenix Legacy".

**Why.** A Helius study (August 2025) puts the cost of updating a quote on Phoenix at
100,000 to 300,000 compute units, against 143 for an oracle update on a proprietary market
maker. Block builders order by tip per compute unit, so cheap updates get cancelled and
replaced first. And an order book shows every quote to everyone, while proprietary makers
quote privately and receive mostly retail flow through aggregators. The team's own
conclusion, in its chief executive's words: "if this is the game that is being played,
we're going to play it the best."

**Inference.** The people who built the best public order book on a general-purpose chain
concluded that private, oracle-driven quoting beats it there.

**Two complications.** Leadership among the private makers also churns (one fell from
$35.9B a month to $0.85B in a year): liquidity follows the best quote and does not stay. And
a feeless order book, Manifest, did $3.26B in the last 30 days, while Solana's block
building now offers makers a way to put quote updates at the top of 50 ms batches. Order
books come back when the venue is free and the chain gives makers priority to cancel.

## 2. Serum and OpenBook

**Serum, facts.** TVL peaked at $1.88B in November 2021. It was $107.7M on 1 November 2022
and $0.40M on 14 November. After FTX was hacked, a developer stated publicly that "the
serum program update key was not controlled by its own organization, but by a private key
connected to FTX". The main aggregator switched Serum off as a source. Serum called itself
defunct on 29 November. Nobody ever showed that the key was misused.

**OpenBook, facts.** The community fork was trading within about five days. Volume ran
$84M to $241M a month through 2023, peaked at $1.08B in March 2024 and is near zero since
April 2025. For years its main use was that another protocol required an OpenBook market to
exist before a pool could be created; in July 2026 that protocol removed the dependency,
noting "there were no OpenBook orders left to stop placing".

**Inference.** The fork fixed the key and could not carry the liquidity across. With no
funded team and no fee, it was infrastructure for someone else, and stopped mattering when
they stopped needing it.

## 3. Interlay

Bitcoin on Polkadot through over-collateralised vaults, with a Bitcoin light client on the
parachain (the XCLAIM design). Launched August 2022.

**Facts.** The bridge itself never held much: about $1.3M to $3.1M. Supply was 53.6 iBTC in
2024, "recently as high as 89.5"; 711 BTC passed through in total. Vaults had to post 135%
collateral to open. They were paid about 46.5% a year, in the project's own token. Mint and
redeem fees had been cut to nearly zero because volume never paid for the vaults. A request
for a treasury subsidy was rejected 49 to 51. The largest holder removed iBTC from its pool,
citing no ongoing development. In June 2026 the team announced the wind-down; the chain
stopped in late July 2026.

**Aftermath, as recorded by another parachain's governance.** All 21 vaults were
liquidated; about 64% of iBTC was sitting on other chains, where it "can no longer be
redeemed for BTC"; the lending module ended with a 7.74% shortfall.

**Why.** At least 135% of volatile collateral per bitcoin, paid for with inflation rather
than fees, so capacity fell with the token price. Demand on Polkadot topped out below 100
BTC. I found no theft and no failure of the vault mechanism: the design worked and the
economics did not.

**Inference.** A wrapped asset inherits the lifetime of the chain that issues it. When the
team left, holders elsewhere were left with a token and no bridge.

## 4. BTC Relay

A Bitcoin light client as an Ethereum contract, 2016. A relayer stores a header and names a
fee; the fee is paid only when somebody later uses that header.

**Facts, from the chain.** 65,836 transactions. Relayers spent about 130 ETH on gas; two
addresses sent 94% of the headers. The contract paid out about 1.2 ETH in fees in its whole
life, all in 2016. It was used by about 382 contract calls in 2016, half from one contract.
Storing a header cost about 195,000 gas. The last regular relayer stopped on 27 December
2017. The repository was archived in February 2025.

**Why.** The maintainer, in 2018: the incentives are "tricky", there is a chicken-and-egg
problem, and "it looks like the last relayer stopped around cryptokitties congestion".

**Inference.** Gas was the trigger and missing demand was the cause. Relaying today would
cost about 0.017 ETH a day, and still nobody does it.

**What later designs changed.** tBTC's relay stores only difficulty-epoch data and refunds
authorised maintainers from a pool (subsidised and permissioned). Babylon refunds gas for
headers, and an audit found the refund could be used for griefing. Atomiq paid watchtowers
per swap claimed and measured escrow expiry in relay height rather than in time.

## 5. The contrast cases, briefly

**CoW Protocol** clears billions a month with batch auctions at uniform prices (peak month
$12.7B, August 2025). Inference, and the decisive point: its solvers draw on liquidity that
already exists in other venues, so it never needed resting liquidity of its own; and on a
chain with 12-second blocks, batching costs nothing in latency. Its solvers are whitelisted,
bonded and, in the shared pool, identified.

**Hyperliquid** won volume with its own chain and an ordering rule: in each block, cancels
are processed before orders that take liquidity. It also has no gas per order. The price is
a small validator set and, in one episode, validators setting a settlement price.

**Bisq** has traded about $1B in ten years and $5M to $10M a month now. Both sides lock
deposits, makers must be online, fiat takes hours to days, disputes need people. Inference:
its throughput is bounded by human attention.

**Atomiq** did what JAMX's bitcoin leg does, on Solana and Starknet. Its escrows never held
more than $0.4M. On 20 August 2026 it took every route offline: "As a small team we are
currently not able to fight against the numerous sophisticated AI-assisted attacks on our
infrastructure." No funds were lost and refunds stayed claimable.

## What JAMX should do differently

Each item says where JAMX stands today.

**1. Whoever is owed money carries the headers; never a fee per lookup.**
BTC Relay spent 130 ETH to earn 1.2. *Done in principle:* JAMX has no relayer role; the
tools carry the missing headers with the receipt. *To do:* store only what is needed (epoch
anchors and a recent window, pruning the rest), and let a claim's fee cover the headers it
needed. Test: with no open deals the light client may go stale and that is fine; with an
open deal the claimant must be able to catch it up alone.

**2. Keep a bitcoin deal's time by Bitcoin's clock, not by the host chain's.**
With a deadline in JAM slots, a seller who paid on time loses to the clock whenever nobody
has carried the headers. Atomiq measured expiry in relay height: the deal cannot expire
until the light client has seen Bitcoin advance, which means whoever wants the refund must
carry the very headers that make the seller's payment provable. *Done* (it was the first
thing built after this study). A deal's limit is a moment in time, the JAM slot time at
which it opens plus the asset's payment window, and it is read off Bitcoin's median time
past: a payment counts if its block is no later than the limit by that clock, and the deal
can be closed as unpaid only when a block buried under the confirmations and a grace period
is past it. Counting blocks from the last one the light client knew, as a first attempt
here did, lets whoever opens the deal shorten the seller's window by opening it on a stale
view. A deal on a header chain that has stalled for a very long time is undone with nobody
penalised. Tests: `a_seller_who_paid_on_time_cannot_lose_to_the_clock` and its neighbours in
`tests/bitcoin.rs`.

**3. No wrapped bitcoin and no vaults.**
Interlay needed 135% collateral, paid for it with inflation, and its shutdown stranded the
wrapped asset on other chains. *Done:* JAMX's bitcoin never enters JAM. Test: if the team
vanished tomorrow, every user gets funds back with a timeout and no operator. JAMX passes:
`DealTimeout` can be sent by anyone.

**4. The off-chain edge is the attack surface.**
Atomiq was stopped through its infrastructure and Bisq lost bitcoin to an unchecked field
from a counterparty. *Partly done:* the service validates everything it is given, and has
been through one adversarial review, which found sixteen flaws, several of them in exactly
this place: a destination the buyer supplies that nobody can pay, a second deal to the same
address, a work item that asks for more than it paid for ([SECURITY.md](SECURITY.md)).
*To do:* bundlers and quote sources must be many, replaceable and rate-limited; fuzz every
field a counterparty supplies; an independent audit.

**5. Do not expect market makers to rest quotes for six seconds.**
Phoenix had 0.4-second blocks and lost anyway. A batch auction removes the race to be
first. It does not remove the risk of a stale quote: a resting order in JAMX can be six
seconds behind the rest of the market when the round clears. Two mechanisms from the cases
address this. *To do:* orders pegged to a reference price and priced at clearing time, and
cancels applied before the round they would otherwise trade in. Test: if after six months
the spread on the main pair is more than twice a centralised exchange's plus six seconds of
volatility, makers have not come.

**6. Aim at flow that does not mind 6 to 18 seconds.**
Bitcoin swaps already take ten minutes to an hour. A bank's token has no competing venue to
be stale against. Large trades want protection from front-running more than speed. Do not
aim at perpetuals or at the pairs where milliseconds decide.

**7. Publish the upgrade key and make it wait.**
Serum lost 99% of its deposits in days on suspicion alone. *Done:* the administrator and
the delay are compiled into the code, upgrades are announced on-chain and can be applied
only after the delay, and the role can be renounced. *To do:* choose a real delay, longer
than the longest deal plus time to leave.

**8. Liquidity has to be brought, not attracted.**
Emissions bought Interlay 90 BTC and a worthless token. CoW borrowed liquidity that JAM does
not have. What worked elsewhere was a named in-house maker (Ellipsis's own, Hyperliquid's
own vault), run openly, with any incentive paid in the asset traded. *To do; a business
decision, not code.*

**9. What not to build first.**
A wrapped bitcoin. A continuous order book for major pairs. A fee market for headers. A
token whose emissions are the operators' only income. Solver bonding and identity checks
before there are solvers. Human arbitration as the main path: Bisq shows where that ends,
and it is the reason the Pix leg here should become a bank's token as soon as one exists.

## Sources

Volumes and TVL: DefiLlama API (`api.llama.fi`), series for phoenix, openbook, serum,
solfi, humidifi, manifest-trade, interlay, interlay-btc, cowswap, bisq, atomiq-exchange and
BOB, pulled 16 November 2026.

Phoenix: [Ellipsis on Phoenix Perpetuals](https://www.ellipsislabs.xyz/blog-posts/introducing-phoenix-perpetuals) (11 Dec 2025);
[Helius on proprietary market makers](https://www.helius.dev/blog/solanas-proprietary-amm-revolution) (c. Aug 2025);
[Solana Compass project page](https://solanacompass.com/projects/phoenix) (Nov 2026);
[notes of an interview with Eugene Chen](https://solanacompass.com/learn/Lightspeed/how-to-improve-solanas-market-structure-eugene-chen) (Jul 2025, machine-generated notes of a podcast);
[The Block on the Series A](https://www.theblock.co/post/286551/paradigm-leads-20-million-series-a-round-for-dex-developer-ellipsis-labs) (Apr 2024);
[SolanaFloor on Manifest](https://solanafloor.com/news/return-of-the-clob-manifest-trade-climbs-solana-dex-rankings) (c. mid-2026).

Serum and OpenBook: [The Block on the Serum key](https://www.theblock.co/post/186499/solana-liquidity-hub-serum-to-be-forked-after-possible-compromise-in-ftx-hack) (13 Nov 2022);
[The Block on Serum calling itself defunct](https://www.theblock.co/post/190566/ftx-backed-dex-serum-calls-itself-defunct-promotes-community-fork) (29 Nov 2022);
[Alchemii on the removal of the OpenBook dependency](https://www.alchemii.io/blog/do-you-need-an-openbook-market-id) (4 Aug 2026).

Interlay: [Polkadot referendum 714](https://polkadot.subsquare.io/referenda/714) (2024; supply, thresholds, vault yield, rejected subsidy);
[Hydration referendum 105](https://hydration.subsquare.io/referenda/105) (removal from the pool);
[Bifrost governance post 210](https://bifrost.subsquare.io/posts/210) (final block, liquidated vaults, stranded iBTC);
[Polkadot forum thread](https://forum.polkadot.network/t/interlays-lease-runs-out-in-less-than-a-month-and-theres-still-funds-locked-in-the-chain-we-should-probably-do-something-about-that-right/15701) (Oct 2025);
the wind-down post at `interlay.subsquare.io/posts/69`, which no longer resolves.

BTC Relay: [repository](https://github.com/ethereum/btcrelay) and [issue 50](https://github.com/ethereum/btcrelay/issues/50) (Jul 2018);
[contract on Blockscout](https://eth.blockscout.com/address/0x41f274c0023f83391de4e0733c609df5a124c3d4);
[Threshold on relays](https://www.threshold.network/blog/blockchain-relays-101/) (Jan 2023);
[Zellic finding on Babylon's refund](https://reports.zellic.io/publications/babylon-genesis-chain/findings/medium-xbtclightclient-griefing-vector-through-fork-handling-in-btclightclient).

Contrast cases: CoW Protocol docs on [bonding pools](https://docs.cow.fi/cow-protocol/reference/core/auctions/bonding-pools) and [solver onboarding](https://docs.cow.fi/cow-protocol/tutorials/solvers/onboard);
Hyperliquid docs on [the order book](https://hyperliquid.gitbook.io/hyperliquid-docs/hypercore/order-book);
[Bitcoin Foundation on the Bisq exploit](https://bitcoinfoundation.org/news/bitcoin/p2p-bitcoin-exchange-bisq-hack/) (May 2026);
[Solana Compass on Atomiq](https://solanacompass.com/projects/atomiq-exchange) and [Crypto Briefing on its outage](https://cryptobriefing.com/starknet-bitcoin-bridging-atomiq-outage/) (20 Aug 2026);
[SolLightning's design notes](https://github.com/adambor/SolLightning-readme).

Not verified: the date OpenBook v2 reached mainnet; Hyperliquid's current validator count
(sources give 4, 16 and 24 to 27); whether Atomiq has restarted (one September 2026 article
describes it as live, its escrows hold about $40).
