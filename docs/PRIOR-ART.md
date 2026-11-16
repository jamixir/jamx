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
| Phoenix | on-chain order book, Solana | technical success, commercial failure; the spot book is now "Phoenix Legacy" and the team's perpetuals venue does about $1.1B a month | public quotes lost to private, oracle-driven quoting |
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

## 6. The wider field in November 2026

A second sweep, on the same day, looked at about twenty more venues. Figures are DefiLlama's
for the 30 days to 16 November 2026 unless a month is named. The researcher could not
audit custody or wash trading, and several items rest on news summaries rather than primary
pages; the source list marks those.

**How much trading is on-chain at all.** In September 2026, on-chain venues did 19.8% of
all spot volume and about 13.7% of all futures volume (CoinDesk Data exchange review).

**Who is large.**

| Venue | 30-day volume | Against its peak month | What it is |
|---|---|---|---|
| Hyperliquid (perps) | $260.5B | 42% below Aug 2025 | its own chain; 27 validators, five named by its foundation hold 47.5% of stake |
| Uniswap | $63.1B | 46% below Aug 2025 | $14.6B of that came from one broker's new chain within four months of its launch; its own chain fell from $12.4B a month in August 2025 to $0.4B |
| Lighter, Aster, edgeX | $79.5B, $48.6B, $16.7B | 75% to 88% below their late-2025 peaks | each fell after its points or token event |
| Variational | $59.4B | rising | one in-house market maker takes every trade |
| CoW Protocol | $4.1B | 74% below Aug 2025 | 16% of aggregator volume on Ethereum |
| Solana, all venues | $46.1B | | private market makers 38%, public pools 49%, public order books 10% |

**Who stopped, and why.**

| Venue | What happened | Main cause |
|---|---|---|
| Drift (Solana perps) | about $285M taken on 1 April 2026 through a 2-of-5 administrator multisig with no time lock; no volume since | keys |
| Paradex | a database error priced bitcoin at zero on 19 January 2026 and the chain was rolled back; volume down 99% | operations, trust |
| dYdX | down 99% from its peak; its founder launched a new venue on a broker's chain in July 2026 | makers and users left; the team's attention moved |
| THORChain | lending arm insolvent in January 2025 with about $200M of liabilities; $10.7M taken in May 2026 by a newly admitted node through a flaw in its threshold-signature scheme, network halted five weeks; pools now hold 187 BTC; the laundering route of record for two exchange thefts | economics, then security, then legal exposure |
| Boltz (Lightning swaps) | suspended all swaps on 3 August 2026: "attackers now iterate faster than a team our size can find and patch" | team too small to defend |
| Maya, Garden | halted or emptied by exploits of their off-chain signers and solvers in 2025 and 2026 | security of the off-chain part |
| Paxful | closed 1 November 2025 citing the cost of compliance; pleaded guilty to unlicensed money transmission | regulation |
| Polkadot venues | Hydration down 91% in a year to $0.04B a month, with staff cuts; Polkadex gone | demand |

**Bitcoin on other chains.** Custodial wrappers hold about 290,000 BTC (WBTC about 116,000,
cbBTC about 106,000, BTCB 68,000). Everything else is small. Networks of bonded signers
who share a threshold key hold about 4,400: tBTC about 4,070 ($263M at $64,700), THORChain
187, Chainflip about 100. The one BitVM bridge that is live holds about 110. That is one
bitcoin in sixty-five outside a single custodian, and all of it behind a set of signers.
(The first version of this section said 400; it had left tBTC out.)

**Bitcoin for fiat, peer to peer.** Bisq did 89.5 BTC in October 2026, about $7M, a third of
a year earlier, and lost 11.6 BTC of users' funds on 1 May 2026 to an unchecked fee field
(later reimbursed). The others that remain (Peach, Hodl Hodl, RoboSats) are smaller.

**Brazil.** Real-denominated tokens total about R$1.03 billion in supply across eight
issuers, the largest three at R$270M to R$370M each. The central bank recorded $6.8 billion
of stablecoin purchases by Brazilians in the first quarter of 2026 alone. The one
non-custodial Pix route with visible volume did $1.6M in October 2026 and has a single
upgrade key.

