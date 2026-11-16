//! Attacks found by an adversarial review of the first version, kept as regression tests.
//!
//! Each test sets up the attack as the review described it and checks that it now fails.
//! The numbers (F1, F2, ...) are the review's own and are used in docs/SECURITY.md.

mod common;
use common::*;
use jamx_core::{
	accumulate::{Genesis, Session, GAS_COMMIT},
	btc,
	codec::Codec,
	crypto::{blake2b, sha256d, Hash32},
	ops::*,
	refine::refine,
	state::{key, mock::MockHost, Host},
	types::*,
};

const OK: u8 = Status::Ok as u8;

fn st(s: Status) -> u8 {
	s as u8
}

/// An order with an explicit nonce (the harness helper always takes the next one).
#[allow(clippy::too_many_arguments)]
fn order_n(
	w: &World,
	who: &Actor,
	base: AssetId,
	quote: AssetId,
	side: Side,
	qty: u64,
	limit: (u64, u64),
	nonce: u64,
) -> SignedOrder {
	SignedOrder::sign(
		&who.seed,
		SVC,
		base,
		quote,
		Order {
			account: who.id,
			nonce,
			side,
			qty,
			limit: Price::new(limit.0, limit.1),
			expiry: w.slot + 10_000,
			max_fee_bps: MAX_FEE_BPS,
			flags: 0,
			delivery: vec![],
		},
	)
}

// ---------------------------------------------------------------------------------------
// Bitcoin deals
// ---------------------------------------------------------------------------------------

const SATS: u64 = 100_000;
const PRICE: (u64, u64) = (6, 10);

struct Btc {
	w: World,
	chain: Regtest,
	btc: AssetId,
	brl: AssetId,
	bank: Actor,
	bob: Actor,
	alice: Actor,
	builder: Actor,
}

fn btc_setup(confirmations: u8, alice_brl: u64) -> Btc {
	let mut w = World::new();
	let mut chain = Regtest::new();
	let (bank, bob, alice, builder) = (actor(1), actor(2), actor(3), actor(9));
	let brl = w.token(&bank, "BRL", 0);
	let btc = w.next_asset();
	w.must(
		&builder,
		Op::AssetCreate {
			symbol: sym("BTC"),
			decimals: 8,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::BtcSpv { confirmations, pay_secs: 3_600, grace_blocks: 2 },
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
	// Deals open once the service has seen ten blocks past its starting point.
	chain.mine_empty(10);
	assert_eq!(w.submit(vec![Item::BtcHeaders { asset: btc, headers: chain.headers(1) }]), vec![OK]);
	w.mint(&bank, brl, &bob, 1_000_000);
	w.mint(&bank, brl, &alice, alice_brl);
	Btc { w, chain, btc, brl, bank, bob, alice, builder }
}

impl Btc {
	fn try_trade(&mut self, buyer: &Actor, seller: &Actor, sats: u64, flags: u8, delivery: Vec<u8>) -> u8 {
		let buy = self.w.order_with(buyer, self.btc, self.brl, Side::Buy, sats, PRICE, flags, delivery);
		let sell = self.w.order(seller, self.btc, self.brl, Side::Sell, sats, PRICE);
		let builder = self.builder;
		self.w.batch(self.btc, self.brl, &builder, 0, vec![buy, sell])
	}

	fn trade(&mut self, buyer: &Actor, seller: &Actor, sats: u64, flags: u8, delivery: Vec<u8>) -> DealId {
		assert_eq!(self.try_trade(buyer, seller, sats, flags, delivery), OK);
		self.w.deals().iter().map(|(id, _)| *id).max().unwrap()
	}

	fn relay(&mut self, from: usize) -> u8 {
		let headers = self.chain.headers(from);
		self.w.submit(vec![Item::BtcHeaders { asset: self.btc, headers }])[0]
	}

	/// The seller's payment: third transaction of a new block, tagged for `deal`.
	fn pay(&mut self, deal: DealId, sats: u64, to: Vec<u8>) -> usize {
		let tx = btc::build_tx(7_000 + deal as u32, &[(sats, to), (0, btc::tag_script(&deal_tag(SVC, deal)))]);
		self.chain.mine(vec![btc::build_tx(1, &[(1, script(1))]), tx, btc::build_tx(2, &[(2, script(2))])])
	}

	fn claim(&mut self, deal: DealId, block: usize) -> u8 {
		let proof = self.chain.blocks[block].proof(deal, 2, 0);
		self.w.submit(vec![Item::BtcProof(proof)])[0]
	}
}

/// F1 (critical). The buyer opens the real deal with Alice (tag required) and a tiny decoy
/// deal with an accomplice, same address, tag waived. When Alice's payment confirms, the
/// accomplice presents it for the decoy. Before the fix that settled the decoy and marked
/// the output as used: Alice's own claim was refused for ever, and at the deadline the buyer
/// was refunded with her bond, holding the bitcoin as well.
///
/// Now a payment that carries a deal's tag can settle that deal only.
#[test]
fn f1_a_decoy_deal_cannot_use_up_the_sellers_payment() {
	let mut s = btc_setup(1, 50_000);
	let (bank, bob, alice, carol) = (s.bank, s.bob, s.alice, actor(4));
	s.w.mint(&bank, s.brl, &carol, 1_000);

	let d1 = s.trade(&bob, &alice, SATS, 0, script(0xb0));
	assert_eq!(s.w.deal(d1).unwrap().flags, DEAL_REQUIRE_TAG);
	let d2 = s.trade(&bob, &carol, 1_000, ORDER_NO_TAG, script(0xb0));
	assert_eq!(s.w.deal(d2).unwrap().flags, DEAL_UNTAGGED);

	let block = s.pay(d1, SATS, script(0xb0));
	assert_eq!(s.relay(1), OK);

	assert_eq!(s.claim(d2, block), st(Status::Mismatch));
	assert_eq!(s.claim(d1, block), OK);
	assert_eq!(s.w.bal(&alice, s.brl), 50_000 + 60_000);
	assert!(s.w.deal(d2).is_some());
}

/// F1, the same idea without tags. If two deals open to untagged payments shared an
/// address, either seller's payment could be claimed for the other's deal. There is never
/// more than one.
#[test]
fn f1b_two_untagged_deals_never_share_an_address() {
	let mut s = btc_setup(1, 50_000);
	let (bank, bob, alice, carol, mallory) = (s.bank, s.bob, s.alice, actor(4), actor(66));
	s.w.mint(&bank, s.brl, &carol, 1_000);
	s.w.mint(&bank, s.brl, &mallory, 10_000);
	let d1 = s.trade(&bob, &alice, SATS, ORDER_NO_TAG, script(0xb0));
	// Neither Bob nor a third party who copies his address can open a second one.
	assert_eq!(s.try_trade(&bob, &carol, 1_000, ORDER_NO_TAG, script(0xb0)), st(Status::NoMatch));
	assert_eq!(s.try_trade(&mallory, &carol, 1_000, ORDER_NO_TAG, script(0xb0)), st(Status::NoMatch));
	assert_eq!(s.w.deals().len(), 1);
	// Alice pays without a tag and is paid.
	let tx = btc::build_tx(1, &[(SATS, script(0xb0))]);
	let block = s.chain.mine(vec![btc::build_tx(2, &[(1, script(1))]), tx]);
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d1, block), OK);
	assert_eq!(s.w.bal(&alice, s.brl), 50_000 + 60_000);
}

