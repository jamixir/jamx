//! The order book: batch auctions between two assets that both live on JAM.

mod common;
use common::*;
use jamx_core::{ops::*, types::*};
use rand::{rngs::StdRng, seq::SliceRandom, Rng, SeedableRng};

const OK: u8 = Status::Ok as u8;

fn st(s: Status) -> u8 {
	s as u8
}

/// A market with a base token (`btc`, think of a bridged asset) and a quote token (`brl`).
fn market(w: &mut World) -> (AssetId, AssetId, Actor) {
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let quote = w.token(&bank, "BRL", 0);
	(base, quote, bank)
}

#[test]
fn a_simple_trade_settles_atomically_between_the_two_accounts() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, quote, &bob, 1_000);

	// Bob would pay up to 60 per unit, Alice would accept 50: they meet halfway, at 55.
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (60, 1)),
		w.order(&alice, base, quote, Side::Sell, 10, (50, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);

	assert_eq!((w.bal(&alice, base), w.bal(&alice, quote)), (0, 550));
	assert_eq!((w.bal(&bob, base), w.bal(&bob, quote)), (10, 450));
	assert_eq!(w.bal(&builder, quote), 0);
	let m = w.market(base, quote).unwrap();
	assert!(m.last_price.same_value(&Price::new(55, 1)));
	assert_eq!((m.batches, m.volume), (1, 10));
}

#[test]
fn everyone_in_a_round_gets_the_same_price() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let builder = actor(9);
	let buyers = [actor(10), actor(11), actor(12)];
	let sellers = [actor(20), actor(21), actor(22)];
	for b in &buyers {
		w.mint(&bank, quote, b, 100_000);
	}
	for s in &sellers {
		w.mint(&bank, base, s, 100);
	}
	let orders = vec![
		w.order(&buyers[0], base, quote, Side::Buy, 50, (120, 1)),
		w.order(&buyers[1], base, quote, Side::Buy, 50, (110, 1)),
		w.order(&buyers[2], base, quote, Side::Buy, 50, (90, 1)), // too low: does not trade
		w.order(&sellers[0], base, quote, Side::Sell, 40, (80, 1)),
		w.order(&sellers[1], base, quote, Side::Sell, 40, (100, 1)),
		w.order(&sellers[2], base, quote, Side::Sell, 40, (130, 1)), // too high: does not trade
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	let price = w.market(base, quote).unwrap().last_price;

	// 80 units trade (the two cheaper sellers), shared by the two higher bidders.
	assert_eq!(w.bal(&sellers[0], base), 60);
	assert_eq!(w.bal(&sellers[1], base), 60);
	assert_eq!(w.bal(&sellers[2], base), 100);
	assert_eq!(w.bal(&buyers[2], base), 0);
	assert_eq!(w.bal(&buyers[0], base) + w.bal(&buyers[1], base), 80);
	// The better bid is served first.
	assert_eq!(w.bal(&buyers[0], base), 50);
	// One price for all: each buyer paid, and each seller received, quantity times it.
	for b in &buyers[..2] {
		let got = w.bal(b, base);
		assert_eq!(100_000 - w.bal(b, quote), price.quote_ceil(got).unwrap());
	}
	for s in &sellers[..2] {
		assert_eq!(w.bal(s, quote), price.quote_floor(40).unwrap());
	}
	// And nobody traded beyond their limit.
	assert!(price.cmp_value(&Price::new(110, 1)) != std::cmp::Ordering::Greater);
	assert!(price.cmp_value(&Price::new(100, 1)) != std::cmp::Ordering::Less);
}

#[test]
fn the_bundlers_ordering_changes_nothing() {
	let run = |shuffle_seed: Option<u64>| {
		let mut w = World::new();
		let (base, quote, bank) = market(&mut w);
		let builder = actor(9);
		let mut orders = vec![];
		for i in 0..6u8 {
			let (b, s) = (actor(10 + i), actor(30 + i));
			w.mint(&bank, quote, &b, 1_000_000);
			w.mint(&bank, base, &s, 1_000);
			orders.push(w.order(&b, base, quote, Side::Buy, 100 + 7 * i as u64, (100 + i as u64, 1)));
			orders.push(w.order(&s, base, quote, Side::Sell, 90 + 11 * i as u64, (98 + i as u64, 1)));
		}
		if let Some(seed) = shuffle_seed {
			orders.shuffle(&mut StdRng::seed_from_u64(seed));
		}
		assert_eq!(w.batch(base, quote, &builder, 3, orders), OK);
		// Compare everything that ended up in state, except receipts (which name the payload).
		w.host.store.into_iter().filter(|(k, _)| k[0] != b'r').collect::<Vec<_>>()
	};
	let reference = run(None);
	for seed in 0..5 {
		assert_eq!(run(Some(seed)), reference);
	}
}

