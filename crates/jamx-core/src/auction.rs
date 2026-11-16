//! The batch auction.
//!
//! Every round clears at **one price**. All buyers pay it and all sellers receive it, so the
//! position of an order inside the batch is worth nothing: there is nothing to front-run.
//! The functions here are pure and deterministic; refine runs them to find the price, and
//! accumulate runs the allocation again over the orders that turn out to be funded.

use crate::types::Price;
use alloc::vec::Vec;
use core::cmp::Ordering;

/// One side's interest: at most `qty` base atoms at `price` or better.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Quote {
	pub price: Price,
	pub qty: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Clearing {
	pub price: Price,
	/// Base atoms that trade if every order is honoured in full.
	pub volume: u64,
}

/// Find the clearing price.
///
/// `bids` must be sorted best first (highest price first) and `asks` best first (lowest
/// price first). The rules, in order:
///
/// 1. maximize the traded volume;
/// 2. among those prices, minimize the quantity left unmatched on the heavier side;
/// 3. the prices that satisfy both form an interval, bounded by the limits of the last
///    orders that trade and the first that do not; take its midpoint, which splits the
///    surplus evenly between buyers and sellers.
///
/// Returns `None` when no bid meets any ask.
pub fn discover(bids: &[Quote], asks: &[Quote]) -> Option<Clearing> {
	if bids.is_empty() || asks.is_empty() {
		return None;
	}
	debug_assert!(bids.windows(2).all(|w| w[0].price.cmp_value(&w[1].price) != Ordering::Less));
	debug_assert!(asks.windows(2).all(|w| w[0].price.cmp_value(&w[1].price) != Ordering::Greater));

	// Every limit in the book, ascending, without duplicates. Demand and supply are step
	// functions that only change at these prices.
	// Both sides arrive sorted, so this is a merge.
	let mut cands: Vec<Price> = Vec::with_capacity(bids.len() + asks.len());
	let mut up = bids.iter().rev().map(|q| q.price).peekable();
	let mut ax = asks.iter().map(|q| q.price).peekable();
	loop {
		let next = match (up.peek(), ax.peek()) {
			(Some(b), Some(a)) if b.cmp_value(a) != Ordering::Greater => up.next(),
			(_, Some(_)) => ax.next(),
			(Some(_), None) => up.next(),
			(None, None) => break,
		};
		if let Some(p) = next {
			if cands.last().map_or(true, |l| !l.same_value(&p)) {
				cands.push(p);
			}
		}
	}

	// demand[k]: bids priced at or above cands[k]; supply[k]: asks priced at or below it.
	let n = cands.len();
	let total_bid: u128 = bids.iter().map(|q| q.qty as u128).sum();
	let mut demand = Vec::with_capacity(n);
	let mut supply = Vec::with_capacity(n);
	let (mut below, mut bi) = (0u128, bids.len());
	let (mut at_or_below, mut ai) = (0u128, 0);
	for c in &cands {
		while bi > 0 && bids[bi - 1].price.cmp_value(c) == Ordering::Less {
			below += bids[bi - 1].qty as u128;
			bi -= 1;
		}
		while ai < asks.len() && asks[ai].price.cmp_value(c) != Ordering::Greater {
			at_or_below += asks[ai].qty as u128;
			ai += 1;
		}
		demand.push(total_bid - below);
		supply.push(at_or_below);
	}

	// Walk the price axis as alternating cells: the point cands[k], then the open interval
	// (cands[k], cands[k+1]). Strictly between two limits, the bids that still qualify are
	// those at or above the upper one and the asks those at or below the lower one.
	let mut best: Option<(u128, u128)> = None; // (volume, imbalance)
	let (mut lo, mut hi) = (cands[0], cands[0]);
	for cell in 0..2 * n - 1 {
		let k = cell / 2;
		let (d, s, cell_lo, cell_hi) = if cell % 2 == 0 {
			(demand[k], supply[k], cands[k], cands[k])
		} else {
			(demand[k + 1], supply[k], cands[k], cands[k + 1])
		};
		let score = (d.min(s), d.abs_diff(s));
		match best {
			Some((v, imb)) if score.0 < v || (score.0 == v && score.1 > imb) => {},
			Some(b) if b == score => hi = cell_hi,
			_ => {
				best = Some(score);
				lo = cell_lo;
				hi = cell_hi;
			},
		}
	}

	let (volume, _) = best?;
	if volume == 0 {
		return None;
	}
	let price = if lo.same_value(&hi) { lo } else { midpoint(lo, hi) };
	Some(Clearing { price, volume: u64::try_from(volume).unwrap_or(u64::MAX) })
}