/// F2 (high). An issuer could write a flag into any account for its own asset, creating an
/// entry there, and an account held a bounded number of entries. A few hundred throw-away
/// tokens filled a victim's account so that it could not be paid in anything new; a buyer
/// used that to make the seller's valid proof fail and collected her bond.
///
/// Now only the account itself adds entries to its record.
#[test]
fn f2_nobody_can_fill_the_payees_account_to_block_her_payout() {
	// Alice holds exactly her bond, so her BRL balance is zero while the deal is open.
	let mut s = btc_setup(1, 6_000);
	let (bob, alice, mallory, bank) = (s.bob, s.alice, actor(66), s.bank);
	let d = s.trade(&bob, &alice, SATS, 0, script(0xb0));
	assert_eq!(s.w.bal(&alice, s.brl), 0);
	let before = s.w.account(&alice.id).balances.len();

	for i in 0..8 {
		let junk = s.w.token(&mallory, "JUNK", ASSET_FREEZABLE);
		let mark = Op::IssuerSet { asset: junk, target: alice.id, action: IssuerAction::Freeze, reason: [i; 32] };
		assert_eq!(s.w.op(&mallory, mark), st(Status::NotFound));
		assert_eq!(s.w.op(&mallory, Op::Mint { asset: junk, to: alice.id, amount: 1 }), st(Status::NotAllowed));
	}
	assert_eq!(s.w.account(&alice.id).balances.len(), before);

	let block = s.pay(d, SATS, script(0xb0));
	assert_eq!(s.relay(1), OK);
	assert_eq!(s.claim(d, block), OK);
	assert_eq!(s.w.bal(&alice, s.brl), 6_000 + 60_000);
	s.w.mint(&bank, s.brl, &alice, 1);
}

/// F2, second face. An account that fills itself with assets and then places an order it
/// could not take delivery of is left out of the round; it does not undo it.
#[test]
fn f2b_an_order_from_a_full_account_is_left_out_of_the_round() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	let (alice, bob, mallory, builder) = (actor(2), actor(3), actor(66), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	w.mint(&bank, quote, &mallory, 1_000);
	for _ in 0..MAX_BALANCES - 1 {
		let junk = w.token(&mallory, "JUNK", 0);
		w.mint(&mallory, junk, &mallory, 1);
	}
	assert_eq!(w.account(&mallory.id).balances.len(), MAX_BALANCES);
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 20, (100, 1)),
		w.order(&mallory, base, quote, Side::Buy, 1, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	assert_eq!(w.bal(&bob, base), 10);
	assert_eq!(w.bal(&mallory, base), 0);
}

// ---------------------------------------------------------------------------------------
// Account rules
// ---------------------------------------------------------------------------------------