#[test]
fn fees_go_to_the_builder_and_are_capped_by_each_order() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 1_000);
	w.mint(&bank, quote, &bob, 1_000_000);
	w.accept(&builder, quote);
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 1_000, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 1_000, (100, 1)),
	];
	// 50 basis points on a notional of 100,000: 500 from each side.
	assert_eq!(w.batch(base, quote, &builder, 50, orders), OK);
	assert_eq!(w.bal(&bob, quote), 1_000_000 - 100_000 - 500);
	assert_eq!(w.bal(&alice, quote), 100_000 - 500);
	assert_eq!(w.bal(&builder, quote), 1_000);

	// A batch cannot charge more than the protocol's ceiling...
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 10, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, MAX_FEE_BPS + 1, orders), st(Status::NoMatch));
	// ...nor more than an order agreed to: such an order is simply left out.
	w.mint(&bank, base, &alice, 10);
	let mut cheap = w.order(&bob, base, quote, Side::Buy, 10, (100, 1));
	cheap = SignedOrder::sign(&bob.seed, SVC, base, quote, Order { max_fee_bps: 5, ..cheap.order });
	let sell = w.order(&alice, base, quote, Side::Sell, 10, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 10, vec![cheap, sell]), st(Status::NoMatch));
}

#[test]
fn an_order_is_cut_down_to_what_its_account_can_fund() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 250); // enough for 2 units at 100, with change
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 100, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 100, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	assert_eq!((w.bal(&bob, base), w.bal(&bob, quote)), (2, 50));
	assert_eq!((w.bal(&alice, base), w.bal(&alice, quote)), (98, 200));
}

#[test]
fn an_unfunded_or_frozen_order_just_drops_out() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, carol, dave, builder) = (actor(2), actor(3), actor(4), actor(5), actor(9));
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, quote, &carol, 10_000);
	w.mint(&bank, quote, &dave, 10_000);
	// Dave freezes his own account before the round settles.
	w.must(&dave, Op::Freeze { target: dave.id });
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),  // Bob has no money at all
		w.order(&dave, base, quote, Side::Buy, 10, (100, 1)), // frozen
		w.order(&carol, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 10, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	// Only Carol could trade; the others cost nobody anything.
	assert_eq!(w.bal(&carol, base), 10);
	assert_eq!(w.bal(&bob, base), 0);
	assert_eq!(w.bal(&dave, base), 0);
	assert_eq!(w.bal(&dave, quote), 10_000);
	assert_eq!(w.bal(&alice, quote), 1_000);
}

#[test]
fn a_filled_order_cannot_be_replayed() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	let buy = w.order(&bob, base, quote, Side::Buy, 10, (100, 1));
	let sell = w.order(&alice, base, quote, Side::Sell, 10, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![buy.clone(), sell.clone()]), OK);
	// The same two signed orders, bundled again by anyone: nothing happens.
	assert_eq!(w.batch(base, quote, &builder, 0, vec![buy.clone(), sell]), st(Status::NoMatch));
	// Bob's spent order against a fresh sell order: still nothing.
	let sell2 = w.order(&alice, base, quote, Side::Sell, 10, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![buy, sell2]), st(Status::NoMatch));
	assert_eq!((w.bal(&bob, base), w.bal(&alice, base)), (10, 90));
}

#[test]
fn a_resting_order_fills_across_rounds_and_then_stops() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, carol, builder) = (actor(2), actor(3), actor(4), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	w.mint(&bank, quote, &carol, 100_000);
	// Alice offers 100 units and leaves the order with the bundlers.
	let sell = w.order(&alice, base, quote, Side::Sell, 100, (100, 1));
	let buy1 = w.order(&bob, base, quote, Side::Buy, 30, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell.clone(), buy1]), OK);
	let open = w.account(&alice.id).open;
	assert_eq!((open.len(), open[0].filled), (1, 30));

	// A later round fills more of the same signed order...
	let buy2 = w.order(&carol, base, quote, Side::Buy, 50, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell.clone(), buy2]), OK);
	assert_eq!(w.account(&alice.id).open[0].filled, 80);

	// ...but never more than she signed for.
	let buy3 = w.order(&bob, base, quote, Side::Buy, 50, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell.clone(), buy3]), OK);
	assert_eq!(w.bal(&alice, base), 0);
	assert!(w.account(&alice.id).open.is_empty());
	assert_eq!(w.bal(&alice, quote), 10_000);
	let buy4 = w.order(&bob, base, quote, Side::Buy, 50, (100, 1));
	w.mint(&bank, base, &alice, 100);
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy4]), st(Status::NoMatch));
	assert_eq!(w.bal(&alice, base), 100);
}