/// The price halfway between `lo` and `hi`, as a ratio that fits in 64 bits. Precision
/// beyond 63 bits is dropped and the result is clamped so it never leaves `[lo, hi]`.
pub fn midpoint(lo: Price, hi: Price) -> Price {
	let a = lo.num as u128 * hi.den as u128;
	let b = hi.num as u128 * lo.den as u128;
	let d = lo.den as u128 * hi.den as u128;
	// (a + b) / 2d, exactly when it fits; otherwise halve first (the lost bit is far below
	// the 64 bits kept).
	let (mut num, mut den) = match (a.checked_add(b), d.checked_mul(2)) {
		(Some(n), Some(d2)) => (n, d2),
		_ => ((a >> 1) + (b >> 1) + (a & b & 1), d),
	};
	let bits = 128 - num.max(den).leading_zeros();
	if bits > 64 {
		num >>= bits - 64;
		den >>= bits - 64;
	}
	let mid = Price::new(num as u64, den as u64);
	if !mid.valid() || mid.cmp_value(&lo) == Ordering::Less {
		lo
	} else if mid.cmp_value(&hi) == Ordering::Greater {
		hi
	} else {
		mid
	}
}

/// Share `volume` among `quotes`, which must be sorted best price first.
///
/// Better prices are filled completely before worse ones receive anything. The orders at
/// the last price level reached share what is left in proportion to their size; the few
/// atoms lost to rounding go one each to the first orders of that level.
pub fn allocate(quotes: &[Quote], volume: u64) -> Vec<u64> {
	let mut fills = alloc::vec![0u64; quotes.len()];
	let mut rem = volume as u128;
	let mut i = 0;
	while i < quotes.len() && rem > 0 {
		let mut j = i + 1;
		while j < quotes.len() && quotes[j].price.same_value(&quotes[i].price) {
			j += 1;
		}
		let level: u128 = quotes[i..j].iter().map(|q| q.qty as u128).sum();
		if level <= rem {
			for k in i..j {
				fills[k] = quotes[k].qty;
			}
			rem -= level;
		} else {
			let mut given: u128 = 0;
			// `rem < level <= sum of u64`, so when the level fits in 64 bits so does `rem`.
			let small = u64::try_from(level).ok().zip(u64::try_from(rem).ok());
			for k in i..j {
				let f = match small.and_then(|(l, r)| crate::types::mul_div_floor(r, quotes[k].qty, l)) {
					Some(f) => f as u128,
					None => rem * quotes[k].qty as u128 / level,
				};
				fills[k] = f as u64;
				given += f;
			}
			let mut left = rem - given;
			for k in i..j {
				if left == 0 {
					break;
				}
				if fills[k] < quotes[k].qty {
					fills[k] += 1;
					left -= 1;
				}
			}
			rem = 0;
		}
		i = j;
	}
	fills
}

