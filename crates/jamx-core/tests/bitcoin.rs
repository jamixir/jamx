//! Buying bitcoin without anyone holding both legs: the header chain, receipts, and the
//! conditional payment released on proof.

mod common;
use common::*;
use jamx_core::{btc, ops::*, types::*};

const OK: u8 = Status::Ok as u8;
const SATS: u64 = 100_000; // 0.001 BTC
const PRICE: (u64, u64) = (6, 10); // 0.6 centavos per satoshi = R$ 600,000 per bitcoin
/// The seller has an hour, by Bitcoin's clock, to get the payment mined.
const PAY_SECS: u32 = 3_600;
const GRACE_BLOCKS: u16 = 2;
/// Height of the test chain when a test begins (see `setup`).
const START: u32 = 10;

fn st(s: Status) -> u8 {
	s as u8
}

struct Setup {
	w: World,
	chain: Regtest,
	btc: AssetId,
	brl: AssetId,
	bank: Actor,
	/// Has reais on JAM, wants bitcoin.
	bob: Actor,
	/// Has bitcoin, wants reais.
	alice: Actor,
	builder: Actor,
}

fn setup(confirmations: u8) -> Setup {
	let mut w = World::new();
	let mut chain = Regtest::new();
	let (bank, bob, alice, builder) = (actor(1), actor(2), actor(3), actor(9));
	let brl = w.token(&bank, "BRL", 0);
	let btc = w.next_asset();
	// Anyone can define the external asset; its parameters and starting header are public.
	w.must(
		&builder,
		Op::AssetCreate {
			symbol: sym("BTC"),
			decimals: 8,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::BtcSpv { confirmations, pay_secs: PAY_SECS, grace_blocks: GRACE_BLOCKS },
					bond_bps: 1_000, // the seller posts 10%
					claim_window: 4_000,
					min_deal: 1_000,
				},
				btc: Some((btc::REGTEST, chain.checkpoint())),
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	// The chain moves on ten blocks and somebody carries the headers in. Its clock now
	// reads the same as JAM's, and the service knows enough of it to open deals.
	chain.mine_empty(START as usize);
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset: btc, headers: chain.headers(1) }]), vec![OK]);
	assert_eq!(chain.blocks.last().unwrap().header.time, wall_clock(w.slot));
	w.mint(&bank, brl, &bob, 1_000_000);
	w.mint(&bank, brl, &alice, 50_000); // for her bond
	Setup { w, chain, btc, brl, bank, bob, alice, builder }
}

impl Setup {
	/// Bob and Alice meet in the order book; returns the deal that results.
	fn trade(&mut self, flags: u8) -> DealId {
		let buy = self.w.order_with(&self.bob, self.btc, self.brl, Side::Buy, SATS, PRICE, flags, script(0xb0));
		let sell = self.w.order(&self.alice, self.btc, self.brl, Side::Sell, SATS, PRICE);
		let builder = self.builder;
		assert_eq!(self.w.batch(self.btc, self.brl, &builder, 0, vec![buy, sell]), OK);
		self.w.deals().last().unwrap().0
	}

	/// Carry every header from `from` on into JAM.
	fn relay(&mut self, from: usize) -> u8 {
		let headers = self.chain.headers(from);
		self.w.submit(vec![Item::BtcHeaders { asset: self.btc, headers }])[0]
	}

	/// Alice pays from her own wallet: an ordinary transaction to Bob's address.
	fn pay(&mut self, deal: DealId, sats: u64, to: Vec<u8>, tagged: bool) -> usize {
		let mut outs = vec![(sats, to)];
		if tagged {
			outs.push((0, btc::tag_script(&deal_tag(SVC, deal))));
		}
		let tx = btc::build_tx(7_000 + deal as u32, &outs);
		self.chain.mine(vec![btc::build_tx(1, &[(1, script(1))]), tx, btc::build_tx(2, &[(2, script(2))])])
	}

	fn claim(&mut self, deal: DealId, block: usize) -> u8 {
		// The payment is the third transaction in the block (after the coinbase and one other).
		let proof = self.chain.blocks[block].proof(deal, 2, 0);
		self.w.submit(vec![Item::BtcProof(proof)])[0]
	}
}