/// F3 (high). Setting a recovery group where there was none was treated as harmless and
/// applied at once, even on an account with a security delay. A thief with a stolen owner
/// key named its own sock-puppet as the sole recovery guardian, and the recovery then
/// replaced every key and lifted the guardian's freeze.
///
/// Now naming a recovery group always waits out the delay, and a frozen account can still
/// drop what is queued.
#[test]
fn f3_a_stolen_key_cannot_install_its_own_recovery_group() {
	let mut w = World::new();
	let (bank, alice, friend, sock, thief) = (actor(1), actor(2), actor(4), actor(60), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.accept(&thief, brl);
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(500)));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(100) }));

	// The thief holds Alice's key (the test signs with it).
	let group = RecoveryCfg { guardians: vec![sock.id], threshold: 1, delay: 0 };
	w.must(&alice, Op::Policy(PolicyChange::SetRecovery(Some(group))));
	let p = w.account(&alice.id).policy.unwrap();
	assert!(p.recovery.is_none() && p.pending.len() == 1);

	// The friend notices and freezes. That stops everything.
	w.must(&friend, Op::Freeze { target: alice.id });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: thief.id, amount: 1 }), st(Status::Frozen));
	assert_eq!(w.op(&sock, Op::RecoveryApprove { target: alice.id, new_owner: thief.id }), st(Status::NotFound));
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), st(Status::NotFound));
	w.slot += 500;
	assert_eq!(w.plain(PlainOp::PolicyExec { account: alice.id, index: 0 }), st(Status::Frozen));
	// Alice, who still has her key, drops the queued change while the account is frozen.
	w.must(&alice, Op::PolicyCancel { index: 0 });
	let p = w.account(&alice.id).policy.unwrap();
	assert!(p.recovery.is_none() && p.pending.is_empty());
	assert_eq!(p.keys, vec![KeyEntry { key: alice.id, role: ROLE_OWNER }]);
	assert_eq!(w.bal(&thief, brl), 0);
}

/// F3, second face. With no guardian in place the thief appointed its own, queued the
/// removal of the daily limit, and kept the account frozen through the whole delay so
/// that the owner could not cancel; then lifted the freeze, applied the change and emptied
/// the account in one payload.
#[test]
fn f3b_a_thief_appointed_guardian_cannot_keep_the_owner_from_cancelling() {
	let mut w = World::new();
	let (bank, alice, sock, thief) = (actor(1), actor(2), actor(60), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.accept(&thief, brl);
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(500)));
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(100) }));

	// Thief, with the stolen key:
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(sock.id))));
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: None }));
	assert_eq!(w.account(&alice.id).policy.unwrap().pending.len(), 1);
	w.must(&sock, Op::Freeze { target: alice.id });

	// Alice sees the queued change and cancels it, frozen or not.
	w.must(&alice, Op::PolicyCancel { index: 0 });

	w.slot += 500;
	let unfreeze = w.sign(&sock, Op::Unfreeze { target: alice.id });
	let drain = w.sign(&alice, Op::Transfer { asset: brl, to: thief.id, amount: 1_000 });
	assert_eq!(
		w.submit(vec![unfreeze, Item::Plain(PlainOp::PolicyExec { account: alice.id, index: 0 }), drain]),
		vec![OK, st(Status::NotFound), st(Status::Limit)]
	);
	assert_eq!(w.bal(&thief, brl), 0);
}

/// F4 (medium). Raising the freeze length was applied at once, but the same figure is the
/// guardian's cool-down: set to the maximum after any earlier freeze, it kept the guardian
/// from ever freezing again.
#[test]
fn f4_the_freeze_length_cannot_be_used_to_switch_the_guardian_off() {
	let mut w = World::new();
	let (bank, alice, friend, thief) = (actor(1), actor(2), actor(4), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(500)));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));
	// Some time ago: a false alarm, frozen and lifted again.
	w.must(&friend, Op::Freeze { target: alice.id });
	w.must(&friend, Op::Unfreeze { target: alice.id });
	w.slot += 2 * DEFAULT_FREEZE_SLOTS;

	// The thief, with Alice's key.
	assert_eq!(w.op(&alice, Op::Policy(PolicyChange::SetFreezeSlots(u32::MAX))), st(Status::Invalid));
	w.must(&alice, Op::Policy(PolicyChange::SetFreezeSlots(MAX_FREEZE_SLOTS)));
	let p = w.account(&alice.id).policy.unwrap();
	assert_eq!((p.pending.len(), p.freeze_slots), (1, DEFAULT_FREEZE_SLOTS));

	// The guardian still has its say.
	w.must(&friend, Op::Freeze { target: alice.id });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: thief.id, amount: 1 }), st(Status::Frozen));
}

/// F4, second face. The same change followed by a self-freeze locked an account until the
/// end of time, with a key whose reach was meant to be bounded by a daily limit.
#[test]
fn f4b_no_key_can_freeze_an_account_for_ever() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.accept(&bob, brl);
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(10) }));
	assert_eq!(w.op(&alice, Op::Policy(PolicyChange::SetFreezeSlots(u32::MAX))), st(Status::Invalid));
	w.must(&alice, Op::Freeze { target: alice.id });
	let until = w.account(&alice.id).policy.unwrap().frozen_until;
	assert!(until <= w.slot + MAX_FREEZE_SLOTS);
	w.slot = until;
	w.must(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 1 });
}