/// Pair filled buyers with filled sellers, walking both lists in order. The result lists
/// `(bid index, ask index, quantity)`; quantities sum to the traded volume.
pub fn pair(bid_fills: &[u64], ask_fills: &[u64]) -> Vec<(usize, usize, u64)> {
	let mut out = Vec::new();
	let (mut i, mut j) = (0, 0);
	let (mut bi, mut aj) = (0u64, 0u64);
	loop {
		while i < bid_fills.len() && bi == 0 {
			bi = bid_fills[i];
			if bi == 0 {
				i += 1;
			}
		}
		while j < ask_fills.len() && aj == 0 {
			aj = ask_fills[j];
			if aj == 0 {
				j += 1;
			}
		}
		if i >= bid_fills.len() || j >= ask_fills.len() {
			return out;
		}
		let q = bi.min(aj);
		out.push((i, j, q));
		bi -= q;
		aj -= q;
		if bi == 0 {
			i += 1;
		}
		if aj == 0 {
			j += 1;
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use rand::{rngs::StdRng, Rng, SeedableRng};

	fn q(num: u64, den: u64, qty: u64) -> Quote {
		Quote { price: Price::new(num, den), qty }
	}

	fn sorted(mut bids: Vec<Quote>, mut asks: Vec<Quote>) -> (Vec<Quote>, Vec<Quote>) {
		bids.sort_by(|a, b| b.price.cmp_value(&a.price));
		asks.sort_by(|a, b| a.price.cmp_value(&b.price));
		(bids, asks)
	}

	/// Volume tradable at an arbitrary price, by definition.
	fn volume_at(bids: &[Quote], asks: &[Quote], p: &Price) -> u128 {
		let d: u128 = bids
			.iter()
			.filter(|b| b.price.cmp_value(p) != Ordering::Less)
			.map(|b| b.qty as u128)
			.sum();
		let s: u128 = asks
			.iter()
			.filter(|a| a.price.cmp_value(p) != Ordering::Greater)
			.map(|a| a.qty as u128)
			.sum();
		d.min(s)
	}

	#[test]
	fn no_cross_no_trade() {
		let (b, a) = sorted(vec![q(9, 1, 10)], vec![q(10, 1, 10)]);
		assert_eq!(discover(&b, &a), None);
		assert_eq!(discover(&[], &a), None);
		assert_eq!(discover(&b, &[]), None);
	}

	#[test]
	fn single_cross_splits_the_surplus() {
		// Buyer would pay 10, seller would take 8: they meet at 9.
		let (b, a) = sorted(vec![q(10, 1, 5)], vec![q(8, 1, 5)]);
		let c = discover(&b, &a).unwrap();
		assert_eq!(c.volume, 5);
		assert!(c.price.same_value(&Price::new(9, 1)));
	}

	#[test]
	fn picks_max_volume_then_min_imbalance() {
		// asks: 10@8, 10@9; bid: 10@10. At 9 and above, 10 would be left unsold.
		let (b, a) = sorted(vec![q(10, 1, 10)], vec![q(8, 1, 10), q(9, 1, 10)]);
		let c = discover(&b, &a).unwrap();
		assert_eq!(c.volume, 10);
		// Any price from 8 up to (not including) 9 trades 10 with nothing left over; the
		// seller who asked 9 stays out and bounds the price from above.
		assert!(c.price.same_value(&Price::new(17, 2)));

		// A deeper book: the price moves to where the most trades.
		let (b, a) =
			sorted(vec![q(12, 1, 5), q(11, 1, 5), q(10, 1, 5)], vec![q(9, 1, 4), q(10, 1, 4), q(11, 1, 4)]);
		let c = discover(&b, &a).unwrap();
		// at 10: D=15,S=8 -> 8 ; at 11: D=10,S=12 -> 10 ; at 12: D=5,S=12 -> 5
		assert_eq!(c.volume, 10);
		assert!(c.price.same_value(&Price::new(11, 1)));
	}

	#[test]
	fn discover_is_optimal_on_random_books() {
		let mut rng = StdRng::seed_from_u64(1);
		for _ in 0..2000 {
			let nb = rng.gen_range(0..8);
			let na = rng.gen_range(0..8);
			let mk = |rng: &mut StdRng| {
				q(rng.gen_range(1..30), rng.gen_range(1..6), rng.gen_range(1..50))
			};
			let (b, a) = sorted(
				(0..nb).map(|_| mk(&mut rng)).collect(),
				(0..na).map(|_| mk(&mut rng)).collect(),
			);
			let best = b
				.iter()
				.chain(a.iter())
				.map(|x| volume_at(&b, &a, &x.price))
				.max()
				.unwrap_or(0);
			match discover(&b, &a) {
				None => assert_eq!(best, 0),
				Some(c) => {
					assert_eq!(c.volume as u128, best);
					// The chosen price really trades that volume, even when it is a midpoint.
					assert_eq!(volume_at(&b, &a, &c.price), best);
				},
			}
		}
	}

	#[test]
	fn allocation_respects_price_priority_and_shares_the_margin() {
		let quotes = vec![q(12, 1, 10), q(11, 1, 10), q(11, 1, 30), q(10, 1, 50)];
		assert_eq!(allocate(&quotes, 100), vec![10, 10, 30, 50]);
		assert_eq!(allocate(&quotes, 500), vec![10, 10, 30, 50]);
		assert_eq!(allocate(&quotes, 10), vec![10, 0, 0, 0]);
		// 20 left for the 11-level of 40: 1/4 and 3/4.
		assert_eq!(allocate(&quotes, 30), vec![10, 5, 15, 0]);
		// Rounding: 7 over (10, 30) -> floor(1.75), floor(5.25) = 1, 5 and one atom left over.
		assert_eq!(allocate(&quotes, 17), vec![10, 2, 5, 0]);
		assert_eq!(allocate(&quotes, 0), vec![0, 0, 0, 0]);
		assert_eq!(allocate(&[], 5), Vec::<u64>::new());
	}

	#[test]
	fn allocation_invariants_on_random_input() {
		let mut rng = StdRng::seed_from_u64(2);
		for _ in 0..3000 {
			let n = rng.gen_range(1..12);
			let mut quotes: Vec<Quote> = (0..n)
				.map(|_| q(rng.gen_range(1..6), 1, rng.gen_range(0..u64::MAX / 16)))
				.collect();
			quotes.sort_by(|a, b| b.price.cmp_value(&a.price));
			let total: u128 = quotes.iter().map(|x| x.qty as u128).sum();
			let volume = rng.gen_range(0..u64::MAX);
			let fills = allocate(&quotes, volume);
			let sum: u128 = fills.iter().map(|f| *f as u128).sum();
			assert_eq!(sum, (volume as u128).min(total));
			for (f, x) in fills.iter().zip(&quotes) {
				assert!(f <= &x.qty);
			}
			// If an order got anything, every strictly better-priced order is full.
			for k in 0..n {
				if fills[k] > 0 {
					for m in 0..k {
						if !quotes[m].price.same_value(&quotes[k].price) {
							assert_eq!(fills[m], quotes[m].qty);
						}
					}
				}
			}
		}
	}

	#[test]
	fn pairing_conserves_quantity() {
		assert_eq!(pair(&[5, 0, 5], &[3, 7]), vec![(0, 0, 3), (0, 1, 2), (2, 1, 5)]);
		assert_eq!(pair(&[0, 0], &[1]), vec![]);
		let mut rng = StdRng::seed_from_u64(3);
		for _ in 0..500 {
			let nb = rng.gen_range(1..8);
			let na = rng.gen_range(1..8);
			let mut b: Vec<u64> = (0..nb).map(|_| rng.gen_range(0..100)).collect();
			let total: u64 = b.iter().sum();
			// Build asks with the same total.
			let mut a = vec![0u64; na];
			let mut left = total;
			for x in a.iter_mut().take(na - 1) {
				*x = rng.gen_range(0..=left);
				left -= *x;
			}
			a[na - 1] = left;
			let pairs = pair(&b, &a);
			for (i, j, qty) in &pairs {
				assert!(*qty > 0);
				b[*i] -= qty;
				a[*j] -= qty;
			}
			assert!(b.iter().all(|x| *x == 0) && a.iter().all(|x| *x == 0));
			assert!(pairs.len() <= nb + na);
		}
	}

	#[test]
	fn midpoint_stays_in_range_at_the_extremes() {
		let lo = Price::new(u64::MAX - 1, u64::MAX);
		let hi = Price::new(u64::MAX, u64::MAX - 1);
		let m = midpoint(lo, hi);
		assert!(m.valid());
		assert!(m.cmp_value(&lo) != Ordering::Less && m.cmp_value(&hi) != Ordering::Greater);
		let m = midpoint(Price::new(1, u64::MAX), Price::new(u64::MAX, 1));
		assert!(m.valid());
		assert!(midpoint(Price::new(8, 1), Price::new(10, 1)).same_value(&Price::new(9, 1)));
		assert!(midpoint(Price::new(1, 3), Price::new(1, 2)).same_value(&Price::new(5, 12)));
	}
}