**The count.** Sorting about twenty failures and declines by their main documented cause:
a security or operational incident, seven; design or economics, seven; demand, four;
regulation, one; a documented technical limit of the host chain, none. (Host-chain limits
are a plausible contributor in three: dYdX's speed, public order books on Solana, and
Bitcoin's script limits holding BitVM bridges near 100 BTC.)

**What the survivors share** (inference): they control their own ordering or sit on someone
else's distribution; the liquidity that grows is private and professional, not open order
books; volume bought with points leaves when the points end; and in bitcoin, custody won.
Two more from the same data. A product that exchanges lack is a lead and not a moat: after
two large exchanges listed stock and commodity perpetuals inside their own accounts,
Hyperliquid's share of that product was reported to fall from 80% to 26% within weeks
(one analytics source). And a venue's own chain is not distribution: Uniswap's fell by 97%
while Uniswap on a broker's chain became a quarter of its volume.

## Would a better chain have saved them?

For each failure the question is the same: was it a limit of the chain the project ran on,
and would JAM change the result? The answers are inferences from the facts above.

| Project | What stopped it | A limit of the host chain? | What JAM changes | Verdict |
|---|---|---|---|---|
| Phoenix | Makers moved to private, oracle-driven quoting. Updating a public quote cost 100,000 to 300,000 compute units and lost the race for position in the block. | Partly. The cost of a quote update and the ordering rule are Solana's. | In JAMX a quote is a signed message that stays off-chain until it trades, so changing it costs nothing on-chain; and a round has one price, so there is no position inside it to race for. It does not change the other two reasons: a public quote can be taken by anyone, and a quote that rests through a six-second round is staler than one refreshed every 50 ms. | Partly technical. JAM removes that part and makes the latency part worse. |
| Serum | The upgrade key was held by FTX. | No. Any chain allows a public, delayed, renounceable upgrade rule; Serum did not have one. | Nothing specific. (A JAM service has no upgrade key unless its own code creates one, so "no key" is the default rather than an option.) | Not technical. |
| OpenBook | No funded team, no fee, and the liquidity did not follow the fork. | No. | Nothing. | Not technical. |
| Interlay | 135% collateral per bitcoin, paid for with an inflating token; demand under 100 BTC. | No. Collateral is the price of *wrapping*: Bitcoin cannot check another chain's state, so whoever holds the coins must be trusted or bonded, whatever the host chain. | Nothing for a wrapped asset. The answer here is not to wrap. The demand problem (few people wanted bitcoin on Polkadot) would be the same on a new chain, or worse. | Not technical. A different product avoids it. |
| BTC Relay | Nobody was paid to carry headers, each cost about 195,000 gas on Ethereum, and almost nobody used them. | Partly. Gas was the trigger. | Proof-of-work is checked in refine, which is cheap and parallel. A header costs about 127,000 gas of accumulate here (measured), so Bitcoin's 144 blocks a day take two work items; what that costs in money depends on a price for core time that does not exist yet. Chains cheaper than 2017 Ethereum fix this part too: tBTC, Babylon and Atomiq all ran relays. The cause, no demand and nobody paid, is fixed by design and not by the chain: whoever needs the proof carries the headers. | The trigger was technical, the cause was not. |
| Atomiq | A small team stopped by attacks on its off-chain infrastructure; escrows never above $0.4M. | No. Its on-chain part worked, on two chains. | Nothing. Bundlers, carriers of headers, front-ends and sources of quotes are off-chain here too. | Not technical. |
| Bisq | Every trade needs deposits, makers who are online, slow fiat and sometimes a human. | No chain is involved in the bottleneck. | Nothing by itself. What removes the bottleneck is fiat as a token issued by a bank, which is a matter of institutions and law. | Not technical. |

Two of seven had a technical part, and in both the other cause was the larger one.

The two successes point the same way. Hyperliquid won with speed and an ordering rule that
favours makers, on a chain built for it; JAM's six-second blocks cannot play that game. CoW
won by routing to liquidity that already sat on its chain; JAM has none to route to.

What JAM does change is who has to be trusted. Heavy computation is done by a few and
re-run by randomly chosen auditors, so there is no sequencer, no whitelisted solver and no
upgrade council in the path; and another chain's proofs are checked by ordinary code inside
the service, so there is no bridge operator. That answers the Serum failure and the
standing weakness of rollups and solver networks. It does not bring a market maker, a bank,
a user or a lawyer, and those are what the other five lacked.

So the honest claim for JAM is narrow: it is a base on which "no key controls more than one
person's money" can be true without an asterisk. Whether anyone comes to trade is decided
elsewhere.

### The security failures

The table above covers the projects studied first, and only one of them was stopped by an
attack. The wider field has seven security or operational failures, and for those the
question has a different answer. It is still an inference.

| Failure | What gave way | Does this design have that part? | What remains |
|---|---|---|---|
| Drift | An administrator multisig that could act at once. | No, if the upgrade rule is settled as lesson 7 says: nobody, or a public delay. Any chain allows that choice. | The service's own code. |
| THORChain, May 2026 | The threshold key of a vault that held users' bitcoin. | No. Nothing here holds bitcoin: the seller pays the buyer on Bitcoin and proves it. Checking that proof on-chain is what removes the vault. JAM makes the check cheap; Atomiq did the same on two other chains. | No pool. A counterparty with inventory must be present for every trade. |
| Maya, Garden | Off-chain signers and the databases of solvers. | No signer set. Makers hold their own inventory under their own keys. | A maker's hot wallet is still a hot wallet. It loses the maker's funds, not the users'. |
| Paradex | An operator's database set a price, and the operator rolled the chain back. | No operator can set a price or roll anything back. | Pegged orders (lesson 5) bring a price reference back in. It has to be signed by its source and bounded by the order's own limit, and must never move funds by itself. |
| Boltz, Atomiq | One small team ran the service's servers and could not defend them. Users' funds were never at risk. | Partly. Anyone can bundle and anyone can quote, so the service does not stop when one team does, provided a second one exists. | Somebody still runs bundlers, quote sources and a front-end, and they will be attacked. |
| Hyperliquid (a standing risk, not a failure) | 27 validators, five of them with about half the stake, behind the bridge that holds every deposit. | JAM is designed for about a thousand validators whose work is audited, and nothing here is bridged. | JAM's main network does not exist yet. Its validator set is a promise. |

In five of these (Drift, THORChain, Maya, Garden, Paradex) the part that broke is one this
design does not have: a key that could move pooled funds, or an operator's server that set
prices or held inventory. That is the strongest thing to be said for it, with two
qualifications.

What removes the part is the design, not the chain: verify a payment instead of holding
the asset. JAM makes that cheap and does not own it, and its price is that there is no
pool, so liquidity has to be a counterparty who is present.

And the risk does not vanish. It moves into the service's code, which holds every token
balance and every bond. One review found sixteen flaws in this code
([SECURITY.md](SECURITY.md)). The two teams that closed in August 2026 were not robbed
through a vault; attackers found flaws faster than they could patch them. A service with
no upgrade key cannot be patched at all, and one with a delayed key announces the flaw
before the fix. That choice is still open here ([ROADMAP.md](ROADMAP.md), section 4.1).

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

The wider field (section 6): DefiLlama API series for perps, spot venues, aggregators, TVL
and stablecoins, and CoinGecko supplies, pulled 16 November 2026;
[CoinDesk Data exchange review, September 2026](https://coindesk.com/research/exchange-review-september-2026);
[The Block on Hyperliquid's share of perps](https://www.theblock.co/post/403384/hyperliquid-record-share-global-perps-market-hip-3-tops-62-billion-monthly-volume) (3 Jun 2026);
[Galaxy Research on the July 2026 oracle liquidations](https://www.galaxy.com/insights/research/hyperliquid-tradexyz-oracle-liquidations);
Hyperliquid's validator API (`api.hyperliquid.xyz/info`);
[crypto.news on Uniswap's fee switch](https://crypto.news/uniswap-fee-switch-defi-governance/) (Aug 2026);
[Cointelegraph on THORChain's restart](https://cointelegraph.com/news/thorchain-resumes-trading-month-10m-exploit) (23 Jun 2026) and [crypto.news on its signature scheme](https://crypto.news/thorchain-faces-backlash-over-gg20-fix-after-10-7m-hack/);
[crypto.news](https://crypto.news/blockstream-launches-swaps-service-after-boltz-suspends-operations/) and [bitcoinnews.com](https://bitcoinnews.com/p/boltz-new-veteran-bitcoiners-take-over) on Boltz (Aug 2026);
[Decrypt on NEAR Intents](https://decrypt.co/379822/near-intents-hacked-days-after-denying-bitget-hacker) (1 Oct 2026);
[Cointelegraph on Garden](https://cointelegraph.com/news/garden-finance-app-offline-450k-htlc-exploit) (27 Jul 2026);
[Bisq's post-mortem](https://bisq.network/blog/security-incident-post-mortem/) (16 May 2026) and its markets API;
[Mariblock on Paxful's closure](https://mariblock.com/paxful-shuts-down-blames-ex-co-founders-youssef-and-schaback) (8 Oct 2025);
[Hydration, "The way ahead"](https://hydration.substack.com/p/the-way-ahead) (9 Jun 2026);
[Step Data](https://stepdata.substack.com/p/prop-amms-on-solana-bisonfi-ends) and [SolanaFloor](https://solanafloor.com/news/solana-prop-amm-volume-share-drops-below-50-first-time-since-november-2025) on Solana's private market makers.
Known only from news summaries, not from the pages themselves: the Drift theft and its
multisig (The Block, 2 Apr 2026), the Paradex rollback (The Block, 19 Jan 2026), the Maya
halt (Decrypt, Aug 2026), the THORChain lending insolvency (The Block, 3 Feb 2025), the
points-driven falls of Lighter, Aster and edgeX, and the central bank's stablecoin figure.

Added in a second pass on 16 November 2026: tBTC's holdings and the volume of Uniswap's
own chain (DefiLlama API); the share of stock and commodity perpetuals
([an analytics post](https://www.buildix.trade/blog/hyperliquid-rwa-perps-market-share-binance-trade-xyz-2026), October 2026, citing a data provider; not confirmed elsewhere);
press reports that put the foundation's share of Hyperliquid's stake at 49% in June 2026
([Crypto Briefing](https://cryptobriefing.com/hyper-foundation-staked-hype-centralization/)).

Not verified: the date OpenBook v2 reached mainnet; Hyperliquid's current validator count
(sources give 4, 16 and 24 to 27); whether Atomiq has restarted (one September 2026 article
describes it as live, its escrows hold about $40).