/// F5 (medium). A trading key ("a phone, a bot") could freeze the account. A frozen owner
/// cannot veto a recovery, so a trading key plus the recovery quorum took the account.
#[test]
fn f5_a_trading_key_cannot_freeze_the_account() {
	let mut w = World::new();
	let (alice, phone, g1, g2, mallory) = (actor(2), actor(22), actor(31), actor(32), actor(66));
	w.must(&alice, Op::Policy(PolicyChange::AddKey { key: phone.id, role: ROLE_TRADER }));
	let group = RecoveryCfg { guardians: vec![g1.id, g2.id], threshold: 2, delay: 300 };
	w.must(&alice, Op::Policy(PolicyChange::SetRecovery(Some(group))));
	assert_eq!(w.op_as(&phone, &alice.id, Op::Freeze { target: alice.id }), st(Status::Unauthorized));
	w.must(&g1, Op::RecoveryApprove { target: alice.id, new_owner: mallory.id });
	w.must(&g2, Op::RecoveryApprove { target: alice.id, new_owner: mallory.id });
	// Alice has her key and objects, as the design promises she can.
	w.must(&alice, Op::RecoveryCancel);
	w.slot += 300;
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), st(Status::NotFound));
	assert_eq!(w.account(&alice.id).policy.unwrap().keys[0].key, alice.id);
}

/// F9 (medium). A failed operation kept its nonce, and a signed operation had no expiry, so
/// the bytes stayed usable. A guardian's redundant `Unfreeze`, refused at the time, was
/// replayed by a thief months later to lift a real freeze.
#[test]
fn f9_a_failed_operation_cannot_be_replayed_later() {
	let mut w = World::new();
	let (bank, alice, friend, thief) = (actor(1), actor(2), actor(4), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.accept(&thief, brl);
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));

	// A false alarm. The friend's wallet sends the unfreeze twice.
	w.must(&friend, Op::Freeze { target: alice.id });
	let first = w.sign(&friend, Op::Unfreeze { target: alice.id });
	let second = w.sign(&friend, Op::Unfreeze { target: alice.id });
	assert_eq!(w.submit(vec![first, second.clone()]), vec![OK, st(Status::Invalid)]);

	// Much later Alice's key really is stolen and the friend freezes in time.
	w.slot += 30 * DAY_SLOTS;
	w.must(&friend, Op::Freeze { target: alice.id });

	// The thief replays the stale bytes. Their nonce went when they failed.
	assert_eq!(w.submit(vec![second]), vec![st(Status::Nonce)]);
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: thief.id, amount: 1_000 }), st(Status::Frozen));
	assert_eq!(w.bal(&thief, brl), 0);
}

// ---------------------------------------------------------------------------------------
// Auction rounds
// ---------------------------------------------------------------------------------------

/// F6 (medium). Two orders of one account whose nonces lay a window apart: marking the
/// far one slid the window past the near one, whose marking then failed and took the whole
/// round with it. Both orders were validly signed, so refine kept them.
#[test]
fn f6_one_accounts_nonces_cannot_void_a_round() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	let (alice, bob, mallory, builder) = (actor(2), actor(3), actor(66), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	w.mint(&bank, base, &mallory, 1);
	w.mint(&bank, quote, &mallory, 100);
	let floor = w.account(&mallory.id).nonce_floor;
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 10, (100, 1)),
		order_n(&w, &mallory, base, quote, Side::Buy, 1, (100, 1), floor + 300),
		order_n(&w, &mallory, base, quote, Side::Sell, 1, (100, 1), floor + 5),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	// The honest trade happened. Of Mallory's two orders only the first seen took part.
	assert_eq!(w.bal(&bob, base) + w.bal(&mallory, base), 11);
	assert_eq!(w.bal(&alice, base), 90);
	assert!(w.bal(&bob, base) >= 9);
}