#[test]
fn the_light_client_follows_the_chain() {
	let mut s = setup(6);
	assert_eq!(s.w.chain(s.btc).tip_height, START);
	s.chain.mine_empty(5);
	assert_eq!(s.relay(START as usize + 1), OK);
	let lc = s.w.chain(s.btc);
	assert_eq!((lc.tip_height, lc.tip), (START + 5, s.chain.tip()));
	// Regtest work is 2 per block.
	assert_eq!(lc.tip_work, 2 * (START as u128 + 5));
	// Carrying the same headers again changes nothing and is not an error.
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.w.chain(s.btc).tip_height, START + 5);
	// Headers that do not attach to anything known are refused.
	let mut other = Regtest::new();
	let a = other.mine_on([7; 32], vec![]);
	other.mine_empty(1);
	let stray = other.headers(a);
	assert_eq!(s.w.submit(vec![Item::BtcHeaders { asset: s.btc, headers: stray }]), vec![st(Status::NotFound)]);
	// Headers for an asset that is not a header chain, likewise.
	let headers = s.chain.headers(1);
	assert_eq!(s.w.submit(vec![Item::BtcHeaders { asset: s.brl, headers }]), vec![st(Status::NotFound)]);
}

#[test]
fn headers_with_the_wrong_difficulty_are_refused() {
	// A light client that expects real difficulty will not accept easy blocks, however
	// well they link: this is what stops anyone "mining" a fake payment on a laptop.
	let mut w = World::new();
	let chain = Regtest::new();
	let any = actor(1);
	let btc_asset = w.next_asset();
	let genesis = &chain.blocks[0];
	w.must(
		&any,
		Op::AssetCreate {
			symbol: sym("BTC"),
			decimals: 8,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::BtcSpv { confirmations: 6, pay_secs: 7_200, grace_blocks: 12 },
					bond_bps: 0,
					claim_window: 10_000,
					min_deal: 1,
				},
				btc: Some((
					btc::MAINNET,
					BtcCheckpoint {
						height: 840_000,
						hash: genesis.hash(),
						time: genesis.header.time,
						bits: 0x1703_4219, // real mainnet difficulty
						period_start: genesis.header.time,
						chainwork: 1 << 90,
					},
				)),
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	// An easy block on top of the checkpoint: refine accepts its (trivial) proof-of-work
	// relative to its own difficulty field, and accumulate rejects the field itself.
	let mut fake = Regtest::new();
	fake.mine_on(genesis.hash(), vec![]);
	let header = fake.blocks.last().unwrap().header.serialize();
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset: btc_asset, headers: vec![header] }]), vec![st(Status::Mismatch)]);
	assert_eq!(w.chain(btc_asset).tip_height, 840_000);
	// Lying about the difficulty field instead gets nowhere either: the hash does not meet
	// it, so refine drops the header and no record reaches accumulate at all.
	let mut liar = fake.blocks.last().unwrap().header.clone();
	liar.bits = 0x1703_4219;
	assert!(w.submit(vec![Item::BtcHeaders { asset: btc_asset, headers: vec![liar.serialize()] }]).is_empty());
}

