# What stands between this prototype and people using it

This note answers one question: if the design works, why would it still not be used, and
what would have to happen in technology, in the market and in law before it is?

It rests on three pieces of research done on 16 November 2026: the state of non-custodial
venues ([PRIOR-ART.md](PRIOR-ART.md)), why centralised exchanges still dominate, and the
legal position of a design like this one in Brazil, the EU and the US. Sources are at the
end. Three warnings:

* The research was done by automated agents in one day. Figures from DefiLlama and laws
  read in their own text are reliable; several 2026 events are known only from press
  summaries, and the source list says which.
* Nothing here is legal advice. The legal section says what the rules appear to be and
  where they are silent. The silences are the important part and need a lawyer.
* "Inference" marks what is reasoned rather than sourced.
* Where a rule is given with an article number, that article was read in the central
  bank's own text on the same day.

## 1. The short version

1. **Technology is not what is missing.** Of about twenty non-custodial venues that failed
   or shrank, none failed mainly because of a limit of the chain it ran on. They failed on
   keys and operations, on incentives that ran out, on demand, and once on regulation.
2. **JAM changes who must be trusted, and nothing else.** That is worth having: the largest
   on-chain losses of 2026 were an administrator multisig with no time lock and a handful of
   small teams whose off-chain signers were attacked. It does not bring a bank, a market
   maker, a user or a licence.
3. **The fiat leg decides everything, and it is a legal object.** In Brazil there is no rule
   yet for issuing a real-denominated token, trading fiat-referenced tokens is treated as
   foreign-exchange activity, and since 6 November 2026 banks may not deal with unlicensed
   crypto providers. Whether a venue with no operator counts as a provider is not settled.
4. **The Pix leg as built does not survive Brazilian practice.** A Pix can be contested long
   after it arrives, the receiving account is blocked and flagged, and a peer-to-peer seller
   who did not identify his buyers has been convicted of laundering. An arbiter can say that
   a Pix arrived; nobody can say that it will stay.
5. **So the realistic first shape is not "an exchange for everyone".** It is settlement
   infrastructure behind one licensed institution that issues the real token, identifies the
   users and brings a market maker, for one flow where waiting a few seconds does not
   matter: buying bitcoin into one's own wallet.
6. **The second flow is probably the larger one.** The trade that already avoids exchanges
   in Brazil is companies dealing in dollar stablecoins, roughly R$11 billion a month. It
   needs dollars to be reachable the way bitcoin is here: by proof, without bringing them
   onto JAM.

## 2. Why exchanges still dominate

**The numbers, October 2026.** On-chain venues did 17% of spot volume and 15% of
perpetuals volume. Derivatives are 79% of what centralised exchanges trade. One exchange has
26% of all exchange spot volume and 46% of derivatives. The on-chain share of spot was 5%
just before FTX failed, peaked at 37% in June 2025 during the memecoin wave (by one
provider's measure) and has been between 17% and 25% since, while on-chain volume itself
fell by about half. An estimated 716 million people own crypto and 40 to 70 million are
active on-chain; services hold 68% of all balances.

**Brazil.** Volume declared to the tax authority was R$510 billion in 2025 and R$283 billion
in the first half of 2026. Stablecoins were 92% of it in that half. In June 2026 bitcoin was
4.6%, about R$2.4 billion, against R$48 billion of dollar stablecoins. Of declared value,
62% went through Brazilian exchanges, 13% through foreign ones and 25% through none, and
that last quarter, roughly R$11 billion a month, is almost entirely companies. In a May
2026 survey 29 million adults had held crypto: 83% of them through a bank's app, 26% in a
wallet of their own, 20% on an exchange; people who hold their own keys are 4% of the
population. Net purchases of
stablecoins abroad fell from $2.5 billion in June 2026 to $0.6 billion in July as the new
rules took hold, and at least seven exchanges closed or cut their retail business that year.

**Why people stay, in order of the strength of the evidence.**

1. *The way in is a bank or an exchange.* 83% entered through a bank's app; three quarters
   of declared value runs through exchanges.
2. *People do not want keys, and mostly do not know who holds them.* In a study of users
   after FTX, 35% knew that the exchange held their keys, 71% moved nothing, and of the
   custodial-only users who did move, 83% went to banks.
3. *Derivatives.* Four fifths of exchange volume, and 85% of perpetuals still trade there.
4. *Institutions are told to.* 81% of institutions with spot exposure prefer an
   exchange-traded product to an exchange or a wallet.
5. *Regulation pushes the same way* (Brazil, above, and section 4.3).
6. *The big thefts cost their customers nothing.* Bybit ($1.5 billion, February 2025),
   Bitget ($0.4 billion, September 2026) and Binance ($6.3 billion, 29 October 2026) were
   absorbed by the operators. Binance's outflow afterwards was about 3% of its stated
   assets and turned positive within a week. Four days later a large Brazilian exchange
   lost $40 to $49 million from its hot wallets (estimates differ) and said that no
   customer would lose anything. In the same year a firmware flaw in a popular
   hardware wallet drained 1,300 to 1,800 BTC from people who held their own keys, and no
   reimbursement was reported.

**What each choice has cost.** Gross losses from 2022 to 2026: about $22 billion at
custodians (FTX, Celsius, Bybit, Bitget, Binance; others not compiled) and about $10 billion
on-chain (exploits, thefts from personal wallets, drainers). Custodians hold about twice
the balances, so the rate per dollar is of the same order. The difference is who pays:
large solvent exchanges made customers whole, insolvent ones took years, and on-chain
victims are rarely repaid. On this evidence neither is clearly safer for an ordinary
person. The totals are floors and the denominators are rough.

**Where non-custodial trading did win.** On-chain perpetuals: a product exchanges could not
offer to everyone, 31% of the venue's token given to its users, and wallets paid for the
order flow they bring. Memecoins on Solana: assets exchanges did not list, traded through
one terminal. Stablecoin payments. And, tellingly, on-chain trading inside an exchange's
own wallet app, which had 95% of wallet swap volume in May 2025. One counter-case: a
prediction market's regulated US venue overtook its own on-chain venue within a year.

Two cautions from the same year. A win lasts until an exchange copies it: after two large
exchanges listed stock and commodity perpetuals inside their own accounts, the on-chain
leader's share of that product was reported to fall from 80% to 26% within weeks (one
analytics source). And a chain of one's own is not distribution: the largest spot venue's
own chain went from $12.4 billion a month in August 2025 to $0.4 billion, while the same
venue did $14.6 billion on a broker's chain four months after that chain opened.