fn pix_asset(w: &mut World, creator: &Actor, oracle: &Actor, bond_bps: u16) -> AssetId {
	let id = w.next_asset();
	w.must(
		creator,
		Op::AssetCreate {
			symbol: sym("PIX"),
			decimals: 2,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::Oracle { oracles: vec![oracle.id], threshold: 1 },
					bond_bps,
					claim_window: 300,
					min_deal: 1,
				},
				btc: None,
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	id
}

/// F7 (medium). In an external market an order was sized by the cost of its whole fill, but
/// charged once per matched pair, each rounded up. A buyer cut down to exactly what it could
/// pay and matched with two sellers at a fractional price was an atom short on the second
/// pair, and the whole round was rolled back, honest trades included.
///
/// Now a pair that cannot be funded is left out by itself.
#[test]
fn f7_a_buyer_one_atom_short_loses_one_pair_not_the_round() {
	let mut w = World::new();
	let (bank, oracle, builder) = (actor(1), actor(41), actor(9));
	let usd = w.token(&bank, "USDX", 0);
	let pix = pix_asset(&mut w, &builder, &oracle, 0);
	let (tight, s1, s2, honest_buyer, s3) = (actor(10), actor(11), actor(12), actor(13), actor(14));
	w.mint(&bank, usd, &tight, 5); // exactly 10 units at 1/2
	w.mint(&bank, usd, &honest_buyer, 1_000);
	let orders = vec![
		w.order_with(&honest_buyer, pix, usd, Side::Buy, 100, (1, 2), 0, b"key-h".to_vec()),
		w.order(&s3, pix, usd, Side::Sell, 100, (1, 2)),
		// Better prices on both sides, so these three orders pair with one another.
		w.order_with(&tight, pix, usd, Side::Buy, 10, (3, 5), 0, b"key-t".to_vec()),
		w.order(&s1, pix, usd, Side::Sell, 5, (2, 5)),
		w.order(&s2, pix, usd, Side::Sell, 5, (2, 5)),
	];
	assert_eq!(w.batch(pix, usd, &builder, 0, orders), OK);
	let deals = w.deals();
	assert!(deals.iter().any(|(_, d)| d.payer == honest_buyer.id && d.ext_amount == 100));
	assert_eq!(deals.iter().filter(|(_, d)| d.payer == tight.id).count(), 1);
	// Nobody was charged more than they had, and nobody past their limit.
	let tight_deal = &deals.iter().find(|(_, d)| d.payer == tight.id).unwrap().1;
	assert_eq!(tight_deal.ext_amount, 5);
	assert!(5 - w.bal(&tight, usd) <= 3); // limit 3/5: at most 3 for 5 units
}

/// F8 (medium). A seller is matched blind with whatever destination the buyer wrote, and a
/// deal had two outcomes: delivered, or the buyer refunded with the seller's bond. A buyer
/// naming a destination that cannot be paid collected the bond of every seller it met.
///
/// Now an oracle can declare a deal void, which penalises nobody, and on a Bitcoin rail
/// only scripts every wallet can pay are accepted as destinations.
#[test]
fn f8_an_unpayable_destination_does_not_earn_the_sellers_bond() {
	let mut w = World::new();
	let (bank, oracle, builder) = (actor(1), actor(41), actor(9));
	let usd = w.token(&bank, "USDX", 0);
	let pix = pix_asset(&mut w, &builder, &oracle, 1_000);
	let (farmer, victim) = (actor(66), actor(3));
	w.mint(&bank, usd, &farmer, 10_000);
	w.mint(&bank, usd, &victim, 1_000);
	let buy = w.order_with(&farmer, pix, usd, Side::Buy, 50_000, (1, 500), 0, vec![0xff; MAX_SCRIPT_LEN]);
	let sell = w.order(&victim, pix, usd, Side::Sell, 50_000, (1, 500));
	assert_eq!(w.batch(pix, usd, &builder, 0, vec![buy, sell]), OK);
	let (d, deal) = w.deals().pop().unwrap();
	assert_eq!((deal.payee, deal.bond), (victim.id, 10));
	w.must(&oracle, Op::OracleAttest { deal: d, verdict: Verdict::Void });
	assert_eq!(w.bal(&farmer, usd), 10_000);
	assert_eq!(w.bal(&victim, usd), 1_000);

	let mut s = btc_setup(1, 50_000);
	let (bob, alice) = (s.bob, s.alice);
	assert_eq!(s.try_trade(&bob, &alice, SATS, 0, vec![0x6a]), st(Status::NoMatch));
	assert!(s.w.deals().is_empty());
}

/// F12 (low). Rounding always went against the trader and nothing bounded it relative to
/// the fill, so a bundler who was also the counterparty filled a resting order one atom at
/// a time: the buyer paid one quote atom per base atom although her limit was a thousandth
/// of that.
///
/// Now a fill worth less than a hundred quote atoms is rounded the trader's way, and the
/// builder makes up the difference.
#[test]
fn f12_dust_fills_cannot_trade_far_outside_the_limit() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "SHIB", 0);
	let quote = w.token(&bank, "GOLD", 0);
	let (victim, mallory) = (actor(2), actor(66));
	w.mint(&bank, quote, &victim, 1_000);
	w.mint(&bank, base, &mallory, 100);
	// "Buy 10,000 at no more than 1 quote per 1,000 base": at most 10 quote for the lot.
	let resting = w.order(&victim, base, quote, Side::Buy, 10_000, (1, 1_000));
	for _ in 0..10 {
		let sell = w.order_with(&mallory, base, quote, Side::Sell, 1, (1, 1_000), ORDER_IOC, vec![]);
		assert_eq!(w.batch(base, quote, &mallory, 0, vec![resting.clone(), sell]), OK);
	}
	// She received ten atoms and paid nothing for them: ten atoms are worth less than one
	// atom of the quote asset at her limit. The attack now costs the attacker.
	assert_eq!(w.bal(&victim, quote), 1_000);
	assert_eq!(w.bal(&victim, base), 10);
	assert_eq!(w.bal(&mallory, quote), 0);
	assert_eq!(w.bal(&mallory, base), 90);
}