#[test]
fn buying_bitcoin_with_reais_end_to_end() {
	let mut s = setup(6);
	let deal_id = s.trade(0);

	// Bob's reais are locked in the deal, together with Alice's bond. Nothing else moved.
	let d = s.w.deal(deal_id).unwrap();
	assert_eq!((d.payer, d.payee), (s.bob.id, s.alice.id));
	assert_eq!((d.ext_amount, d.amount, d.bond), (SATS, 60_000, 6_000));
	assert_eq!(d.delivery, script(0xb0));
	assert_eq!(d.flags, DEAL_REQUIRE_TAG);
	assert_eq!(s.w.bal(&s.bob, s.brl), 940_000);
	assert_eq!(s.w.bal(&s.alice, s.brl), 44_000);

	// Alice pays on Bitcoin. Until the payment is buried, nothing is released.
	let block = s.pay(deal_id, SATS, script(0xb0), true);
	assert_eq!(s.claim(deal_id, block), st(Status::NotFound)); // JAM has not seen the block yet
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(deal_id, block), st(Status::Unconfirmed)); // one confirmation
	s.chain.mine_empty(4);
	assert_eq!(s.relay(block + 1), OK);
	assert_eq!(s.claim(deal_id, block), st(Status::Unconfirmed)); // five
	s.chain.mine_empty(1);
	assert_eq!(s.relay(block + 5), OK);

	// Six blocks deep: anyone can carry the receipt in, and the reais go to Alice.
	assert_eq!(s.claim(deal_id, block), OK);
	assert!(s.w.deal(deal_id).is_none());
	assert_eq!(s.w.bal(&s.alice, s.brl), 50_000 + 60_000);
	assert_eq!(s.w.bal(&s.bob, s.brl), 940_000);

	// The same receipt cannot be presented twice.
	assert_eq!(s.claim(deal_id, block), st(Status::NotFound));
}

#[test]
fn one_payment_cannot_settle_two_deals() {
	let mut s = setup(1);
	let (bank, builder, carol) = (s.bank, s.builder, actor(4));
	s.w.mint(&bank, s.brl, &carol, 50_000);
	// Bob did not ask for tags (Alice pays from somewhere that cannot add one), so only
	// the address ties a payment to the deal.
	let d1 = s.trade(ORDER_NO_TAG);
	assert_eq!(s.w.deal(d1).unwrap().flags, DEAL_UNTAGGED);

	// While that deal is open, no second untagged deal can be opened to the same address:
	// either seller's payment could otherwise be claimed for the other's deal.
	let buy = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, ORDER_NO_TAG, script(0xb0));
	let sell = s.w.order(&carol, s.btc, s.brl, Side::Sell, SATS, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy.clone(), sell.clone()]), st(Status::NoMatch));
	// To another address it can, and so can a deal that demands its own tag.
	let other = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, ORDER_NO_TAG, script(0xb1));
	let sell2 = s.w.order(&carol, s.btc, s.brl, Side::Sell, SATS, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![other, sell2]), OK);

	let block = s.pay(d1, SATS, script(0xb0), false);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d1, block), OK);

	// With the first deal settled the address is free again. Alice paid once: her old
	// payment settles nothing more.
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, sell]), OK);
	let d3 = s.w.deals().iter().map(|(id, _)| *id).max().unwrap();
	assert_eq!(s.w.deal(d3).unwrap().delivery, script(0xb0));
	assert_ne!(s.claim(d3, block), OK);
	assert!(s.w.deal(d3).is_some());
}

#[test]
fn an_order_that_is_filled_by_two_sellers_without_tags_opens_one_deal_at_a_time() {
	let mut s = setup(1);
	let (bank, builder, carol) = (s.bank, s.builder, actor(4));
	s.w.mint(&bank, s.brl, &carol, 50_000);
	let buy = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, ORDER_NO_TAG, script(0xb0));
	let s1 = s.w.order(&s.alice, s.btc, s.brl, Side::Sell, 60_000, PRICE);
	let s2 = s.w.order(&carol, s.btc, s.brl, Side::Sell, 40_000, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy.clone(), s1, s2.clone()]), OK);
	// One deal, not two: the other seller simply was not matched this time.
	let deals = s.w.deals();
	assert_eq!(deals.len(), 1);
	assert_eq!(s.w.bal(&s.bob, s.brl), 1_000_000 - deals[0].1.amount);
	assert_eq!(s.w.bal(&carol, s.brl) + s.w.bal(&s.alice, s.brl), 100_000 - deals[0].1.bond);
	// Bob's order rests with the remainder and trades it once the first deal is done.
	let bob = s.bob;
	assert_eq!(s.w.op(&bob, Op::DealRelease { deal: deals[0].0 }), OK);
	let other_seller = if deals[0].1.payee == s.alice.id { carol } else { s.alice };
	let rest = SATS - deals[0].1.ext_amount;
	let sell = s.w.order(&other_seller, s.btc, s.brl, Side::Sell, rest, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, sell]), OK);
	assert_eq!(s.w.deals().len(), 1);
	assert_eq!(s.w.deals()[0].1.ext_amount, rest);
}