#[test]
fn a_resting_order_can_be_cancelled_and_expires() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	let sell = w.order(&alice, base, quote, Side::Sell, 100, (100, 1));
	let buy = w.order(&bob, base, quote, Side::Buy, 30, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell.clone(), buy]), OK);
	// Cancel by nonce: the rest of the order is dead.
	w.must(&alice, Op::Cancel { nonces: vec![sell.order.nonce] });
	assert!(w.account(&alice.id).open.is_empty());
	let buy = w.order(&bob, base, quote, Side::Buy, 30, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy]), st(Status::NoMatch));

	// A never-filled order can be cancelled before it ever trades.
	let sell = w.order(&alice, base, quote, Side::Sell, 10, (100, 1));
	w.must(&alice, Op::Cancel { nonces: vec![sell.order.nonce] });
	let buy = w.order(&bob, base, quote, Side::Buy, 10, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy]), st(Status::NoMatch));

	// And an order past its expiry does not trade.
	let sell = w.order(&alice, base, quote, Side::Sell, 10, (100, 1));
	let buy = w.order(&bob, base, quote, Side::Buy, 10, (100, 1));
	w.slot = sell.order.expiry + 1;
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy]), st(Status::NoMatch));
	assert_eq!(w.bal(&alice, base), 70);
}

#[test]
fn immediate_or_cancel_orders_do_not_rest() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	let sell = w.order_with(&alice, base, quote, Side::Sell, 100, (100, 1), ORDER_IOC, vec![]);
	let buy = w.order(&bob, base, quote, Side::Buy, 30, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell.clone(), buy]), OK);
	assert!(w.account(&alice.id).open.is_empty());
	let buy = w.order(&bob, base, quote, Side::Buy, 30, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![sell, buy]), st(Status::NoMatch));
	assert_eq!(w.bal(&alice, base), 70);
}

#[test]
fn two_orders_of_one_account_cannot_spend_the_same_money() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 1_000); // funds for 10 units in total
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 100, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	assert_eq!((w.bal(&bob, base), w.bal(&bob, quote)), (10, 0));
}

#[test]
fn the_daily_limit_also_binds_trades() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: base, per_day: Some(25) }));
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 100, (100, 1)),
		w.order(&alice, base, quote, Side::Sell, 100, (100, 1)),
	];
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	// Selling everything at once to an accomplice is exactly what a thief would try.
	assert_eq!(w.bal(&alice, base), 75);
}

#[test]
fn a_trader_key_can_place_orders() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, phone, bob, builder) = (actor(2), actor(22), actor(3), actor(9));
	w.mint(&bank, base, &alice, 100);
	w.mint(&bank, quote, &bob, 100_000);
	w.must(&alice, Op::Policy(PolicyChange::AddKey { key: phone.id, role: ROLE_TRADER }));
	let nonce = w.nonce(&alice.id);
	let order = Order {
		account: alice.id,
		nonce,
		side: Side::Sell,
		qty: 10,
		limit: Price::new(100, 1),
		expiry: w.slot + 100,
		max_fee_bps: 0,
		flags: 0,
		delivery: vec![],
	};
	// Signed by the phone, for Alice's account.
	let by_phone = SignedOrder::sign(&phone.seed, SVC, base, quote, order.clone());
	// Signed by a stranger, claiming to be Alice.
	let forged = SignedOrder::sign(&actor(66).seed, SVC, base, quote, Order { nonce: nonce + 1, ..order });
	let buy = w.order(&bob, base, quote, Side::Buy, 20, (100, 1));
	assert_eq!(w.batch(base, quote, &builder, 0, vec![by_phone, forged, buy]), OK);
	assert_eq!(w.bal(&alice, base), 90);
}