**What follows** (inference).

* *Safety does not sell to the customer.* Three exchange thefts in twenty months each moved
  a few percent of assets, briefly. "No key holds everyone's money" matters most to the
  institution that would otherwise hold that key, and to its supervisor.
* *The on-ramp is the product.* A non-custodial venue gets used when a bank's app reaches
  it in one step.
* *Custody has to be invisible and forgiving.* That is an argument for the account rules
  built here (recovery, a guardian, limits), and a reminder that holding one's own keys
  has disasters of its own. The same rules answer coercion: someone forced to sign can
  move no more than the daily limit, and raising the limit waits out the account's delay.
* *Somebody stands behind losses at an exchange.* A venue with no operator must either
  make losses much less likely and show why, or find someone to insure them.
* *Every non-custodial win came from a product nobody else had, plus paid distribution.*
  None came from an argument about safety.
* *In Brazil the flow is dollars, not bitcoin*: twenty to one.
* *The trade that already avoids exchanges is corporate.* A quarter of declared value goes
  through no exchange, and nearly all of it is companies dealing in dollar stablecoins
  with each other or through desks. Those trades settle on trust: one side sends first. A
  venue that settles them delivery against payment solves a problem they have today, and
  its customers are identified already.

## 3. What JAM does and does not change

See the table in [PRIOR-ART.md](PRIOR-ART.md), "Would a better chain have saved them?".
In one line each:

| The problem | Does JAM change it? |
|---|---|
| An operator, sequencer, solver set or upgrade council that must be trusted | Yes. Computation is re-run by randomly chosen auditors; the code has no key unless it gives itself one. |
| A bridge operator between chains | Yes, for reading: another chain's proofs are checked by ordinary code. No, for writing: Bitcoin cannot be made to check JAM. |
| The cost of updating a quote on-chain | Yes. A quote is a signed message off-chain until it trades. |
| Latency | No. Six-second blocks, twelve to eighteen seconds to settlement. Worse than the venues market makers use. |
| Liquidity | No. A new chain starts with none. |
| Off-chain components that can be attacked | Partly. Nothing off-chain here holds funds or keys that move them (bundlers and header carriers are replaceable by anyone), except oracles on the Pix rail. |
| A flaw in the service's own code | No. Removing keys and operators moves the risk there: the code holds every token balance and every bond. See "The security failures" in PRIOR-ART.md. |
| Fiat | No. A real is a bank's liability on any chain. |
| Law | No. |

## 4. The path

### 4.1 Technology

In order. Items 1 to 6 are needed for a pilot; the rest only once there is something to
scale.

1. **Outside this project:** a JAM main network, and a market for core time with a known
   price. Until then nobody can say what a round costs. Run on the public test network,
   across implementations, as soon as one exists.