/// F13 (low). An account remembers sixteen resting orders. A seventeenth that was partly
/// filled was not remembered, but its nonce was spent, so it was dead after its first
/// fill: a counterparty killed a market maker's orders for one atom each.
///
/// Now an order that could come to rest is not matched unless there is room to remember it.
#[test]
fn f13_a_resting_order_is_never_half_forgotten() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	let (maker, mallory, bob, builder) = (actor(2), actor(66), actor(3), actor(9));
	w.mint(&bank, base, &maker, 10_000);
	w.mint(&bank, quote, &mallory, 100_000);
	w.mint(&bank, quote, &bob, 100_000);
	let quotes: Vec<SignedOrder> =
		(0..17).map(|_| w.order(&maker, base, quote, Side::Sell, 100, (100, 1))).collect();
	for (i, q) in quotes.iter().enumerate() {
		let nibble = w.order_with(&mallory, base, quote, Side::Buy, 1, (100, 1), ORDER_IOC, vec![]);
		let expected = if i < MAX_OPEN_ORDERS { OK } else { st(Status::NoMatch) };
		assert_eq!(w.batch(base, quote, &builder, 0, vec![q.clone(), nibble]), expected);
	}
	assert_eq!(w.account(&maker.id).open.len(), MAX_OPEN_ORDERS);
	assert_eq!(w.bal(&maker, base), 10_000 - 16);
	// When one of the sixteen is done, the seventeenth can trade, whole.
	let buy = w.order(&bob, base, quote, Side::Buy, 99, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![quotes[0].clone(), buy]), OK);
	let buy = w.order(&bob, base, quote, Side::Buy, 100, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![quotes[16].clone(), buy]), OK);
	assert_eq!(w.bal(&bob, base), 199);
}

/// F14 (low). Cancelling a nonce beyond the window did nothing and reported success; the
/// order it was meant to cancel still traded.
#[test]
fn f14_a_cancel_that_cannot_reach_says_so() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	let far = w.account(&alice.id).nonce_floor + 300;
	let sell = order_n(&w, &alice, base, quote, Side::Sell, 10, (100, 1), far);
	assert_eq!(w.op(&alice, Op::Cancel { nonces: vec![far] }), st(Status::Nonce));
	// The way to cancel that far ahead works, and the order is dead.
	w.must(&alice, Op::BumpNonce { floor: far + 1 });
	let buy = w.order(&bob, base, quote, Side::Buy, 10, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy]), st(Status::NoMatch));
	assert_eq!(w.bal(&alice, base), 100);
}

// ---------------------------------------------------------------------------------------
// Receipts and headers
// ---------------------------------------------------------------------------------------

/// F10 (medium; on mainnet it needs about 2^70 hash operations and a miner). Refusing
/// 64-byte "transactions" closed one direction of the Merkle ambiguity. The other was open:
/// nothing fixed the depth of the tree, so a 64-byte transaction really in a block could be
/// read as an inner node whose right half was the identifier of a transaction that never
/// existed, "proved" one level below the block's real tree.
///
/// Now a receipt must show the block's coinbase at the same depth.
#[test]
fn f10_a_transaction_cannot_be_proved_below_the_real_tree() {
	let mut s = btc_setup(1, 50_000);
	let (bob, alice) = (s.bob, s.alice);
	let d = s.trade(&bob, &alice, SATS, 0, script(0xb0));

	// The payment Alice never broadcasts.
	let fake = btc::build_tx(9, &[(SATS, script(0xb0)), (0, btc::tag_script(&deal_tag(SVC, d)))]);
	let fake_txid = sha256d(&fake);
	// What she does get mined: 64 bytes ending in that identifier.
	let mut t64 = vec![0x11u8; 64];
	t64[32..].copy_from_slice(&fake_txid);
	let block = s.chain.mine(vec![t64.clone()]);
	assert_eq!(s.relay(1), OK);
	let b = &s.chain.blocks[block];
	assert_eq!(b.txs.len(), 2); // coinbase + the 64-byte one: depth 1

	let coinbase_txid = b.txids()[0];
	let mut left: Hash32 = [0; 32];
	left.copy_from_slice(&t64[..32]);
	let deep_branch = vec![left, coinbase_txid]; // depth 2
	// With the coinbase's true path (depth 1) the depths disagree...
	let honest_coinbase = BtcProof {
		deal: d,
		block_hash: b.hash(),
		tx: fake.clone(),
		vout: 0,
		index: 3,
		branch: deep_branch.clone(),
		coinbase: b.txs[0].clone(),
		coinbase_branch: vec![sha256d(&t64)],
	};
	// ...and a coinbase path stretched to depth 2 does not lead to the same root.
	let stretched = BtcProof { coinbase_branch: vec![sha256d(&t64), [0; 32]], ..honest_coinbase.clone() };
	// Nor can the 64-byte transaction itself be passed off as the coinbase.
	let impostor = BtcProof { coinbase: t64.clone(), coinbase_branch: deep_branch, ..honest_coinbase.clone() };
	for proof in [honest_coinbase, stretched, impostor] {
		assert!(s.w.submit(vec![Item::BtcProof(proof)]).is_empty());
	}
	assert!(s.w.deal(d).is_some());
	assert_eq!(s.w.bal(&alice, s.brl), 50_000 - 6_000);
}