#[test]
fn a_payment_made_before_the_deal_does_not_count() {
	let mut s = setup(1);
	// Somebody paid Bob's address last week, for something else.
	let old = s.pay(999, SATS, script(0xb0), false);
	assert_eq!(s.relay(1), OK);
	let d = s.trade(ORDER_NO_TAG);
	// Alice tries to pass that old payment off as hers.
	assert_eq!(s.claim(d, old), st(Status::TooEarly));
}

#[test]
fn a_receipt_must_match_the_deal() {
	let mut s = setup(1);
	let d = s.trade(0);
	// Wrong address.
	let b1 = s.pay(d, SATS, script(0xee), true);
	// Too little.
	let b2 = s.pay(d, SATS - 1, script(0xb0), true);
	// Right address and amount, but not tagged for this deal.
	let b3 = s.pay(d, SATS, script(0xb0), false);
	// Tagged for a different deal.
	let other_tag = btc::tag_script(&deal_tag(SVC, d + 1));
	let tx = btc::build_tx(5, &[(SATS, script(0xb0)), (0, other_tag)]);
	let b4 = s.chain.mine(vec![btc::build_tx(1, &[(1, script(1))]), tx]);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, b1), st(Status::Mismatch));
	assert_eq!(s.claim(d, b2), st(Status::Insufficient));
	assert_eq!(s.claim(d, b3), st(Status::Mismatch));
	assert_eq!(s.claim(d, b4), st(Status::Mismatch));
	// A receipt whose Merkle path does not lead where the coinbase's does is not a receipt.
	let mut proof = s.chain.blocks[b1].proof(d, 2, 0);
	proof.branch[0][0] ^= 1;
	assert!(s.w.submit(vec![Item::BtcProof(proof)]).is_empty());
	// One whose paths agree with each other but not with the block JAM knows.
	let mut proof = s.chain.blocks[b1].proof(d, 2, 0);
	proof.block_hash = s.chain.blocks[b2].hash();
	assert_eq!(s.w.submit(vec![Item::BtcProof(proof)]), vec![st(Status::Mismatch)]);
	// A receipt naming a block JAM has never heard of.
	let mut proof = s.chain.blocks[b1].proof(d, 2, 0);
	proof.block_hash = [9; 32];
	assert_eq!(s.w.submit(vec![Item::BtcProof(proof)]), vec![st(Status::NotFound)]);
	assert!(s.w.deal(d).is_some());

	// An honest payment still settles it. Paying more than agreed is fine too.
	let good = s.pay(d, SATS + 5, script(0xb0), true);
	assert_eq!(s.relay(good), OK);
	assert_eq!(s.claim(d, good), OK);
}

/// With the test chain's ten-minute blocks, a block's median time past is the timestamp of
/// the block five below it. A deal opened at height 10 with an hour to pay therefore accepts
/// payments mined up to height 21, and block 22 is the first one past the window.
const LAST_IN_TIME: u32 = START + 11;

#[test]
fn if_alice_walks_bob_gets_his_reais_back_and_her_bond() {
	let mut s = setup(6);
	let d = s.trade(0);
	let deal = s.w.deal(d).unwrap();
	// The deal's clock is Bitcoin's: Alice has an hour from now.
	assert_eq!((deal.min_height, deal.pay_until), (START, wall_clock(s.w.slot) + PAY_SECS));
	// Nobody can cut the wait short, and JAM's own clock does not enter into it.
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::TooEarly));
	s.w.slot += 1_000;
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::TooEarly));
	// The chain moves on without a payment. The deal can fail once the first block past
	// the window lies under six confirmations and the grace period; one block short of
	// that is still too early.
	let closes = LAST_IN_TIME + 1 + 6 + GRACE_BLOCKS as u32;
	s.chain.mine_empty((closes - 1 - START) as usize);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.w.chain(s.btc).tip_height, closes - 1);
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::TooEarly));
	s.chain.mine_empty(1);
	assert_eq!(s.relay(closes as usize), OK);
	// Now anyone can close it.
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), OK);
	assert!(s.w.deal(d).is_none());
	// Bob is whole and keeps the bond: holding a free option on the price was not free.
	assert_eq!(s.w.bal(&s.bob, s.brl), 1_000_000 + 6_000);
	assert_eq!(s.w.bal(&s.alice, s.brl), 44_000);
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::NotFound));
}

