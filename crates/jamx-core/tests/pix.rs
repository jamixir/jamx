//! The fiat leg without a custodian of crypto: a token locked on JAM, reais sent by Pix,
//! and an oracle (or a human arbiter) whose reach is that one trade.

mod common;
use common::*;
use jamx_core::{ops::*, types::*};

const OK: u8 = Status::Ok as u8;

fn st(s: Status) -> u8 {
	s as u8
}

struct Setup {
	w: World,
	pix: AssetId,
	usd: AssetId,
	/// Holds a dollar token on JAM and wants reais in her bank account.
	seller: Actor,
	/// Has reais in the bank and wants the token; will pay by Pix.
	buyer: Actor,
	oracles: [Actor; 3],
	builder: Actor,
}

/// The market is PIX/USD-token: the "base" is reais delivered by Pix (in centavos), the
/// quote is a token on JAM. Whoever wants the reais locks tokens; whoever sends the Pix
/// posts a bond and receives the tokens once the payment is attested.
fn setup(threshold: u8) -> Setup {
	let mut w = World::new();
	let (issuer, seller, buyer, builder) = (actor(1), actor(2), actor(3), actor(9));
	let oracles = [actor(41), actor(42), actor(43)];
	let usd = w.token(&issuer, "USDX", 0);
	let pix = w.next_asset();
	w.must(
		&builder,
		Op::AssetCreate {
			symbol: sym("PIX"),
			decimals: 2,
			flags: 0,
			kind: AssetKindCfg::External {
				cfg: ExternalCfg {
					rail: Rail::Oracle { oracles: oracles.iter().map(|o| o.id).collect(), threshold },
					bond_bps: 500,
					claim_window: 300, // half an hour
					min_deal: 100,
				},
				btc: None,
			},
			attestors: vec![],
			threshold: 0,
		},
	);
	w.mint(&issuer, usd, &seller, 10_000);
	w.mint(&issuer, usd, &buyer, 1_000); // for the bond
	Setup { w, pix, usd, seller, buyer, oracles, builder }
}

impl Setup {
	/// 50,000 centavos (R$ 500) for 100 token units: 1 token per R$ 5.
	fn trade(&mut self) -> DealId {
		let pix_key = b"pix:seller@bank.example".to_vec();
		let want_reais =
			self.w.order_with(&self.seller, self.pix, self.usd, Side::Buy, 50_000, (1, 500), 0, pix_key);
		let send_reais = self.w.order(&self.buyer, self.pix, self.usd, Side::Sell, 50_000, (1, 500));
		let builder = self.builder;
		assert_eq!(self.w.batch(self.pix, self.usd, &builder, 0, vec![want_reais, send_reais]), OK);
		self.w.deals().last().unwrap().0
	}
}

#[test]
fn tokens_are_released_when_the_oracle_sees_the_pix() {
	let mut s = setup(1);
	let d = s.trade();
	let deal = s.w.deal(d).unwrap();
	assert_eq!((deal.payer, deal.payee), (s.seller.id, s.buyer.id));
	assert_eq!((deal.ext_amount, deal.amount, deal.bond), (50_000, 100, 5));
	assert_eq!(deal.delivery, b"pix:seller@bank.example".to_vec());
	assert_eq!(s.w.bal(&s.seller, s.usd), 9_900);

	// A stranger's word counts for nothing.
	let stranger = actor(66);
	assert_eq!(s.w.op(&stranger, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), st(Status::Unauthorized));
	// The buyer cannot vouch for his own payment either.
	let buyer = s.buyer;
	assert_eq!(s.w.op(&buyer, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), st(Status::Unauthorized));

	let oracle = s.oracles[0];
	assert_eq!(s.w.op(&oracle, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), OK);
	assert!(s.w.deal(d).is_none());
	assert_eq!(s.w.bal(&s.buyer, s.usd), 1_000 + 100);
	// The oracle decided one escrow. It never had a way to touch anything else.
	assert_eq!(s.w.bal(&s.seller, s.usd), 9_900);
}