2. **Hardening.** An independent audit, fuzzing of every field a counterparty supplies, a
   bug bounty. One automated review found sixteen flaws in a version its author thought
   correct ([SECURITY.md](SECURITY.md)); assume there are more. Decide the upgrade rule for
   real: no administrator, or a public body with a delay longer than the longest deal. The
   2026 record is plain about what kills venues: an administrator key without a time lock
   (about $285M at one Solana venue) and small teams whose off-chain signers or solvers
   were attacked until they gave up. It is just as plain about holding one's own keys: a
   firmware flaw in a hardware wallet cost its users over a thousand bitcoin. The account
   rules here (limits, a guardian, recovery) are the answer to that, and need a wallet
   that uses them (item 6). Keep the property that nothing off-chain here can lose
   anyone's funds, and do not add a component that breaks it.

   One thing is not decided: what happens on the day a flaw is found in the service
   itself. Two swap services closed in August 2026 saying that attackers, helped by AI,
   find flaws faster than a small team can patch them. With no upgrade key a flaw cannot
   be fixed; with a delayed one it is public before the fix lands. The usual answer is a
   brake that can only stop new rounds and new deals, cannot move or freeze anything, and
   lapses by itself. It is not built.
3. **What a licensed issuer will ask for.** Already there: an allowlist per asset, a logged
   freeze, attested reserves. Missing: statements a user can hand to the tax authority;
   limits and reports at the thresholds the central bank sets; a way for the issuer to see
   what it must see without everyone seeing everything (item 9); and an answer to a court
   order. Brazilian courts seize bank balances electronically every day. Here an issuer
   can freeze a balance of its own token and cannot move it. That was meant as a selling
   point and is also a gap, because a regulated issuer has to be able to comply. The
   narrow form is a property of the asset, fixed and public when the asset is created: a
   frozen balance may be cancelled and re-issued under a logged order. It must not exist
   for an asset whose issuer is not its debtor, and it cannot exist for bitcoin, which the
   service never holds.
4. **What a market maker and a distributor will ask for.** For the maker: orders pegged to
   a reference price, and cancels that take effect before the round they would trade in.
   Without these a professional has no reason to quote into a six-second round. For the
   distributor: a share of the fee. Today the whole fee of a round goes to whoever bundled
   it, so an app that sends its users' orders into a shared round earns nothing. A field
   in the order naming where it came from, signed and capped by the user, lets one round
   pay several apps. The largest on-chain venue pays wallets this way, and that is how it
   reached retail users without a retail brand. (In Brazil the app that takes such a fee
   is an intermediary and needs the licence: section 4.3.)
5. **Fees and deposits.** Today the service pays for everybody's storage. Before any public
   use it needs a deposit per account and per deal, and a fee per round that covers core
   time.
6. **Keys people can live with.** The account rules (second device, guardian, recovery,
   limits, delay) exist in the service. A wallet that uses them does not, and phones sign
   with P-256, not Ed25519, so passkeys need a second signature scheme in refine. This is
   in the pilot on the evidence of section 2: four percent of people hold their own keys,
   and a pilot that hands its users a seed phrase tests the wrong thing.
7. **Bitcoin over Lightning, by hash lock.** A purchase settled on Bitcoin's chain waits
   for confirmations, half an hour to an hour. For small amounts there is a faster rail
   that needs no header chain: the buyer's wallet issues a Lightning invoice, the buyer's
   tokens are locked here against the invoice's payment hash, the seller pays the invoice,
   and the secret that Lightning hands the payer on settlement is what releases the
   tokens. The receipt is 32 bytes and one hash. It is the same deal with a different
   proof, and the smallest addition on this list. Its known costs: the buyer can stall
   until the invoice expires (the free option of every atomic swap, bounded by a short
   expiry), receiving on Lightning without a custodian is still awkward, and the seller's
   node is a hot wallet of its own.
8. **Dollars by proof.** Nine tenths of the Brazilian flow is dollar stablecoins, and they
   live on Ethereum and Tron. They can be reached the way bitcoin is: the dollars never
   enter JAM; the seller delivers them on their own chain and proves it here. That needs
   a light client for each chain inside the service (for Ethereum the sync committee's
   aggregate signature and a receipt proof; for Tron the signatures of its 27 block
   producers). Large, but engineering and not research, and it removes the need for a
   dollar issuer on JAM. The trust is weaker than for bitcoin: these light clients
   believe a committee of 512 validators or 27 producers, not a chain of work.