#[test]
fn a_payment_is_in_time_by_bitcoins_own_clock() {
	// Mined in the last block inside the window: good.
	let mut s = setup(1);
	let d = s.trade(0);
	s.chain.mine_empty((LAST_IN_TIME - START - 1) as usize);
	let block = s.pay(d, SATS, script(0xb0), true);
	assert_eq!(block as u32, LAST_IN_TIME);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, block), OK);

	// One block later: too late, however promptly the receipt is presented.
	let mut s = setup(1);
	let d = s.trade(0);
	s.chain.mine_empty((LAST_IN_TIME - START) as usize);
	let block = s.pay(d, SATS, script(0xb0), true);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, block), st(Status::Expired));
	assert!(s.w.deal(d).is_some());
}

#[test]
fn a_seller_who_paid_on_time_cannot_lose_to_the_clock() {
	// The failure this design exists to prevent: Alice pays within minutes, but nobody
	// carries Bitcoin's headers into JAM for a long time. With a deadline in JAM slots the
	// deal would time out and Bob would keep the bitcoin, the reais and her bond.
	let mut s = setup(3);
	let d = s.trade(0);
	let block = s.pay(d, SATS, script(0xb0), true);
	s.chain.mine_empty(30);
	// Hours pass on JAM. As far as the service knows, Bitcoin has not moved, and so
	// neither has the deal's clock: Bob cannot time it out.
	s.w.slot += 2_500;
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::TooEarly));
	// To time it out Bob would have to carry the headers in himself. The very headers
	// that move the clock are the ones that prove Alice's payment, and her receipt is
	// good for as long as the deal is open.
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, block), OK);
	assert_eq!(s.w.bal(&s.alice, s.brl), 50_000 + 60_000);
}

#[test]
fn a_stale_view_of_bitcoin_does_not_shorten_the_sellers_window() {
	// Nobody has carried a header for two and a half hours: the service still thinks
	// Bitcoin is at block 10, and it is really at block 25.
	let mut s = setup(1);
	s.chain.mine_empty(15);
	s.w.slot += 15 * 100;
	assert_eq!(s.chain.blocks.last().unwrap().header.time, wall_clock(s.w.slot));
	// A deal opened now gives Alice her full hour from now, not from block 10. (Were the
	// window counted in blocks from the last one the service knows, it would be over
	// before it began, and whoever built the batch could arrange exactly that.)
	let d = s.trade(0);
	assert_eq!(s.w.deal(d).unwrap().pay_until, wall_clock(s.w.slot) + PAY_SECS);
	s.chain.mine_empty(4);
	let block = s.pay(d, SATS, script(0xb0), true); // forty minutes later
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, block), OK);
}

#[test]
fn if_the_header_chain_stalls_the_deal_is_undone_and_nobody_is_blamed() {
	let mut s = setup(6);
	let d = s.trade(0);
	let deadline = s.w.deal(d).unwrap().deadline;
	// No header ever arrives. After a long fallback period the deal can be cancelled:
	// Bob gets his reais back, Alice her bond. There is no evidence against either.
	s.w.slot = deadline;
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), st(Status::TooEarly));
	s.w.slot = deadline + 1;
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), OK);
	assert_eq!(s.w.bal(&s.bob, s.brl), 1_000_000);
	assert_eq!(s.w.bal(&s.alice, s.brl), 50_000);
}