#[test]
fn the_seller_can_simply_confirm_receipt() {
	let mut s = setup(1);
	let d = s.trade();
	// The usual case needs no oracle at all: the money arrived and she says so.
	let seller = s.seller;
	assert_eq!(s.w.op(&seller, Op::DealRelease { deal: d }), OK);
	assert_eq!(s.w.bal(&s.buyer, s.usd), 1_100);
}

#[test]
fn if_the_pix_never_comes_the_tokens_go_back_with_the_bond() {
	let mut s = setup(1);
	let d = s.trade();
	let oracle = s.oracles[1];
	// The oracle can say so explicitly...
	assert_eq!(s.w.op(&oracle, Op::OracleAttest { deal: d, verdict: Verdict::NotPaid }), OK);
	assert_eq!(s.w.bal(&s.seller, s.usd), 10_000 + 5);
	assert_eq!(s.w.bal(&s.buyer, s.usd), 995);

	// ...or the deal just times out.
	let d = s.trade();
	s.w.slot += 301;
	assert_eq!(s.w.op(&oracle, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), st(Status::Expired));
	assert_eq!(s.w.plain(PlainOp::DealTimeout { deal: d }), OK);
	assert_eq!(s.w.bal(&s.seller, s.usd), 10_000 + 10);
}

#[test]
fn several_oracles_must_agree() {
	let mut s = setup(2);
	let d = s.trade();
	let [o1, o2, o3] = s.oracles;
	assert_eq!(s.w.op(&o1, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), OK);
	assert!(s.w.deal(d).is_some());
	// Repeating oneself does not count twice.
	assert_eq!(s.w.op(&o1, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), OK);
	assert!(s.w.deal(d).is_some());
	// A dissenting oracle: one yes, one no, still no quorum.
	assert_eq!(s.w.op(&o2, Op::OracleAttest { deal: d, verdict: Verdict::NotPaid }), OK);
	assert!(s.w.deal(d).is_some());
	// The third breaks the tie.
	assert_eq!(s.w.op(&o3, Op::OracleAttest { deal: d, verdict: Verdict::Paid }), OK);
	assert!(s.w.deal(d).is_none());
	assert_eq!(s.w.bal(&s.buyer, s.usd), 1_100);
}

#[test]
fn a_bitcoin_receipt_means_nothing_on_an_oracle_rail() {
	let mut s = setup(1);
	let d = s.trade();
	// A well-formed receipt (a block holding one transaction, its own coinbase).
	let tx = jamx_core::btc::build_coinbase(1, &[(50_000, vec![0x51])]);
	let proof = BtcProof {
		deal: d,
		block_hash: [1; 32],
		tx: tx.clone(),
		vout: 0,
		index: 0,
		branch: vec![],
		coinbase: tx,
		coinbase_branch: vec![],
	};
	assert_eq!(s.w.submit(vec![Item::BtcProof(proof)]), vec![st(Status::Invalid)]);
	assert!(s.w.deal(d).is_some());
}

#[test]
fn a_pix_key_that_cannot_be_paid_voids_the_deal_and_costs_nobody() {
	let mut s = setup(2);
	let d = s.trade();
	let [o1, o2, _] = s.oracles;
	// The key the seller gave does not exist. That is not the buyer's failure to pay, and
	// the buyer's bond must not be the price of it.
	assert_eq!(s.w.op(&o1, Op::OracleAttest { deal: d, verdict: Verdict::Void }), OK);
	assert!(s.w.deal(d).is_some());
	assert_eq!(s.w.op(&o2, Op::OracleAttest { deal: d, verdict: Verdict::Void }), OK);
	assert!(s.w.deal(d).is_none());
	assert_eq!(s.w.bal(&s.seller, s.usd), 10_000);
	assert_eq!(s.w.bal(&s.buyer, s.usd), 1_000);
}