9. **Privacy.** Balances and orders are public. The article calls proving a balance
   without revealing it the condition for a bank to take part, banking secrecy law points
   the same way, and the central bank's own ledger pilot was switched off with that
   problem unsolved (as reported). It is not a condition for the pilot: the real tokens
   that exist today all live on public chains. It is the gate to institutions trading
   their own books here. This is research, not engineering. Refine is the natural place
   to check the proofs, at a price: by the ratio of native running times, checking one
   pairing-based proof costs as much as some tens of signatures (an estimate, not
   measured here), so privately funded orders would be tens per work-package, not
   hundreds, unless the proofs are aggregated.
10. **Throughput.** About 290 orders per work-package per core now; a fixed-layout account
    record should reach about 1,000, packed balances several thousand
    ([THROUGHPUT.md](THROUGHPUT.md)). Not the constraint until there is volume. The header
    chain needs the same treatment (127,000 gas per header today).

### 4.2 The market

"Marketing" here means three things: what it is for, how it reaches people, and where the
liquidity comes from. The evidence on each is in section 2 and in PRIOR-ART.md.

1. **Do not compete where exchanges are strong.** Major pairs, perpetuals and anything
   where milliseconds decide belong to venues with their own ordering rules or to private
   market makers. Public order books are a tenth of volume even on the chain best suited
   to them.
2. **Pick a flow that does not mind a few seconds and does mind custody.** Four
   candidates, in order of how soon they are reachable:
   * *Bitcoin bought into one's own wallet with a bank's real token.* The peer-to-peer
     venues that do this today are tiny (the largest about $7M a month) because their fiat
     leg is slow, reversible and legally exposed. A bank token removes exactly that. The
     whole declared bitcoin market in Brazil is about R$2.4 billion a month. This is the
     flow the code can serve today: bitcoin needs no bridge, only headers (and for small
     amounts not even those: section 4.1, item 7).
   * *Companies that already trade without an exchange.* Roughly R$11 billion a month,
     nearly all of it dollar stablecoins, between identified counterparties who today
     send first and trust the other side. Settlement of delivery against payment is a
     product for them, and the weakness of an arbiter between strangers does not arise.
     It needs dollars reachable by proof (section 4.1, item 8).
   * *Exchange between bank tokens* (reais for dollars), on institutions' own books.
     This is where the money is: about R$48 billion a month of dollar stablecoins, which
     the central bank now treats as foreign exchange. A venue where licensed institutions
     meet with no custodian between them is a business-to-business product. It needs
     them to accept a public ledger, or privacy (section 4.1, item 9).
   * *Large trades* that want one price and no front-running more than they want speed.
3. **Borrow distribution; do not build a retail brand.** The pattern is consistent: a
   broker's new chain gave the largest spot venue $14.6B a month within four months; on
   Solana one aggregator sends nine trades in ten to private market makers; venues that
   bought users with points lost 75% to 99% of their volume when the points ended. Be a
   route inside an app people already have, a bank's or a broker's, and let that
   institution own the customer. Pay it from the protocol: a share of the fee for the app
   that brings the order (section 4.1, item 4). And start with the issuers that already
   have a real token. There are about eight, with reserves, with licences or applications
   for them, and with almost no market for their token against bitcoin that is not
   somebody's custody. Offer them a shared venue, not a rival token.
4. **Give the institution a reason.** Inference, to be checked with a lawyer: under the
   central bank's 2025 rules a provider that intermediates without custody needs less
   capital than one that also holds clients' assets (the press reports R$10.8M against up
   to R$37.2M), and it does not become the target that every custodian is. "Offer trading
   without holding the customers' coins" is a cost and risk argument, not an ideology.
   A second reason, for an issuer: every real token held here is a real in its reserves.
   Who may be that institution is a closed list (section 4.3).
5. **Bring the liquidity.** One named market maker, the partner's own desk or a hired firm,
   quoting from the first day and paid in fees. What worked elsewhere was an in-house maker
   run openly. Emissions bought the Polkadot bitcoin bridge 90 BTC and a worthless token.
   Pay the maker in fees, not in the interest on the reserves: a token that passes yield
   to whoever holds it invites the securities regulator.
6. **Sell the one true thing, to the people it matters to.** Publish, for every key in
   the system, what it can move; add the audit, the bounty and the upgrade rule. The
   evidence in section 2 is that this does not move retail customers, who were made whole
   after each theft. It does matter to whoever would otherwise hold the key: in the last
   two years that was the exchanges that replaced $1.5 billion, $0.4 billion and $6.3
   billion out of their own pockets, and the venue that lost $285M through an
   administrator multisig and did not come back. The audience is the institution and its
   supervisor. For the customer the message is the product: bitcoin in your own wallet,
   from your bank's app, with a way back if you lose the phone.
