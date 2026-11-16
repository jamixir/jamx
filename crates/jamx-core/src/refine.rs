//! The stateless half: everything that can be checked without reading the chain.
//!
//! Refine runs in-core, in parallel across JAM's lanes, with a budget of seconds rather
//! than the milliseconds accumulate gets. It does the expensive work and hands accumulate a
//! compact, already-authenticated list of records:
//!
//! * every signature is verified here, and only here;
//! * every order batch is authenticated, sorted and matched, and reduced to the orders
//!   that cross;
//! * every Bitcoin header has its proof-of-work checked, and every receipt its Merkle path.
//!
//! Refine cannot see balances or anything else in state. Anything that depends on state
//! (is the account funded? is this nonce spent? is that header on the best chain?) is
//! decided by accumulate. An invalid item is simply dropped: it produces no record.

use crate::{
	auction::{self, Quote},
	btc,
	codec::{Codec, Reader},
	crypto::{blake2b, ed25519_verify, sha256d, Hash32},
	ops::*,
	types::*,
};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Largest refine output. JAM caps the variable-size part of a work-report at 48 KiB
/// (`W_R`); some room is left for the authorizer trace and other items in the package.
pub const MAX_OUTPUT: usize = 46_000;

/// Refine a whole payload into the work output.
pub fn refine(service: u32, payload: &[u8]) -> Vec<u8> {
	refine_with_limit(service, payload, MAX_OUTPUT)
}

pub fn refine_with_limit(service: u32, payload: &[u8], max_output: usize) -> Vec<u8> {
	let Ok(frames) = payload_frames(payload) else { return Vec::new() };
	let mut out = Vec::new();
	for (tag, body) in frames {
		let Ok(item) = Item::dec_body(tag, &mut Reader::new(body)) else { continue };
		let Some(rec) = refine_item(service, item) else { continue };
		let before = out.len();
		push_record(&mut out, &rec);
		if out.len() > max_output {
			// Does not fit: leave it for another package rather than invalidate this one.
			out.truncate(before);
		}
	}
	out
}

/// Check one item. `None` means it is invalid on its face and is dropped.
pub fn refine_item(service: u32, item: Item) -> Option<Record> {
	match item {
		Item::Signed(s) => {
			Envelope::from_bytes(&s.body).ok()?;
			ed25519_verify(&s.signer, &op_message(service, &s.body), &s.sig)
				.then_some(Record::Op { signer: s.signer, body: s.body })
		},
		Item::Batch(b) => refine_batch(service, b).map(Record::Batch),
		Item::BtcHeaders { asset, headers } => refine_headers(asset, &headers),
		Item::BtcProof(p) => refine_proof(service, &p).map(Record::BtcPayment),
		Item::Plain(p) => Some(Record::Plain(p)),
	}
}