#[test]
fn no_deal_opens_until_the_service_knows_enough_of_the_chain() {
	// Median time past is taken over eleven blocks. A light client that has just been
	// started from a checkpoint does not have them yet.
	let mut w = World::new();
	let mut chain = Regtest::new();
	let (bank, bob, alice, builder) = (actor(1), actor(2), actor(3), actor(9));
	let brl = w.token(&bank, "BRL", 0);
	let asset = w.next_asset();
	w.must(
		&builder,
		Op::AssetCreate {
			symbol: sym("BTC"),
			decimals: 8,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::BtcSpv { confirmations: 1, pay_secs: PAY_SECS, grace_blocks: 2 },
					bond_bps: 1_000,
					claim_window: 4_000,
					min_deal: 1_000,
				},
				btc: Some((btc::REGTEST, chain.checkpoint())),
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	w.mint(&bank, brl, &bob, 1_000_000);
	w.mint(&bank, brl, &alice, 50_000);
	let round = |w: &mut World| {
		let buy = w.order_with(&bob, asset, brl, Side::Buy, SATS, PRICE, 0, script(0xb0));
		let sell = w.order(&alice, asset, brl, Side::Sell, SATS, PRICE);
		w.batch(asset, brl, &builder, 0, vec![buy, sell])
	};
	assert_eq!(round(&mut w), st(Status::Stale));
	chain.mine_empty(9);
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset, headers: chain.headers(1) }]), vec![OK]);
	assert_eq!(round(&mut w), st(Status::Stale));
	chain.mine_empty(1);
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset, headers: chain.headers(10) }]), vec![OK]);
	assert_eq!(round(&mut w), OK);
	assert_eq!(w.deals().len(), 1);
}

#[test]
fn bob_can_release_early_and_only_bob() {
	let mut s = setup(6);
	let d = s.trade(0);
	let (alice, bob) = (s.alice, s.bob);
	// Alice cannot release Bob's money to herself.
	assert_eq!(s.w.op(&alice, Op::DealRelease { deal: d }), st(Status::Unauthorized));
	// Bob sees the bitcoin in his wallet and does not make her wait for six blocks.
	assert_eq!(s.w.op(&bob, Op::DealRelease { deal: d }), OK);
	assert_eq!(s.w.bal(&alice, s.brl), 50_000 + 60_000);
}

#[test]
fn a_reorganisation_takes_the_confirmations_with_it() {
	let mut s = setup(3);
	let d = s.trade(0);
	s.chain.mine_empty(1); // block 11, common ancestor
	let fork_point = s.chain.tip();
	let block = s.pay(d, SATS, script(0xb0), true); // block 12 carries the payment
	s.chain.mine_empty(1); // block 13
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.w.chain(s.btc).tip_height, START + 3);
	assert_eq!(s.claim(d, block), st(Status::Unconfirmed)); // two confirmations of three

	// A competing branch from block 11, without the payment, grows longer.
	let mut fork = Regtest::new();
	fork.clock = s.chain.blocks[block].header.time;
	let a = fork.mine_on(fork_point, vec![]);
	fork.mine_empty(2);
	let fork_headers = fork.headers(a);
	assert_eq!(s.w.submit(vec![Item::BtcHeaders { asset: s.btc, headers: fork_headers }]), vec![OK]);
	let lc = s.w.chain(s.btc);
	assert_eq!((lc.tip_height, lc.tip), (START + 4, fork.tip()));

	// The payment's block is no longer on the best chain: no depth counts for anything.
	s.chain.mine_empty(1); // block 14 on the old branch: equal work, not more
	let old_tip_header = s.chain.blocks.last().unwrap().header.serialize();
	assert_eq!(s.w.submit(vec![Item::BtcHeaders { asset: s.btc, headers: vec![old_tip_header] }]), vec![OK]);
	assert_eq!(s.w.chain(s.btc).tip, fork.tip()); // first seen wins a tie
	assert_eq!(s.claim(d, block), st(Status::Unconfirmed));

	// If the original branch overtakes again, the confirmations are back.
	s.chain.mine_empty(1); // block 15
	let h = s.chain.blocks.last().unwrap().header.serialize();
	assert_eq!(s.w.submit(vec![Item::BtcHeaders { asset: s.btc, headers: vec![h] }]), vec![OK]);
	assert_eq!(s.w.chain(s.btc).tip, s.chain.tip());
	assert_eq!(s.claim(d, block), OK);
}