7. **A tax argument exists, and is weaker than it looks.** Gains on disposals of up to
   R$35,000 a month are exempt when the sale is made in Brazil; crypto held with a foreign
   entity is taxed at a flat 15% with no exemption, and the measure that would have ended
   the exemption lapsed in 2025. So a resident who buys and sells here should keep an
   exemption that the customer of an offshore exchange has lost. Three cautions. It is
   parity with Brazilian exchanges and bank apps, where four people in five already are,
   not an edge over them. The tax authority has not said clearly how it treats coins in
   one's own wallet, and guides disagree. And a 3.5% tax on purchases of crypto-assets,
   proposed in February 2026, was undecided at the time of writing; it would fall hardest
   on reais exchanged for dollar tokens. Nothing for an advertisement until a tax lawyer
   has signed it.
8. **Measure honestly.** Real-denominated tokens total about R$1 billion today. A pilot
   that settles R$10M a month with no incident, a spread on the main pair within twice an
   exchange's, and a second institution asking to join would be success. If after six
   months the spread is wider than that, makers have not come and nothing else matters.

### 4.3 Law, Brazil first

What the rules appear to be (details and sources in section 6), and what follows for this
design. The second column is inference throughout.

| Rule, as reported | What it means here |
|---|---|
| Providers of virtual-asset services need central-bank authorisation; intermediation "on behalf of third parties" is licensed even without custody, and order-book-like mechanisms are regulated (Lei 14.478/2022; Resoluções BCB 519 to 521, in force 2 February 2026; intermediation is defined in article 7 of Resolução 520 as buying, selling and exchanging "por conta de terceiros"). | A company that runs the front-end, or bundles orders for a fee, most likely needs the intermediary licence. Its only argument otherwise is that it executes nothing on anyone's behalf. Unsettled; no case, no statement. |
| Protocols with no operator are not addressed. | The silence is not permission. The first question for a lawyer. |
| Since 6 November 2026 financial institutions, payment institutions and other authorised institutions may not carry out or enable operations in the virtual-asset market whose counterparties are entities that provide virtual-asset services without authorisation. The ban reaches accounts, payment transactions and foreign exchange used for that purpose (Resolução BCB 520, article 91, as worded by Resolução 589). | If the venue, or a bundler, is "an entity that provides virtual-asset services" without authorisation, no bank can issue a token for it, keep an account for it or let customers reach it. The issuer's lawyers will ask this before anything else. |
| Besides the new virtual-asset companies, only commercial, exchange, investment and universal banks, the federal savings bank, securities brokers and dealers, and foreign-exchange brokers (intermediation only) may intermediate or hold virtual assets. They may start 90 days after a formal notice with an independent technical certification, and must have been authorised for a year (Resolução BCB 520, articles 20 and 21). Payment institutions are not on the list. | The institution at the front door must be one of these, or hold the new licence. A payment-institution licence alone is not enough. |
| Buying, selling or swapping fiat-referenced virtual assets, and transfers to or from a self-custody wallet, are foreign-exchange-market activity; third parties' funds may be moved this way only in the service of an institution authorised for foreign exchange (article 76-A of Resolução BCB 277, inserted by Resolução 521). Real-referenced tokens are not exempted. | Trades in the real token may be foreign-exchange operations needing an authorised institution and its reporting. Another reason the front door is a licensed firm. |
| There is no regime for issuing a real token. The central bank calls stablecoins "privately issued money", between e-money and deposits; a bill (PL 4.308/2024) is in committee; the central bank's own ledger pilot was switched off. Several real tokens exist, one issued by a licensed bank. | The token this design needs exists in practice and not yet in law. Its issuer decides where it may trade. |
| Self-custody is allowed. Providers must identify the owner of a self-custody wallet they deal with (the same article 76-A); transfers of US$10,000 or more are reported; from 1 January 2027 withdrawals above US$10,000 a day to self-custody are held for 24 hours. | An account here is a self-custody wallet. The issuer's allowlist is the identification. The thresholds become product limits. |
| A Pix can be contested (the refund mechanism now traces beyond the first account); the receiving account is blocked and the receiver flagged; the press reports up to 80 days. A federal court sentenced a peer-to-peer seller to five years for laundering after R$35M of Pix receipts from unidentified buyers. Since May 2026 lending a bank account for criminal proceeds to pass through is a crime of its own (Lei 15.397/2026). | The Pix rail with an arbiter is not safe for the party who releases value against a Pix from a stranger, and exposes the arbiter. Fiat should enter only through the issuer, which takes the Pix under its own identity checks and carries that risk as any bank does. Keep the oracle rail for counterparties who are identified. |
| Courts seize bank balances electronically, and a regulated issuer must comply. Nothing was found on how such an order reaches a token in a self-custody wallet. | Today the issuer here can freeze and cannot transfer. Decide and publish, per asset, whether a frozen balance can be cancelled and re-issued under an order (section 4.1, item 3). |
| A token that pays yield, or a product that behaves like a derivative, can be a security under the securities regulator's guidance (CVM, Parecer de Orientação 40). | No yield on the token and no derivatives in the first stages, so that one regulator is at the table and not two. |
| Residents who trade through a decentralised platform report it themselves above R$35,000 a month (IN RFB 2.291/2025, in effect since July 2026). Gains on disposals of up to R$35,000 a month made in Brazil are exempt; crypto held with a foreign entity is taxed at 15% with no exemption (Lei 14.754/2023). How coins in one's own wallet are treated is not clearly stated, and a 3.5% tax on crypto purchases proposed in February 2026 was undecided at the time of writing (press and tax guides, not the tax authority's own text). | The product must give users the statement they need. The exemption is a possible selling point against offshore exchanges, not against Brazilian ones (section 4.2, item 7). The proposed tax is a risk to the second flow. |
| Abroad, developers of non-custodial software were convicted where they kept control, took fees and knew of misuse; a US staff statement shelters interfaces that hold no keys and route nothing. Nothing Brazilian either way. | No upgrade key held by the developers at launch, or a governed one with a long delay. No fee from the flow to the developers. The developers do not run the public front-end. |
| In the EU only an EU bank or e-money institution may offer an e-money token; the exclusion for "fully decentralised" services is under consultation. | A real token cannot be offered there. Brazil first. |