#[test]
fn permissioned_quote_token_keeps_strangers_out_of_the_fiat_leg() {
	let mut w = World::new();
	let bank = actor(1);
	let base = w.token(&bank, "WBTC", 0);
	let brl = w.token(&bank, "BRL", ASSET_PERMISSIONED);
	let (alice, bob, mallory, builder) = (actor(2), actor(3), actor(66), actor(9));
	let allow = |t: &Actor| Op::IssuerSet { asset: brl, target: t.id, action: IssuerAction::Allow, reason: [0; 32] };
	w.accept(&bob, brl);
	w.accept(&alice, brl);
	w.must(&bank, allow(&bob));
	w.must(&bank, allow(&alice));
	w.mint(&bank, brl, &bob, 10_000);
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, base, &mallory, 10);
	// Mallory offers a better price, but has not passed the bank's checks and so cannot
	// receive the bank's token. Her order is left out; Alice's trades.
	let orders = vec![
		w.order(&bob, base, brl, Side::Buy, 10, (100, 1)),
		w.order(&mallory, base, brl, Side::Sell, 10, (90, 1)),
		w.order(&alice, base, brl, Side::Sell, 10, (100, 1)),
	];
	// The builder has not been approved either: its fee is destroyed rather than paid.
	assert_eq!(w.batch(base, brl, &builder, 10, orders), OK);
	assert_eq!(w.bal(&mallory, base), 10);
	assert_eq!(w.bal(&alice, base), 0);
	assert_eq!(w.bal(&builder, brl), 0);
	assert!(w.asset(brl).supply < 10_000);
}

#[test]
fn markets_need_two_real_assets() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, builder) = (actor(2), actor(3), actor(9));
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, quote, &bob, 1_000);
	let mk = |w: &mut World, b, q| {
		vec![w.order(&bob, b, q, Side::Buy, 10, (100, 1)), w.order(&alice, b, q, Side::Sell, 10, (100, 1))]
	};
	let o = mk(&mut w, base, 77);
	assert_eq!(w.batch(base, 77, &builder, 0, o), st(Status::NotFound));
	let o = mk(&mut w, 77, quote);
	assert_eq!(w.batch(77, quote, &builder, 0, o), st(Status::NotFound));
	let o = mk(&mut w, base, base);
	assert_eq!(w.batch(base, base, &builder, 0, o), st(Status::NoMatch));
	// Orders signed for one market mean nothing in another.
	let o = mk(&mut w, quote, base);
	assert_eq!(w.batch(base, quote, &builder, 0, o), st(Status::NoMatch));
}