#[test]
fn partial_fills_open_one_deal_per_pair() {
	let mut s = setup(1);
	let carol = actor(4);
	let (bank, builder) = (s.bank, s.builder);
	s.w.mint(&bank, s.brl, &carol, 50_000);
	// Bob wants 100k sats; Alice sells 60k and Carol 40k.
	let buy = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, 0, script(0xb0));
	let s1 = s.w.order(&s.alice, s.btc, s.brl, Side::Sell, 60_000, PRICE);
	let s2 = s.w.order(&carol, s.btc, s.brl, Side::Sell, 40_000, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, s1, s2]), OK);
	let deals = s.w.deals();
	assert_eq!(deals.len(), 2);
	let mut sizes: Vec<u64> = deals.iter().map(|(_, d)| d.ext_amount).collect();
	sizes.sort();
	assert_eq!(sizes, vec![40_000, 60_000]);
	assert!(deals.iter().all(|(_, d)| d.payer == s.bob.id && d.delivery == script(0xb0)));
	assert_eq!(s.w.bal(&s.bob, s.brl), 1_000_000 - 60_000);
	// Each seller proves her own payment; the two deals are independent.
	let (id_a, _) = deals.iter().find(|(_, d)| d.payee == s.alice.id).unwrap().clone();
	let block = s.pay(id_a, 60_000, script(0xb0), true);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(id_a, block), OK);
	assert_eq!(s.w.deals().len(), 1);
}

#[test]
fn a_seller_without_a_bond_cannot_take_the_other_side() {
	let mut s = setup(1);
	let broke = actor(77);
	let builder = s.builder;
	let buy = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, 0, script(0xb0));
	let sell = s.w.order(&broke, s.btc, s.brl, Side::Sell, SATS, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, sell]), st(Status::NoMatch));
	assert!(s.w.deals().is_empty());
	assert_eq!(s.w.bal(&s.bob, s.brl), 1_000_000);
	// A buyer who gives no delivery address is not matched either.
	let buy = s.w.order(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE);
	let sell = s.w.order(&s.alice, s.btc, s.brl, Side::Sell, SATS, PRICE);
	assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, sell]), st(Status::NoMatch));
	// Nor one whose "address" is something no wallet can pay: the seller would be matched
	// with a deal she cannot complete and lose her bond for it.
	for unpayable in [vec![0x6a], vec![0x51], vec![0xff; 34], script(0xb0)[..21].to_vec()] {
		let buy = s.w.order_with(&s.bob, s.btc, s.brl, Side::Buy, SATS, PRICE, 0, unpayable);
		let sell = s.w.order(&s.alice, s.btc, s.brl, Side::Sell, SATS, PRICE);
		assert_eq!(s.w.batch(s.btc, s.brl, &builder, 0, vec![buy, sell]), st(Status::NoMatch));
	}
	assert!(s.w.deals().is_empty());
}