/// F15 (low). The design said an issuer cannot attest for itself; nothing stopped it from
/// naming itself as an attestor.
#[test]
fn f15_an_issuer_cannot_be_its_own_attestor() {
	let mut w = World::new();
	let bank = actor(1);
	let create = Op::AssetCreate {
		symbol: sym("BRL"),
		decimals: 2,
		flags: 0,
		kind: AssetKindCfg::Token,
		attestors: vec![bank.id],
		threshold: 1,
	};
	assert_eq!(w.op(&bank, create), st(Status::Invalid));
}

/// F16 (low). A starting header whose total work is near the top of the range made a light
/// client that accepted every header and never advanced. Sellers on such an asset would
/// pay and never be able to prove it.
#[test]
fn f16_a_checkpoint_must_leave_room_for_the_chain_to_grow() {
	let mut w = World::new();
	let chain = Regtest::new();
	let create = |chainwork: u128| Op::AssetCreate {
		symbol: sym("BTC"),
		decimals: 8,
		flags: 0,
		kind: AssetKindCfg::External {
			cfg: ExternalCfg {
				rail: Rail::BtcSpv { confirmations: 2, pay_secs: 3_600, grace_blocks: 2 },
				bond_bps: 0,
				claim_window: 2_600,
				min_deal: 1,
			},
			btc: Some((btc::REGTEST, BtcCheckpoint { chainwork, ..chain.checkpoint() })),
		},
		attestors: vec![],
		threshold: 0,
	};
	assert_eq!(w.op(&actor(9), create(u128::MAX - 1)), st(Status::Invalid));
	assert_eq!(w.op(&actor(9), create(1 << 100)), OK);
	// The fallback deadline must leave the header chain time to run a deal's course.
	let hasty = Op::AssetCreate {
		symbol: sym("BTC"),
		decimals: 8,
		flags: 0,
		kind: AssetKindCfg::External {
			cfg: ExternalCfg {
				rail: Rail::BtcSpv { confirmations: 2, pay_secs: 3_600, grace_blocks: 2 },
				bond_bps: 0,
				claim_window: 2_599,
				min_deal: 1,
			},
			btc: Some((btc::REGTEST, chain.checkpoint())),
		},
		attestors: vec![],
		threshold: 0,
	};
	assert_eq!(w.op(&actor(9), hasty), st(Status::Invalid));
}

// ---------------------------------------------------------------------------------------
// Gas
// ---------------------------------------------------------------------------------------

/// A host that charges for storage roughly as measured in docs/GAS.md and traps when the
/// invocation's gas is gone, as the PVM does.
struct Metered {
	inner: MockHost,
	gas: u64,
}

const GAS_LOAD: u64 = 5_000;
const GAS_STORE: u64 = 4_600;

impl Metered {
	fn charge(&mut self, g: u64) {
		if self.gas < g {
			panic!("out of gas");
		}
		self.gas -= g;
	}
}

impl Host for Metered {
	fn read(&mut self, key: &[u8], buf: &mut Vec<u8>) -> bool {
		self.charge(GAS_LOAD);
		self.inner.read(key, buf)
	}
	fn write(&mut self, key: &[u8], value: &[u8]) -> bool {
		self.charge(GAS_STORE);
		self.inner.write(key, value)
	}
	fn remove(&mut self, key: &[u8]) {
		self.charge(GAS_STORE);
		self.inner.remove(key)
	}
	fn gas(&mut self) -> u64 {
		self.gas
	}
	fn checkpoint(&mut self) {
		self.inner.checkpoint()
	}
	fn upgrade(&mut self, code_hash: &Hash32, a: u64, b: u64) {
		self.inner.upgrade(code_hash, a, b)
	}
	fn solicit(&mut self, hash: &Hash32, len: u32) -> bool {
		self.inner.solicit(hash, len)
	}
	fn preimage_available(&mut self, hash: &Hash32) -> bool {
		self.inner.preimage_available(hash)
	}
}

fn committed_balance(host: &Metered, who: &Actor, asset: AssetId) -> u64 {
	host.inner.committed.get(&key::account(&who.id)).map_or(0, |b| Account::from_bytes(b).unwrap().free(asset))
}