/// Many random rounds with random books and balances. After every round the test harness
/// checks that holdings equal supply; here we also check that no order was ever filled
/// beyond its size or its limit price, and that nobody's balance moved without an order.
#[test]
fn random_rounds_conserve_everything_and_respect_every_limit() {
	let mut rng = StdRng::seed_from_u64(7);
	for round in 0..300 {
		let mut w = World::new();
		let (base, quote, bank) = market(&mut w);
		// The builder keeps a little of the quote asset: rounding on very small fills goes
		// the traders' way and the builder makes up the atom.
		let builder = actor(200);
		w.mint(&bank, quote, &builder, 1_000);
		let n = rng.gen_range(2..14usize);
		let traders: Vec<Actor> = (0..n).map(|i| actor(10 + i as u8)).collect();
		for t in &traders {
			if rng.gen_bool(0.8) {
				w.mint(&bank, base, t, rng.gen_range(1..5_000));
			}
			if rng.gen_bool(0.8) {
				w.mint(&bank, quote, t, rng.gen_range(1..500_000));
			}
		}
		let before: Vec<(u64, u64)> = traders.iter().map(|t| (w.bal(t, base), w.bal(t, quote))).collect();
		let fee_bps = rng.gen_range(0..=30u16);
		let mut orders = vec![];
		let mut meta = vec![]; // (trader index, side, qty, limit)
		for _ in 0..rng.gen_range(1..25) {
			let i = rng.gen_range(0..n);
			let side = if rng.gen_bool(0.5) { Side::Buy } else { Side::Sell };
			let qty = rng.gen_range(1..3_000);
			let limit = (rng.gen_range(50..150u64), rng.gen_range(1..4u64));
			orders.push(w.order(&traders[i], base, quote, side, qty, limit));
			meta.push((i, side, qty, Price::new(limit.0, limit.1)));
		}
		let status = w.batch(base, quote, &builder, fee_bps, orders);
		assert!(status == OK || status == st(Status::NoMatch), "round {round}: status {status}");
		if status != OK {
			for (t, b) in traders.iter().zip(&before) {
				assert_eq!((w.bal(t, base), w.bal(t, quote)), *b);
			}
			continue;
		}
		let price = w.market(base, quote).unwrap().last_price;
		for (i, t) in traders.iter().enumerate() {
			let (b0, q0) = before[i];
			let (b1, q1) = (w.bal(t, base), w.bal(t, quote));
			let bought: u64 = meta.iter().filter(|m| m.0 == i && m.1 == Side::Buy).map(|m| m.2).sum();
			let sold: u64 = meta.iter().filter(|m| m.0 == i && m.1 == Side::Sell).map(|m| m.2).sum();
			// Base moved by no more than the orders allowed, in either direction.
			assert!(b1 <= b0 + bought, "round {round}: bought more than ordered");
			assert!(b1 + sold >= b0, "round {round}: sold more than ordered");
			// A trader with only buy orders paid at most the clearing price plus the fee,
			// and the clearing price is within each of their limits that traded.
			let only_buys = sold == 0 && bought > 0;
			let only_sells = bought == 0 && sold > 0;
			if only_buys && b1 > b0 {
				let got = b1 - b0;
				let gross = price.quote_ceil(got).unwrap() + meta.iter().filter(|m| m.0 == i).count() as u64;
				let max_paid = gross + gross * fee_bps as u64 / 10_000 + 1;
				assert!(q0 - q1 <= max_paid, "round {round}: paid {} for {got} at {price:?}", q0 - q1);
				let best_limit = meta.iter().filter(|m| m.0 == i).map(|m| m.3).max_by(|a, b| a.cmp_value(b)).unwrap();
				assert!(price.cmp_value(&best_limit) != std::cmp::Ordering::Greater);
			}
			if only_sells && b1 < b0 {
				let gave = b0 - b1;
				let gross = price.quote_floor(gave).unwrap();
				let fills = meta.iter().filter(|m| m.0 == i).count() as u64;
				let min_received = (gross - gross * fee_bps as u64 / 10_000).saturating_sub(2 * fills);
				assert!(q1 - q0 >= min_received, "round {round}: received {} for {gave}", q1 - q0);
				let best_limit = meta.iter().filter(|m| m.0 == i).map(|m| m.3).min_by(|a, b| a.cmp_value(b)).unwrap();
				assert!(price.cmp_value(&best_limit) != std::cmp::Ordering::Less);
			}
			if bought == 0 && sold == 0 {
				assert_eq!((b1, q1), (b0, q0), "round {round}: a bystander's balance moved");
			}
		}
	}
}

#[test]
fn an_account_that_cannot_take_delivery_cannot_void_a_round() {
	let mut w = World::new();
	let (base, quote, bank) = market(&mut w);
	let (alice, bob, hoarder, builder) = (actor(2), actor(3), actor(66), actor(9));
	w.mint(&bank, base, &alice, 10);
	w.mint(&bank, quote, &bob, 10_000);
	w.mint(&bank, base, &hoarder, 10);
	// The hoarder fills its account with worthless assets up to the limit, so that it could
	// not receive the quote asset, then offers the best price in the round.
	let mut items = vec![];
	for i in 0..MAX_BALANCES as u32 - 1 {
		let id = w.next_asset() + i;
		items.push(w.sign(&hoarder, Op::AssetCreate {
			symbol: sym("JUNK"),
			decimals: 0,
			flags: 0,
			kind: AssetKindCfg::Token,
			attestors: vec![],
			threshold: 0,
		}));
		items.push(w.sign(&hoarder, Op::Mint { asset: id, to: hoarder.id, amount: 1 }));
	}
	for chunk in items.chunks(100) {
		assert!(ok(&w.submit(chunk.to_vec())));
	}
	assert_eq!(w.account(&hoarder.id).balances.len(), MAX_BALANCES);
	let orders = vec![
		w.order(&bob, base, quote, Side::Buy, 10, (100, 1)),
		w.order(&hoarder, base, quote, Side::Sell, 10, (90, 1)),
		w.order(&alice, base, quote, Side::Sell, 10, (100, 1)),
	];
	// The round goes ahead without the hoarder.
	assert_eq!(w.batch(base, quote, &builder, 0, orders), OK);
	assert_eq!(w.bal(&alice, base), 0);
	assert_eq!(w.bal(&hoarder, base), 10);
	// The hoarder has no room to accept the quote asset, so nobody can send it any.
	assert_eq!(w.op(&hoarder, Op::Accept { asset: quote, on: true }), st(Status::Invalid));
	assert_eq!(w.op(&bob, Op::Transfer { asset: quote, to: hoarder.id, amount: 1 }), st(Status::NotAllowed));
}