#[test]
fn the_real_bitcoin_chain_is_followed_across_a_difficulty_change() {
	// The same code, fed real mainnet headers: blocks 840,668 to 840,679, which cross the
	// retarget at 840,672, and a real transaction from block 840,673.
	let fx: std::collections::BTreeMap<String, String> = include_str!("fixtures/btc_mainnet.txt")
		.lines()
		.filter_map(|l| l.split_once('='))
		.map(|(k, v)| (k.into(), v.into()))
		.collect();
	let h80 = |s: &str| -> [u8; 80] { hex::decode(s).unwrap().try_into().unwrap() };
	let rev = |s: &str| -> [u8; 32] {
		let mut b: [u8; 32] = hex::decode(s).unwrap().try_into().unwrap();
		b.reverse();
		b
	};
	let run: Vec<[u8; 80]> = fx["run_headers"].split(',').map(h80).collect();
	let first_height: u32 = fx["run_first_height"].parse().unwrap();
	let first = btc::Header::parse(&run[0]);
	let period_start = btc::Header::parse(&h80(&fx["period_start_header"])).time;

	let mut w = World::new();
	let (bank, bob, alice, builder) = (actor(1), actor(2), actor(3), actor(9));
	let brl = w.token(&bank, "BRL", 0);
	let btc_asset = w.next_asset();
	w.must(
		&builder,
		Op::AssetCreate {
			symbol: sym("BTC"),
			decimals: 8,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::BtcSpv { confirmations: 6, pay_secs: 7_200, grace_blocks: 12 },
					bond_bps: 500,
					claim_window: 10_000,
					min_deal: 546,
				},
				btc: Some((
					btc::MAINNET,
					BtcCheckpoint {
						height: first_height,
						hash: first.hash(),
						time: first.time,
						bits: first.bits,
						period_start,
						chainwork: 0,
					},
				)),
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	w.mint(&bank, brl, &bob, 10_000_000);
	w.mint(&bank, brl, &alice, 1_000_000);

	// The real transaction: find what it paid, and let that be the deal.
	let raw = hex::decode(&fx["proof_a_tx_hex"]).unwrap();
	let tx = btc::strip_witness(&raw).unwrap();
	let outs = btc::parse_outputs(&tx).unwrap();
	let (vout, out) = outs.iter().enumerate().find(|(_, o)| o.value > 1_000 && o.script.len() <= 40).unwrap();
	let (value, pay_script) = (out.value, out.script.to_vec());

	// Carry the first real headers in. Changing one bit of any of them breaks its
	// proof-of-work.
	let mut tampered = run[1];
	tampered[40] ^= 1;
	assert!(w.submit(vec![Item::BtcHeaders { asset: btc_asset, headers: vec![tampered] }]).is_empty());
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset: btc_asset, headers: run[1..12].to_vec() }]), vec![OK]);

	// With eleven blocks known past the starting point, the deal opens. (The blocks are
	// from April 2024, before JAM's clock starts, so by Bitcoin's clock the payment that
	// follows is comfortably in time.)
	let buy = w.order_with(&bob, btc_asset, brl, Side::Buy, value, PRICE, ORDER_NO_TAG, pay_script.clone());
	let sell = w.order(&alice, btc_asset, brl, Side::Sell, value, PRICE);
	assert_eq!(w.batch(btc_asset, brl, &builder, 0, vec![buy, sell]), OK);
	let (deal_id, deal) = w.deals().pop().unwrap();
	assert_eq!(deal.min_height, first_height + 11);
	let proof_height: u32 = fx["proof_block_height"].parse().unwrap();
	assert!(proof_height > deal.min_height);

	assert_eq!(w.submit(vec![Item::BtcHeaders { asset: btc_asset, headers: run[12..].to_vec() }]), vec![OK]);
	let lc = w.chain(btc_asset);
	assert_eq!(lc.tip_height, first_height + run.len() as u32 - 1);
	// The difficulty changed at the boundary and the client followed it.
	let retarget: u32 = fx["retarget_height"].parse().unwrap();
	let at = |h: u32| w.header(btc_asset, &btc::header_hash(&run[(h - first_height) as usize])).unwrap();
	assert_ne!(at(retarget).bits, at(retarget - 1).bits);
	assert_eq!(at(retarget).period_start, at(retarget).time);
	assert_eq!(at(retarget + 1).period_start, at(retarget).time);

	// The receipt for the real transaction releases the deal.
	let proof = BtcProof {
		deal: deal_id,
		block_hash: rev(&fx["proof_block_hash"]),
		tx,
		vout: vout as u32,
		index: fx["proof_a_pos"].parse().unwrap(),
		branch: fx["proof_a_merkle"].split(',').map(rev).collect(),
		// The block's real coinbase, which fixes the depth of its Merkle tree (12 levels
		// for its 2,788 transactions).
		coinbase: btc::strip_witness(&hex::decode(&fx["proof_cb_tx_hex"]).unwrap()).unwrap(),
		coinbase_branch: fx["proof_cb_merkle"].split(',').map(rev).collect(),
	};
	assert!(btc::is_coinbase(&proof.coinbase));
	assert_eq!(proof.coinbase_branch.len(), 12);
	// Block 840,673 has six blocks on top of it in this run: seven confirmations.
	assert_eq!(w.submit(vec![Item::BtcProof(proof)]), vec![OK]);
	assert!(w.deal(deal_id).is_none());
	assert_eq!(w.bal(&alice, brl), 1_000_000 + deal.amount);
}