What to do, in order:

1. **Get a written legal opinion on five questions:** is a bundler or a front-end a
   "prestadora" when it never holds assets and cannot refuse or reorder; are trades in a
   real token foreign-exchange operations; who carries the reversal and laundering risk of
   a Pix when nobody is the intermediary; how a court order is executed against a token in
   a self-custody wallet; and how a sale from one's own wallet is taxed.
2. **Find the issuer first.** One of the institutions on the closed list above, or a
   provider already authorised; start with those that already issue a real token. The
   design gives it what it needs to say yes (allowlist, freeze log, attestations) and it
   gives the design what it cannot have alone: a lawful real, identified users, and a
   licence at the front door.
3. **Ask the regulator in the open**, through the issuer: a formal consultation or a
   sandbox entry describing exactly who holds which key. The answer to "is this a provider"
   is worth more than any feature.
4. **Take part in the stablecoin bill and the central bank's agenda** while the rule for
   real tokens is still unwritten. The property to argue for is narrow: an issuer that can
   freeze its own liability, can cancel and re-issue it only under a logged order, and
   can touch nothing else.
5. **Remove the developers from the path** before launch: governance or nobody holds the
   upgrade key; the licensed partner runs the front-end.

## 5. An order of work

| Stage | What is done | The gate to the next |
|---|---|---|
| 0. Now | Legal opinion; conversations with two or three issuers; independent audit; public test network. | An issuer's lawyers say the venue can lawfully carry its token. |
| 1. Pilot | One issuer, its identified customers, one market maker, one pair (bitcoin for the real token), a wallet with passkeys and recovery, fees and deposits in place, statements for users. Lightning for small amounts if the pilot's users ask for it. | Six months without an incident; spread within twice an exchange's. |
| 2. Dollars | Dollar stablecoins for the real token, delivered on their own chain and proved here; companies and desks that already trade without an exchange; pegged orders and cancel-first; a fee share for the app that brings the order; open bundling; a second issuer. | A second institution joins for its own reasons. |
| 3. Institutions | Privacy of balances and orders; exchange between bank tokens on institutions' own books; throughput. | |

## 6. What would show the idea is wrong

* No licensed issuer will allow its token to trade where no licensed operator stands. Then
  the fiat leg does not exist and the rest is a bitcoin swap service competing with venues
  that have all closed or been attacked.
* The regulator answers that bundlers are providers and each needs a licence. Then "anyone
  can bundle" is over, and the design reduces to a licensed exchange with unusual plumbing.
* No market maker quotes a tight price into six-second rounds even with pegged orders.
  Then the venue is correct and empty, like the best public order book on Solana.
* A tax lands on exchanging reais for dollar tokens (3.5% was proposed). Then the dollar
  flow stays where it is and only the bitcoin flow is left.