/// F11 (medium; gas is simulated from the figures in docs/GAS.md). JAM runs one accumulate
/// invocation per service per block, with the gas of all its work items pooled. A batch
/// checked up front that it could afford its orders; a run of headers checked nothing. A
/// work item with the most headers the format allows, declared with little gas, spent
/// everybody else's and then trapped: the items behind it in the block were never applied.
///
/// Now each item may spend only the gas it declared, and loops stop before they run dry.
#[test]
fn f11_a_greedy_work_item_cannot_spend_its_neighbours_gas() {
	let mut s = btc_setup(6, 50_000);
	let (bob, alice) = (s.bob, s.alice);
	s.chain.mine_empty(MAX_HEADERS_PER_ITEM);
	let headers = s.chain.headers(11);
	assert_eq!(headers.len(), MAX_HEADERS_PER_ITEM);
	let attacker_payload = encode_payload(&[Item::BtcHeaders { asset: s.btc, headers }]);
	let pay = s.w.sign(&bob, Op::Transfer { asset: s.brl, to: alice.id, amount: 7 });
	let victim_payload = encode_payload(&[pay]);
	let (out_a, out_v) = (refine(SVC, &attacker_payload), refine(SVC, &victim_payload));
	assert!(!out_a.is_empty());

	// Same block, attacker's item first: 1,000,000 gas declared by the attacker, 200,000
	// by the victim, which is plenty for one transfer.
	let (gas_a, gas_v) = (1_000_000, 200_000);
	let mut host = Metered { inner: s.w.host.clone(), gas: gas_a + gas_v };
	let slot = s.w.slot;
	{
		let mut sess = Session::new(&mut host, SVC, slot, &Genesis::default());
		let before = sess.ctx.host.gas();
		// The attacker's item is served as far as its own gas goes...
		assert_eq!(sess.work(&blake2b(&[&attacker_payload]), &out_a, gas_a), vec![OK]);
		assert!(before - sess.ctx.host.gas() <= gas_a);
		// ...and the victim's is untouched by it.
		assert_eq!(sess.work(&blake2b(&[&victim_payload]), &out_v, gas_v), vec![OK]);
		sess.finish();
	}
	assert_eq!(committed_balance(&host, &alice, s.brl), 50_000 + 7);
	let lc = btc::LightClient::from_bytes(&host.inner.committed[&key::chain(s.btc)]).unwrap();
	assert!(lc.tip_height > 10 && (lc.tip_height as usize) < MAX_HEADERS_PER_ITEM, "tip {}", lc.tip_height);
}

/// The same for a round of orders too large for the gas its item brought: it fails as a
/// whole, cleanly, and the item behind it is applied.
#[test]
fn a_round_too_large_for_its_gas_fails_alone() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, quote, &bob, 1_000);
	let mut orders = vec![];
	for i in 0..40u8 {
		let (buyer, seller) = (actor(100 + i), actor(150 + i));
		w.mint(&bank, quote, &buyer, 10_000);
		w.mint(&bank, base, &seller, 100);
		orders.push(w.order(&buyer, base, quote, Side::Buy, 10, (100, 1)));
		orders.push(w.order(&seller, base, quote, Side::Sell, 10, (100, 1)));
	}
	let round = encode_payload(&[Item::Batch(Batch { base, quote, fee_account: builder.id, fee_bps: 0, orders })]);
	let pay = w.sign(&bob, Op::Transfer { asset: quote, to: alice.id, amount: 7 });
	w.accept(&alice, quote);
	let transfer = encode_payload(&[pay]);
	let (out_r, out_t) = (refine(SVC, &round), refine(SVC, &transfer));

	// Eighty orders need eighty accounts loaded: far more than 300,000 gas.
	let mut host = Metered { inner: w.host.clone(), gas: 300_000 + 200_000 };
	let slot = w.slot;
	{
		let mut sess = Session::new(&mut host, SVC, slot, &Genesis::default());
		assert_eq!(sess.work(&blake2b(&[&round]), &out_r, 300_000), vec![st(Status::OutOfGas)]);
		assert_eq!(sess.work(&blake2b(&[&transfer]), &out_t, 200_000), vec![OK]);
		sess.finish();
	}
	assert_eq!(committed_balance(&host, &alice, quote), 7);
	assert_eq!(committed_balance(&host, &actor(100), base), 0);

	// With the gas it needs, the same round settles.
	let mut host = Metered { inner: w.host.clone(), gas: 5_000_000 };
	{
		let mut sess = Session::new(&mut host, SVC, slot, &Genesis::default());
		assert_eq!(sess.work(&blake2b(&[&round]), &out_r, 5_000_000), vec![OK]);
		sess.finish();
	}
	assert_eq!(committed_balance(&host, &actor(100), base), 10);
}

/// A work item that does not bring enough gas for its own receipt is ignored, at no cost
/// to the others.
#[test]
fn an_item_without_gas_for_its_own_receipt_is_ignored() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.accept(&bob, brl);
	let item = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 60 });
	let payload = encode_payload(&[item]);
	let out = refine(SVC, &payload);
	let mut host = Metered { inner: w.host.clone(), gas: 1_000_000 };
	let slot = w.slot;
	let mut sess = Session::new(&mut host, SVC, slot, &Genesis::default());
	let before = sess.ctx.host.gas();
	assert!(sess.work(&blake2b(&[&payload]), &out, GAS_COMMIT - 1).is_empty());
	assert_eq!(sess.ctx.host.gas(), before);
	// The signed operation was not touched and can be sent again with enough gas.
	assert_eq!(sess.work(&blake2b(&[&payload]), &out, 500_000), vec![OK]);
	sess.finish();
	assert_eq!(committed_balance(&host, &bob, brl), 60);
}