/// Authenticate the orders of a batch, sort them, and match them.
///
/// The result does not depend on the order in which the bundler listed the orders: they
/// are sorted by price and then by hash before anything else happens. What comes out is
/// the crossing book; see [`BatchRec`].
pub fn refine_batch(service: u32, b: Batch) -> Option<BatchRec> {
	if b.base == b.quote || b.fee_bps > MAX_FEE_BPS {
		return None;
	}
	let mut bids: Vec<(Hash32, OrderRec)> = Vec::new();
	let mut asks: Vec<(Hash32, OrderRec)> = Vec::new();
	for so in b.orders {
		let o = so.order;
		if o.qty == 0 || !o.limit.valid() || o.max_fee_bps < b.fee_bps {
			continue;
		}
		let msg = order_message(service, b.base, b.quote, &o.to_bytes());
		if !ed25519_verify(&so.signer, &msg, &so.sig) {
			continue;
		}
		let hash = blake2b(&[&msg]);
		let mut tag = [0u8; 8];
		if o.flags & ORDER_IOC == 0 {
			tag.copy_from_slice(&hash[..8]);
		}
		let rec = OrderRec {
			signer: so.signer,
			account: o.account,
			nonce: o.nonce,
			qty: o.qty,
			limit: o.limit,
			expiry: o.expiry,
			max_fee_bps: o.max_fee_bps,
			flags: o.flags,
			tag,
			delivery: o.delivery,
		};
		match o.side {
			Side::Buy => bids.push((hash, rec)),
			Side::Sell => asks.push((hash, rec)),
		}
	}

	// Canonical order: best price first, ties by order hash.
	bids.sort_unstable_by(|a, b| b.1.limit.cmp_value(&a.1.limit).then(a.0.cmp(&b.0)));
	asks.sort_unstable_by(|a, b| a.1.limit.cmp_value(&b.1.limit).then(a.0.cmp(&b.0)));
	// One order per (account, nonce): a signer cannot enter the same nonce twice.
	dedup_nonces(&mut bids, &mut asks);

	// Run the auction on the orders as signed. If nothing crosses there is no round.
	let quotes = |v: &[(Hash32, OrderRec)]| -> Vec<Quote> {
		v.iter().map(|(_, o)| Quote { price: o.limit, qty: o.qty }).collect()
	};
	auction::discover(&quotes(&bids), &quotes(&asks))?;

	// Keep the crossing book: a bid below every ask, or an ask above every bid, cannot
	// trade in this round whatever accumulate finds, and need not travel any further.
	let best_ask = asks.first()?.1.limit;
	let best_bid = bids.first()?.1.limit;
	let bids: Vec<OrderRec> = bids
		.into_iter()
		.map(|(_, o)| o)
		.filter(|o| o.limit.cmp_value(&best_ask) != Ordering::Less)
		.collect();
	let asks: Vec<OrderRec> = asks
		.into_iter()
		.map(|(_, o)| o)
		.filter(|o| o.limit.cmp_value(&best_bid) != Ordering::Greater)
		.collect();
	Some(BatchRec { base: b.base, quote: b.quote, fee_account: b.fee_account, fee_bps: b.fee_bps, bids, asks })
}

fn dedup_nonces(bids: &mut Vec<(Hash32, OrderRec)>, asks: &mut Vec<(Hash32, OrderRec)>) {
	let mut seen: Vec<(AccountId, u64)> = Vec::with_capacity(bids.len() + asks.len());
	let mut keep = |o: &OrderRec| -> bool {
		let k = (o.account, o.nonce);
		match seen.binary_search(&k) {
			Ok(_) => false,
			Err(i) => {
				seen.insert(i, k);
				true
			},
		}
	};
	bids.retain(|(_, o)| keep(o));
	asks.retain(|(_, o)| keep(o));
}

/// Check that the headers link to one another and that each carries the proof-of-work its
/// own difficulty field demands. Stops at the first header that fails and keeps what came
/// before it. Whether the difficulty itself is right is for accumulate, which knows the chain.
pub fn refine_headers(asset: AssetId, headers: &[[u8; 80]]) -> Option<Record> {
	let first = btc::Header::parse(headers.first()?);
	let prev = first.prev;
	let mut expected_prev = prev;
	let mut digests = Vec::with_capacity(headers.len());
	for raw in headers {
		let h = btc::Header::parse(raw);
		let hash = btc::header_hash(raw);
		if h.prev != expected_prev || !btc::check_pow_self(&hash, h.bits) {
			break;
		}
		digests.push(HeaderDigest { hash, merkle_root: h.merkle_root, time: h.time, bits: h.bits });
		expected_prev = hash;
		if digests.len() == MAX_HEADERS_PER_ITEM {
			break;
		}
	}
	(!digests.is_empty()).then_some(Record::BtcHeaders { asset, prev, headers: digests })
}