* JAM's main network or its core-time market is years away. Then the same design on
  another base has to be judged on what it loses there (see the explainer's comparison).

## Sources

Research date for all three: 16 November 2026. "Summary only" means the researcher saw a
search engine's summary of the page and not the page.

**Market structure and why exchanges dominate**
* CoinDesk Data exchange reviews, [October 2026](https://www.coindesk.com/research/exchange-review-october-2026) and [September 2026](https://coindesk.com/research/exchange-review-september-2026): shares of spot and derivatives.
* CoinGecko, [DEX to CEX ratio](https://www.coingecko.com/research/publications/dex-to-cex-ratio) (updated April 2026): the 2022 to 2025 series.
* a16z, [State of Crypto 2025](https://a16zcrypto.com/posts/article/state-of-crypto-report-2025/): owners against active users.
* Chainalysis, [Latin America 2026](https://www.chainalysis.com/blog/latin-america-crypto-adoption-2026/) and [2026 Global Adoption Index](https://www.chainalysis.com/blog/2026-global-crypto-adoption-index/).
* Receita Federal, [open data on crypto-assets](https://www.gov.br/receitafederal/pt-br/assuntos/orientacao-tributaria/declaracoes-e-demonstrativos/criptoativos/arquivos/criptoativos_dados_abertos_20260826.pdf) (26 August 2026): declared volumes, asset and channel shares; the percentages are the researcher's arithmetic on its tables.
* InvestNews on the [Datafolha survey](https://investnews.com.br/investimentos/brasileiro-investe-mais-em-criptomoedas-do-que-em-cdbs-e-acoes-mostra-datafolha/amp/) (August 2026; sponsored by an exchange and an asset manager), on [stablecoin purchases after the new rules](https://investnews.com.br/investimentos/compra-de-stablecoins-no-exterior-recua-80-apos-novas-regras-do-bc/amp/) and on [exchanges closing](https://investnews.com.br/investimentos/por-que-tantas-exchanges-de-criptomoedas-estao-fechando-no-brasil/amp/).
* [Study of user behaviour after FTX](https://doi.org/10.1145/3772318.3791341), CHI 2026 (US sample of 430).
* EY-Parthenon and Coinbase, [institutional survey](https://www.ey.com/en_us/financial-services/institutional-digital-assets-survey) (January 2026).
* Reuters via Khaleej Times on the [Binance theft and outflows](https://www.khaleejtimes.com/business/binance-sees-outflows-stabilising-as-exchange-returns-online-after-63-billionheist) (2 November 2026) and [Cointelegraph's account](https://cointelegraph.com/news/recap-binances-record-63b-hack-unfolded); figures are preliminary and partly the company's own.
* BleepingComputer on [Bitget](https://www.bleepingcomputer.com/news/security/hackers-steal-3516-million-in-bitget-crypto-exchange-hack) (September 2026); The Block on the [hardware-wallet firmware flaw](https://www.theblock.co/amp/post/410515/okx-says-coldcard-exploit-record-inflows-centralized-exchanges) (August 2026).
* Loss totals: [Immunefi](https://immunefi.com/blog/research/the-ecosystem-vulnerability-scoreboard-6-years-of-defi-loss-data/), [CoinGecko on hacks 2016 to 2026](https://www.coingecko.com/learn/crypto-hacks-and-exploits-2016-to-2026), [The Block on Chainalysis theft data](https://www.theblock.co/amp/post/382477/crypto-hack-2025-chainalysis).
* Added in a second pass on the same day: DefiLlama for the monthly volume of the largest spot venue's own chain; an analytics post on [the on-chain share of stock and commodity perpetuals](https://www.buildix.trade/blog/hyperliquid-rwa-perps-market-share-binance-trade-xyz-2026) (October 2026; one source, citing a data provider); press on the theft at a Brazilian exchange on 2 November 2026 ([Tecnoblog](https://tecnoblog.net/noticias/mercado-bitcoin-pode-ter-perdido-r-217-milhoes-em-ataque-hacker/), [Folha de Pernambuco](https://www.folhape.com.br/economia/mercado-bitcoin-criptomoedas-hacker/521962/); summaries only).
* Not found by the researcher: 2026 market shares of Brazilian exchanges, the share of funding that arrives by Pix, the share of on-chain venues in Brazil, like-for-like trading costs at several trade sizes, and the share of fiat that enters through exchanges.

**Non-custodial venues:** see the source list of [PRIOR-ART.md](PRIOR-ART.md).

**Law**
* Brazil: [Lei 14.478/2022](https://www.planalto.gov.br/ccivil_03/_ato2019-2022/2022/lei/l14478.htm) (read from a mirror); Resoluções BCB [520](https://www.bcb.gov.br/estabilidadefinanceira/exibenormativo?tipo=Resolu%C3%A7%C3%A3o%20BCB&numero=520) and 521 of 10 November 2025, and 561, 584, 589 and 591 of 2026, read on the central bank's site (same address with the number changed); the central bank's votes [90/2026](https://normativos.bcb.gov.br/Votos/BCB/202690/Voto_do_BC_90_2026.pdf) and [108/2026](https://normativos.bcb.gov.br/Votos/BCB/2026108/Voto_do_BC_108_2026.pdf); the central bank's [slides on the Pix refund mechanism](https://www.bcb.gov.br/content/estabilidadefinanceira/pix/MED/MED_2-0_Circuito_Pix-Dia_1.pdf) (October 2025); CVM [Parecer de Orientação 40](https://conteudo.cvm.gov.br/export/sites/cvm/legislacao/pareceres-orientacao/anexos/Pare040.pdf); [IN RFB 2.291/2025](https://www.normaslegais.com.br/legislacao/instrucao-normativa-rfb-2291-2025.htm) (a copy).
* Brazil, read in the central bank's text in a second pass: articles 7, 20, 21 and 91 of Resolução BCB 520 and article 76-A of Resolução BCB 277 as inserted by Resolução 521. [Lei 15.397/2026](https://www.planalto.gov.br/ccivil_03/_Ato2023-2026/2026/Lei/L15397.htm) (summary only).
* Brazil, tax, from summaries of the press and of tax guides, not from the tax authority's text: the monthly exemption and the rule for assets held abroad ([Estado de Minas](https://www.em.com.br/trends/2026/03/amp/7368075-como-declarar-bitcoin-e-outras-criptos-no-imposto-de-renda-2026.html), [a guide to investments abroad](https://netcpa.com.br/colunas/receita-cobrara-ir-sobre-resgates-de-aplicacoes-no-exterior-abaixo-de-r-35-mil/28264)); the unsettled treatment of self-custody ([a study guide](https://www.estrategiaconcursos.com.br/blog/tributacao-criptoativos-brasil-2026/amp/)); the proposed tax on purchases ([Forbes Brasil](https://forbes.com.br/coluna/2026/02/governo-quer-taxar-criptomoedas-com-iof-o-que-isso-muda-para-o-investidor/), February 2026) and a [ruling of 6 November 2026](https://apet.org.br/?p=144092) that trading stablecoins abroad does not by itself attract it.
* Brazil, from the press or summaries only: capital requirements by licence ([Capital Aberto](https://capitalaberto.com.br/regulacao/2026/10/prazo-do-bc-aperta-cerco-sobre-ativos-virtuais)); the central bank's view of stablecoins ([ConJur](https://www.conjur.com.br/2026-jul-01/o-cerco-do-banco-central-as-stablecoins/)); the 80-day window for contesting a Pix; the conviction of a peer-to-peer seller in São Paulo (the court's page timed out); the state of the central bank's ledger pilot.
* EU: MiCA article 48 on [ESMA's rulebook](https://www.esma.europa.eu/publications-and-data/interactive-single-rulebook/mica/article-48-requirements-offer-public-or); ESMA's [statement on the end of the transition](https://www.esma.europa.eu/sites/default/files/2026-06/ESMA75-113276571-1710_Public_Statement_MiCA_transitional_period_ends.pdf) (June 2026); a law firm's note on the [consultation about the DeFi exclusion](https://www.cerhahempel.com/blog/fintech-ledger/testing-the-boundaries-of-micas-defi-exemption).
* US: [FinCEN's withdrawal of its unhosted-wallet proposals](https://www.fincen.gov/news/news-releases/fincen-announces-withdrawals-proposed-digital-asset-related-rules) (October 2026); law-firm notes on the [stablecoin act's implementation](https://www.mofo.com/resources/insights/261028-occ-finalizes-rule-implementing) and on the [market-structure bill stalling](https://www.orrick.com/Insights/2026/10/The-CLARITY-Act-Stalls-in-the-Senate-Whats-Next-for-Digital-Asset-Regulation); [Ledger Insights on the SEC staff statement](https://www.ledgerinsights.com/sec-wallets-defi-interfaces-arent-broker-dealers-even-for-tokenized-securities/) (April 2026); the [DeFi Education Fund on the Tornado Cash case](https://www.defieducationfund.org/u-s-v-storm-2026-update/).
* FATF, [targeted update on virtual assets](https://www.fatf-gafi.org/en/publications/Fatfrecommendations/targeted-updated-virtualassets-vasps-2026.html), July 2026 (read from a mirror).
* Not checked against primary text: the criminal-law articles on unauthorised financial institutions, the FATF 2021 guidance wording, the dates of the EU transfer-of-funds and anti-money-laundering regulations, and anything after October 2026 on the US bill or the Tornado Cash case.