/// Check a receipt: compute the transaction's identifier, walk the Merkle path to a root,
/// and read the output it claims to pay.
///
/// The receipt must also show the block's coinbase under the same root, by a path of the
/// same length. All transactions of a block sit at the same depth of its Merkle tree, and
/// nothing but a coinbase looks like one, so this pins the depth: neither can an inner node
/// be passed off as a transaction, nor a transaction as an inner node with an invented
/// transaction beneath it.
pub fn refine_proof(service: u32, p: &BtcProof) -> Option<PaymentRec> {
	let outs = btc::parse_outputs(&p.tx).ok()?;
	let out = outs.get(p.vout as usize)?;
	if out.script.len() > MAX_SCRIPT_LEN {
		return None;
	}
	let txid = sha256d(&p.tx);
	let merkle_root = btc::merkle_root(&txid, p.index, &p.branch)?;
	if !btc::is_coinbase(&p.coinbase) || p.coinbase_branch.len() != p.branch.len() {
		return None;
	}
	if btc::merkle_root(&sha256d(&p.coinbase), 0, &p.coinbase_branch)? != merkle_root {
		return None;
	}
	let tag_script = btc::tag_script(&deal_tag(service, p.deal));
	let tag = if outs.iter().any(|o| o.script == &tag_script[..]) {
		TagState::This
	} else if outs.iter().any(|o| btc::is_tag_script(o.script)) {
		TagState::Other
	} else {
		TagState::None
	};
	Some(PaymentRec {
		deal: p.deal,
		block_hash: p.block_hash,
		merkle_root,
		txid,
		vout: p.vout,
		value: out.value,
		script: out.script.to_vec(),
		tag,
	})
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::crypto::ed25519_public;
	use alloc::vec;

	const SVC: u32 = 77;

	fn order(seed: u8, nonce: u64, side: Side, qty: u64, num: u64, den: u64) -> SignedOrder {
		let s = [seed; 32];
		SignedOrder::sign(
			&s,
			SVC,
			1,
			2,
			Order {
				account: ed25519_public(&s),
				nonce,
				side,
				qty,
				limit: Price::new(num, den),
				expiry: 1000,
				max_fee_bps: 10,
				flags: 0,
				delivery: vec![],
			},
		)
	}

	fn batch(orders: Vec<SignedOrder>) -> Batch {
		Batch { base: 1, quote: 2, fee_account: [9; 32], fee_bps: 5, orders }
	}

	fn records(out: &[u8]) -> Vec<Record> {
		output_frames(out)
			.unwrap()
			.into_iter()
			.map(|(t, b)| Record::dec_body(t, &mut Reader::new(b)).unwrap())
			.collect()
	}

	#[test]
	fn signed_ops_pass_only_with_a_valid_signature() {
		let seed = [1u8; 32];
		let env = Envelope {
			account: ed25519_public(&seed),
			nonce: 0,
			expiry: 100,
			op: Op::Transfer { asset: 1, to: [2; 32], amount: 5 },
		};
		let good = SignedOp::sign(&seed, SVC, &env);
		let mut forged = good.clone();
		forged.sig[0] ^= 1;
		let mut tampered = good.clone();
		*tampered.body.last_mut().unwrap() ^= 1;
		let other_service = SignedOp::sign(&seed, SVC + 1, &env);
		let garbage = SignedOp { signer: good.signer, sig: good.sig, body: vec![1, 2, 3] };
		let payload = encode_payload(&[
			Item::Signed(forged),
			Item::Signed(good.clone()),
			Item::Signed(tampered),
			Item::Signed(other_service),
			Item::Signed(garbage),
		]);
		let recs = records(&refine(SVC, &payload));
		assert_eq!(recs, vec![Record::Op { signer: good.signer, body: good.body }]);
	}

	#[test]
	fn batch_is_matched_and_pruned() {
		let orders = vec![
			order(1, 0, Side::Buy, 10, 12, 1),
			order(2, 0, Side::Buy, 10, 9, 1),
			order(5, 0, Side::Buy, 10, 7, 1), // below every ask: cannot trade, dropped
			order(3, 0, Side::Sell, 10, 8, 1),
			order(4, 0, Side::Sell, 10, 20, 1), // above every bid: dropped
		];
		let rec = refine_batch(SVC, batch(orders)).unwrap();
		// Best price first on each side.
		assert_eq!(
			rec.bids.iter().map(|o| o.account).collect::<Vec<_>>(),
			vec![ed25519_public(&[1; 32]), ed25519_public(&[2; 32])]
		);
		assert_eq!(rec.asks.len(), 1);
		assert_eq!(rec.asks[0].account, ed25519_public(&[3; 32]));
		assert_ne!(rec.bids[0].tag, [0; 8]);
	}

	#[test]
	fn batch_result_ignores_the_bundlers_ordering() {
		let mut orders = vec![
			order(1, 0, Side::Buy, 10, 12, 1),
			order(2, 0, Side::Buy, 7, 12, 1),
			order(5, 0, Side::Buy, 3, 11, 1),
			order(3, 0, Side::Sell, 10, 8, 1),
			order(4, 0, Side::Sell, 6, 8, 1),
			order(6, 0, Side::Sell, 9, 10, 1),
		];
		let a = refine_batch(SVC, batch(orders.clone())).unwrap();
		orders.reverse();
		let b = refine_batch(SVC, batch(orders.clone())).unwrap();
		orders.swap(1, 4);
		let c = refine_batch(SVC, batch(orders)).unwrap();
		assert_eq!(a, b);
		assert_eq!(a, c);
	}

	#[test]
	fn batch_drops_bad_signatures_and_duplicate_nonces() {
		let mut forged = order(1, 0, Side::Buy, 10, 12, 1);
		forged.order.qty = 1000; // signature no longer matches
		let wrong_market = {
			let s = [7u8; 32];
			SignedOrder::sign(
				&s,
				SVC,
				2,
				1,
				Order {
					account: ed25519_public(&s),
					nonce: 0,
					side: Side::Buy,
					qty: 500,
					limit: Price::new(50, 1),
					expiry: 1000,
					max_fee_bps: 10,
					flags: 0,
					delivery: vec![],
				},
			)
		};
		let orders = vec![
			forged,
			wrong_market,
			order(2, 0, Side::Buy, 10, 12, 1),
			order(2, 0, Side::Buy, 99, 12, 1), // same account and nonce, different order
			order(3, 0, Side::Sell, 10, 8, 1),
		];
		let rec = refine_batch(SVC, batch(orders)).unwrap();
		assert_eq!(rec.bids.len(), 1);
		assert_eq!(rec.bids[0].account, ed25519_public(&[2; 32]));
	}

	#[test]
	fn batch_without_a_cross_produces_nothing() {
		let orders = vec![order(1, 0, Side::Buy, 10, 7, 1), order(3, 0, Side::Sell, 10, 8, 1)];
		assert!(refine_batch(SVC, batch(orders)).is_none());
		let mut b = batch(vec![]);
		b.fee_bps = MAX_FEE_BPS + 1;
		assert!(refine_batch(SVC, b).is_none());
	}

	#[test]
	fn orders_refusing_the_fee_are_left_out() {
		let mut b = batch(vec![order(1, 0, Side::Buy, 10, 12, 1), order(3, 0, Side::Sell, 10, 8, 1)]);
		b.fee_bps = 11; // both orders accept at most 10
		assert!(refine_batch(SVC, b).is_none());
	}

	#[test]
	fn headers_must_link_and_carry_work() {
		let bits = btc::REGTEST.pow_limit_bits;
		let h1 = btc::mine([0; 32], [1; 32], 100, bits);
		let h2 = btc::mine(h1.hash(), [2; 32], 101, bits);
		let h3 = btc::mine(h2.hash(), [3; 32], 102, bits);
		let orphan = btc::mine([9; 32], [4; 32], 103, bits);
		let rec = refine_headers(5, &[h1.serialize(), h2.serialize(), orphan.serialize(), h3.serialize()]);
		let Some(Record::BtcHeaders { asset, prev, headers }) = rec else { panic!() };
		assert_eq!((asset, prev), (5, [0; 32]));
		assert_eq!(headers.len(), 2);
		assert_eq!(headers[1].hash, h2.hash());

		// A header claiming mainnet difficulty with regtest work is refused outright.
		let mut lazy = btc::mine([0; 32], [1; 32], 100, bits);
		lazy.bits = 0x1703_4219;
		assert!(refine_headers(5, &[lazy.serialize()]).is_none());
		assert!(refine_headers(5, &[]).is_none());
	}

	#[test]
	fn receipts_are_tied_to_the_deal() {
		let script = vec![0x00, 0x14, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7];
		let tagged_tx = btc::build_tx(1, &[(5000, script.clone()), (0, btc::tag_script(&deal_tag(SVC, 42)))]);
		let plain_tx = btc::build_tx(2, &[(5000, script.clone())]);
		let coinbase = btc::build_coinbase(7, &[(50, vec![0x51])]);
		let txids = [sha256d(&coinbase), sha256d(&tagged_tx), sha256d(&plain_tx)];
		let txid = txids[1];
		let (root, branch) = btc::merkle_branch(&txids, 1);
		let (_, coinbase_branch) = btc::merkle_branch(&txids, 0);
		let proof = BtcProof {
			deal: 42,
			block_hash: [5; 32],
			tx: tagged_tx.clone(),
			vout: 0,
			index: 1,
			branch,
			coinbase: coinbase.clone(),
			coinbase_branch: coinbase_branch.clone(),
		};
		let rec = refine_proof(SVC, &proof).unwrap();
		assert_eq!(rec.merkle_root, root);
		assert_eq!((rec.value, rec.txid, rec.tag), (5000, txid, TagState::This));
		assert_eq!(rec.script, script);
		// The same transaction offered for another deal is marked as made for a different one.
		let rec2 = refine_proof(SVC, &BtcProof { deal: 43, ..proof.clone() }).unwrap();
		assert_eq!(rec2.tag, TagState::Other);
		// A transaction with no tag at all says so.
		let (_, plain_branch) = btc::merkle_branch(&txids, 2);
		let untagged = BtcProof { tx: plain_tx, index: 2, branch: plain_branch, ..proof.clone() };
		assert_eq!(refine_proof(SVC, &untagged).unwrap().tag, TagState::None);
		// A wrong position gives a different root from the coinbase's: refused here.
		assert!(refine_proof(SVC, &BtcProof { index: 0, ..proof.clone() }).is_none());
		// The coinbase must be one, must sit under the same root, at the same depth.
		assert!(refine_proof(SVC, &BtcProof { coinbase: tagged_tx.clone(), ..proof.clone() }).is_none());
		let mut wrong = coinbase_branch.clone();
		wrong[0][0] ^= 1;
		assert!(refine_proof(SVC, &BtcProof { coinbase_branch: wrong, ..proof.clone() }).is_none());
		let mut longer = coinbase_branch.clone();
		longer.push([0; 32]);
		assert!(refine_proof(SVC, &BtcProof { coinbase_branch: longer, ..proof.clone() }).is_none());
		// A missing output or a malformed transaction gives nothing.
		assert!(refine_proof(SVC, &BtcProof { vout: 9, ..proof.clone() }).is_none());
		assert!(refine_proof(SVC, &BtcProof { tx: vec![1, 2, 3], ..proof }).is_none());
	}

	#[test]
	fn oversized_output_drops_whole_records() {
		let seed = [1u8; 32];
		let mk = |n| {
			Item::Signed(SignedOp::sign(
				&seed,
				SVC,
				&Envelope {
					account: ed25519_public(&seed),
					nonce: n,
					expiry: 100,
					op: Op::Transfer { asset: 1, to: [2; 32], amount: 5 },
				},
			))
		};
		let payload = encode_payload(&[mk(0), mk(1), mk(2)]);
		let full = refine(SVC, &payload);
		assert_eq!(records(&full).len(), 3);
		let one = records(&full)[..1].to_vec();
		let limited = refine_with_limit(SVC, &payload, encode_records(&one).len() + 3);
		assert_eq!(records(&limited), one);
	}

	#[test]
	fn garbage_payload_refines_to_nothing() {
		assert!(refine(SVC, &[]).is_empty());
		assert!(refine(SVC, &[9, 9, 9]).is_empty());
		assert!(refine(SVC, &[PAYLOAD_VERSION, 1, 200]).is_empty());
		// Unknown item kinds are skipped, known ones around them survive.
		let mut payload = vec![PAYLOAD_VERSION, 99, 2, 0, 0];
		payload.extend_from_slice(&encode_payload(&[Item::Plain(PlainOp::GovExecUpgrade)])[1..]);
		assert_eq!(records(&refine(SVC, &payload)), vec![Record::Plain(PlainOp::GovExecUpgrade)]);
	}
}
