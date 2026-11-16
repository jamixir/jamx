//! Reading Bitcoin from the outside.
//!
//! JAM has no network connection, so everything it learns about Bitcoin is carried in by
//! somebody and checked here by arithmetic alone:
//!
//! * a **header** is accepted only with the proof-of-work its own difficulty field demands,
//!   and (in accumulate) only if that difficulty is the one consensus prescribes;
//! * a **receipt** ties a transaction to a header through a Merkle path.
//!
//! A carrier can withhold data but cannot invent a payment. This module is pure; the header
//! chain itself (fork choice, confirmations) lives in [`crate::accumulate`].

use crate::{
	codec::{Codec, DecodeError, DecodeResult, Reader, Writer},
	crypto::{sha256d, sha256d_pair, Hash32},
	types::BtcParams,
};
use alloc::vec::Vec;
use core::cmp::Ordering;

/// Blocks per difficulty period.
pub const RETARGET_INTERVAL: u32 = 2016;
/// Intended length of a difficulty period: two weeks, in seconds.
pub const TARGET_TIMESPAN: u32 = 14 * 24 * 60 * 60;

// ---------------------------------------------------------------------------------------
// 256-bit unsigned integers (just enough for targets and work)
// ---------------------------------------------------------------------------------------

/// Little-endian limbs: `0` is least significant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct U256(pub [u64; 4]);

impl U256 {
	pub const ZERO: U256 = U256([0; 4]);
	pub const MAX: U256 = U256([u64::MAX; 4]);

	pub fn from_u64(v: u64) -> Self {
		U256([v, 0, 0, 0])
	}

	/// A block hash read as a number (hashes are little-endian byte strings).
	pub fn from_le_bytes(b: &[u8; 32]) -> Self {
		let mut l = [0u64; 4];
		for (i, limb) in l.iter_mut().enumerate() {
			let mut w = [0u8; 8];
			w.copy_from_slice(&b[i * 8..i * 8 + 8]);
			*limb = u64::from_le_bytes(w);
		}
		U256(l)
	}

	pub fn is_zero(&self) -> bool {
		self.0 == [0; 4]
	}

	pub fn bits(&self) -> u32 {
		for i in (0..4).rev() {
			if self.0[i] != 0 {
				return 64 * i as u32 + 64 - self.0[i].leading_zeros();
			}
		}
		0
	}

	fn bit(&self, i: u32) -> bool {
		self.0[(i / 64) as usize] >> (i % 64) & 1 == 1
	}

	pub fn shl(&self, n: u32) -> U256 {
		if n >= 256 {
			return U256::ZERO;
		}
		let (words, bits) = ((n / 64) as usize, n % 64);
		let mut out = [0u64; 4];
		for i in words..4 {
			out[i] = self.0[i - words] << bits;
			if bits > 0 && i > words {
				out[i] |= self.0[i - words - 1] >> (64 - bits);
			}
		}
		U256(out)
	}

	pub fn shr(&self, n: u32) -> U256 {
		if n >= 256 {
			return U256::ZERO;
		}
		let (words, bits) = ((n / 64) as usize, n % 64);
		let mut out = [0u64; 4];
		for i in 0..4 - words {
			out[i] = self.0[i + words] >> bits;
			if bits > 0 && i + words + 1 < 4 {
				out[i] |= self.0[i + words + 1] << (64 - bits);
			}
		}
		U256(out)
	}

	pub fn overflowing_add(&self, o: &U256) -> (U256, bool) {
		let mut out = [0u64; 4];
		let mut carry = false;
		for i in 0..4 {
			let (a, c1) = self.0[i].overflowing_add(o.0[i]);
			let (b, c2) = a.overflowing_add(carry as u64);
			out[i] = b;
			carry = c1 || c2;
		}
		(U256(out), carry)
	}

	pub fn wrapping_sub(&self, o: &U256) -> U256 {
		let mut out = [0u64; 4];
		let mut borrow = false;
		for i in 0..4 {
			let (a, b1) = self.0[i].overflowing_sub(o.0[i]);
			let (b, b2) = a.overflowing_sub(borrow as u64);
			out[i] = b;
			borrow = b1 || b2;
		}
		U256(out)
	}

	pub fn not(&self) -> U256 {
		U256([!self.0[0], !self.0[1], !self.0[2], !self.0[3]])
	}

	/// `self * m`, or `None` on overflow.
	pub fn checked_mul_u32(&self, m: u32) -> Option<U256> {
		let mut out = [0u64; 4];
		let mut carry: u128 = 0;
		for i in 0..4 {
			let v = self.0[i] as u128 * m as u128 + carry;
			out[i] = v as u64;
			carry = v >> 64;
		}
		(carry == 0).then_some(U256(out))
	}

	pub fn div_u32(&self, d: u32) -> U256 {
		let mut out = [0u64; 4];
		let mut rem: u128 = 0;
		for i in (0..4).rev() {
			let cur = (rem << 64) | self.0[i] as u128;
			out[i] = (cur / d as u128) as u64;
			rem = cur % d as u128;
		}
		U256(out)
	}

	/// `self / d` by shift-and-subtract. `d` must not be zero.
	pub fn div(&self, d: &U256) -> U256 {
		let mut q = U256::ZERO;
		let mut r = U256::ZERO;
		for i in (0..self.bits()).rev() {
			r = r.shl(1);
			if self.bit(i) {
				r.0[0] |= 1;
			}
			if r.cmp(d) != Ordering::Less {
				r = r.wrapping_sub(d);
				q.0[(i / 64) as usize] |= 1 << (i % 64);
			}
		}
		q
	}

	pub fn low_u128(&self) -> u128 {
		self.0[0] as u128 | (self.0[1] as u128) << 64
	}

	pub fn fits_u128(&self) -> bool {
		self.0[2] == 0 && self.0[3] == 0
	}
}

impl PartialOrd for U256 {
	fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
		Some(self.cmp(other))
	}
}

impl Ord for U256 {
	fn cmp(&self, other: &Self) -> Ordering {
		for i in (0..4).rev() {
			match self.0[i].cmp(&other.0[i]) {
				Ordering::Equal => continue,
				o => return o,
			}
		}
		Ordering::Equal
	}
}

/// Expand Bitcoin's compact difficulty encoding (`nBits`). `None` for zero, negative or
/// overflowing values, all of which consensus rejects.
pub fn target_from_compact(bits: u32) -> Option<U256> {
	let size = bits >> 24;
	let word = bits & 0x007f_ffff;
	if word == 0 || bits & 0x0080_0000 != 0 {
		return None;
	}
	if size > 34 || (word > 0xff && size > 33) || (word > 0xffff && size > 32) {
		return None;
	}
	let t = if size <= 3 {
		U256::from_u64((word >> (8 * (3 - size))) as u64)
	} else {
		U256::from_u64(word as u64).shl(8 * (size - 3))
	};
	(!t.is_zero()).then_some(t)
}

/// The inverse of [`target_from_compact`], with Bitcoin's rounding.
pub fn compact_from_target(t: &U256) -> u32 {
	let mut size = t.bits().div_ceil(8);
	let mut compact: u32 = if size <= 3 {
		(t.0[0] << (8 * (3 - size))) as u32
	} else {
		t.shr(8 * (size - 3)).0[0] as u32
	};
	if compact & 0x0080_0000 != 0 {
		compact >>= 8;
		size += 1;
	}
	compact | size << 24
}

/// Expected number of hashes to find a block at this target: `2^256 / (target + 1)`.
/// Saturates at `u128::MAX`, far beyond any real difficulty.
pub fn work_from_target(t: &U256) -> u128 {
	let (t1, overflow) = t.overflowing_add(&U256::from_u64(1));
	if overflow {
		return 1;
	}
	// 2^256 / (t+1) == (~t / (t+1)) + 1
	let (w, o) = t.not().div(&t1).overflowing_add(&U256::from_u64(1));
	if o || !w.fits_u128() {
		u128::MAX
	} else {
		w.low_u128()
	}
}

pub fn work_from_compact(bits: u32) -> Option<u128> {
	target_from_compact(bits).map(|t| work_from_target(&t))
}

// ---------------------------------------------------------------------------------------
// Headers
// ---------------------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Header {
	pub version: u32,
	pub prev: Hash32,
	pub merkle_root: Hash32,
	pub time: u32,
	pub bits: u32,
	pub nonce: u32,
}

impl Header {
	pub fn parse(raw: &[u8; 80]) -> Self {
		let mut r = Reader::new(raw);
		// An 80-byte slice always has exactly these fields.
		Header {
			version: r.u32().unwrap_or(0),
			prev: r.array().unwrap_or([0; 32]),
			merkle_root: r.array().unwrap_or([0; 32]),
			time: r.u32().unwrap_or(0),
			bits: r.u32().unwrap_or(0),
			nonce: r.u32().unwrap_or(0),
		}
	}

	pub fn serialize(&self) -> [u8; 80] {
		let mut v = Vec::with_capacity(80);
		v.put_u32(self.version);
		v.put(&self.prev);
		v.put(&self.merkle_root);
		v.put_u32(self.time);
		v.put_u32(self.bits);
		v.put_u32(self.nonce);
		let mut out = [0u8; 80];
		out.copy_from_slice(&v);
		out
	}

	pub fn hash(&self) -> Hash32 {
		sha256d(&self.serialize())
	}
}

pub fn header_hash(raw: &[u8; 80]) -> Hash32 {
	sha256d(raw)
}

/// Does `hash` carry the proof-of-work that `bits` claims, within the chain's limit?
pub fn check_pow(hash: &Hash32, bits: u32, params: &BtcParams) -> bool {
	let (Some(target), Some(limit)) =
		(target_from_compact(bits), target_from_compact(params.pow_limit_bits))
	else {
		return false;
	};
	target <= limit && U256::from_le_bytes(hash) <= target
}

/// Same check when only the difficulty field is known to be self-consistent (refine does
/// not know which chain an asset tracks; accumulate enforces the chain's limit).
pub fn check_pow_self(hash: &Hash32, bits: u32) -> bool {
	target_from_compact(bits).is_some_and(|t| U256::from_le_bytes(hash) <= t)
}

/// The difficulty the block after `parent` must declare.
///
/// `parent_period_start` is the timestamp of the first block of the parent's period.
pub fn next_bits(
	params: &BtcParams,
	parent_height: u32,
	parent_bits: u32,
	parent_time: u32,
	parent_period_start: u32,
) -> u32 {
	if !params.retarget || (parent_height + 1) % RETARGET_INTERVAL != 0 {
		return parent_bits;
	}
	let actual = (parent_time as i64 - parent_period_start as i64)
		.clamp((TARGET_TIMESPAN / 4) as i64, (TARGET_TIMESPAN * 4) as i64) as u32;
	let limit = target_from_compact(params.pow_limit_bits).unwrap_or(U256::MAX);
	let Some(old) = target_from_compact(parent_bits) else { return parent_bits };
	let new = match old.checked_mul_u32(actual) {
		Some(x) => x.div_u32(TARGET_TIMESPAN),
		None => limit,
	};
	compact_from_target(if new > limit { &limit } else { &new })
}

// ---------------------------------------------------------------------------------------
// Merkle receipts
// ---------------------------------------------------------------------------------------

/// Walk a Merkle path from a transaction to the root.
///
/// Returns `None` if `index` has bits beyond the length of the path, which would let one
/// path stand for several positions.
pub fn merkle_root(txid: &Hash32, mut index: u32, branch: &[Hash32]) -> Option<Hash32> {
	let mut h = *txid;
	for sib in branch {
		h = if index & 1 == 1 { sha256d_pair(sib, &h) } else { sha256d_pair(&h, sib) };
		index >>= 1;
	}
	(index == 0).then_some(h)
}

/// Build the Merkle root and the path for one leaf (tools and tests).
pub fn merkle_branch(txids: &[Hash32], index: usize) -> (Hash32, Vec<Hash32>) {
	let mut level: Vec<Hash32> = txids.to_vec();
	let mut branch = Vec::new();
	let mut i = index;
	while level.len() > 1 {
		if level.len() % 2 == 1 {
			level.push(level[level.len() - 1]);
		}
		branch.push(level[i ^ 1]);
		level = level.chunks(2).map(|p| sha256d_pair(&p[0], &p[1])).collect();
		i /= 2;
	}
	(level.first().copied().unwrap_or([0; 32]), branch)
}

// ---------------------------------------------------------------------------------------
// Transactions
// ---------------------------------------------------------------------------------------

fn compact_size(r: &mut Reader) -> DecodeResult<u64> {
	Ok(match r.u8()? {
		0xfd => r.u16()? as u64,
		0xfe => r.u32()? as u64,
		0xff => r.u64()?,
		n => n as u64,
	})
}

fn put_compact_size(out: &mut Vec<u8>, n: u64) {
	match n {
		0..=0xfc => out.put_u8(n as u8),
		0xfd..=0xffff => {
			out.put_u8(0xfd);
			out.put_u16(n as u16);
		},
		0x1_0000..=0xffff_ffff => {
			out.put_u8(0xfe);
			out.put_u32(n as u32);
		},
		_ => {
			out.put_u8(0xff);
			out.put_u64(n);
		},
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TxOut<'a> {
	pub value: u64,
	pub script: &'a [u8],
}

/// Parse the outputs of a transaction given in its non-witness serialization.
///
/// A 64-byte "transaction" is refused: an inner Merkle node is exactly 64 bytes, and
/// accepting one as a leaf is the classic way to forge a receipt.
pub fn parse_outputs(tx: &[u8]) -> DecodeResult<Vec<TxOut<'_>>> {
	if tx.len() == 64 {
		return Err(DecodeError);
	}
	let mut r = Reader::new(tx);
	r.u32()?; // version
	let n_in = compact_size(&mut r)?;
	if n_in == 0 {
		// Either the segwit marker (the caller must strip witness data first) or nonsense.
		return Err(DecodeError);
	}
	for _ in 0..n_in {
		r.bytes(36)?;
		let len = compact_size(&mut r)? as usize;
		r.bytes(len)?;
		r.u32()?;
	}
	let n_out = compact_size(&mut r)?;
	if n_out as usize > r.remaining() {
		return Err(DecodeError);
	}
	let mut outs = Vec::with_capacity(n_out as usize);
	for _ in 0..n_out {
		let value = r.u64()?;
		let len = compact_size(&mut r)? as usize;
		outs.push(TxOut { value, script: r.bytes(len)? });
	}
	r.u32()?; // lock time
	r.finish()?;
	Ok(outs)
}

/// The script of an `OP_RETURN` output carrying a 32-byte tag.
pub fn tag_script(tag: &Hash32) -> Vec<u8> {
	let mut s = alloc::vec![0x6a, 0x20];
	s.extend_from_slice(tag);
	s
}

/// Does this script have the shape of a deal tag (whichever deal it names)?
pub fn is_tag_script(script: &[u8]) -> bool {
	script.len() == 34 && script[0] == 0x6a && script[1] == 0x20
}

/// Is this one of the output scripts every wallet can pay to (P2PKH, P2SH, P2WPKH, P2WSH,
/// P2TR)? A buyer who named anything else as the destination could be asking for a payment
/// that no seller is able to make.
pub fn is_standard_script(s: &[u8]) -> bool {
	match s.len() {
		25 => s[..3] == [0x76, 0xa9, 0x14] && s[23..] == [0x88, 0xac],
		23 => s[..2] == [0xa9, 0x14] && s[22] == 0x87,
		22 => s[..2] == [0x00, 0x14],
		34 => s[..2] == [0x00, 0x20] || s[..2] == [0x51, 0x20],
		_ => false,
	}
}

/// Is this the coinbase of a block: a transaction whose only input spends nothing?
///
/// The check is on the transaction's first bytes. An inner node of the Merkle tree (two
/// hashes side by side) cannot pass it: it would need a hash with 27 chosen bytes.
pub fn is_coinbase(tx: &[u8]) -> bool {
	const NULL_PREVOUT: [u8; 36] = {
		let mut p = [0u8; 36];
		p[32] = 0xff;
		p[33] = 0xff;
		p[34] = 0xff;
		p[35] = 0xff;
		p
	};
	tx.len() > 41 && tx[4] == 1 && tx[5..41] == NULL_PREVOUT
}

/// Remove witness data from a transaction, giving the form its txid is computed from.
/// A transaction already in that form is returned unchanged.
pub fn strip_witness(tx: &[u8]) -> DecodeResult<Vec<u8>> {
	let mut r = Reader::new(tx);
	let version = r.u32()?;
	if r.peek_rest().first() != Some(&0) {
		return Ok(tx.to_vec());
	}
	r.u8()?;
	if r.u8()? != 1 {
		return Err(DecodeError);
	}
	let mut out = Vec::with_capacity(tx.len());
	out.put_u32(version);
	let n_in = compact_size(&mut r)?;
	put_compact_size(&mut out, n_in);
	for _ in 0..n_in {
		out.put(r.bytes(36)?);
		let len = compact_size(&mut r)?;
		put_compact_size(&mut out, len);
		out.put(r.bytes(len as usize)?);
		out.put(r.bytes(4)?);
	}
	let n_out = compact_size(&mut r)?;
	put_compact_size(&mut out, n_out);
	for _ in 0..n_out {
		out.put(r.bytes(8)?);
		let len = compact_size(&mut r)?;
		put_compact_size(&mut out, len);
		out.put(r.bytes(len as usize)?);
	}
	for _ in 0..n_in {
		let items = compact_size(&mut r)?;
		for _ in 0..items {
			let len = compact_size(&mut r)? as usize;
			r.bytes(len)?;
		}
	}
	out.put(r.bytes(4)?);
	r.finish()?;
	Ok(out)
}

/// Build a minimal transaction paying `outputs` (tools and tests; it spends a dummy input
/// and would not be valid on a real network).
pub fn build_tx(marker: u32, outputs: &[(u64, Vec<u8>)]) -> Vec<u8> {
	let mut tx = Vec::new();
	tx.put_u32(2);
	put_compact_size(&mut tx, 1);
	tx.put(&[0u8; 32]);
	tx.put_u32(marker);
	put_compact_size(&mut tx, 0);
	tx.put_u32(0xffff_ffff);
	put_compact_size(&mut tx, outputs.len() as u64);
	for (value, script) in outputs {
		tx.put_u64(*value);
		put_compact_size(&mut tx, script.len() as u64);
		tx.put(script);
	}
	tx.put_u32(0);
	tx
}

/// Build a coinbase-shaped transaction (tools and tests).
pub fn build_coinbase(marker: u32, outputs: &[(u64, Vec<u8>)]) -> Vec<u8> {
	let mut tx = Vec::new();
	tx.put_u32(2);
	put_compact_size(&mut tx, 1);
	tx.put(&[0u8; 32]);
	tx.put_u32(0xffff_ffff);
	put_compact_size(&mut tx, 4);
	tx.put_u32(marker);
	tx.put_u32(0xffff_ffff);
	put_compact_size(&mut tx, outputs.len() as u64);
	for (value, script) in outputs {
		tx.put_u64(*value);
		put_compact_size(&mut tx, script.len() as u64);
		tx.put(script);
	}
	tx.put_u32(0);
	tx
}

/// Grind a nonce until the header meets its own target. Only practical at regtest
/// difficulty, which is what local demonstrations use.
pub fn mine(prev: Hash32, merkle_root: Hash32, time: u32, bits: u32) -> Header {
	let mut h = Header { version: 0x2000_0000, prev, merkle_root, time, bits, nonce: 0 };
	while !check_pow_self(&h.hash(), bits) {
		h.nonce = h.nonce.wrapping_add(1);
	}
	h
}

// ---------------------------------------------------------------------------------------
// Stored forms
// ---------------------------------------------------------------------------------------

/// What the service keeps about every header it has accepted.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeaderRec {
	pub height: u32,
	pub prev: Hash32,
	pub merkle_root: Hash32,
	pub time: u32,
	pub bits: u32,
	/// Total work from the checkpoint to this block.
	pub chainwork: u128,
	pub period_start: u32,
}

impl Codec for HeaderRec {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_u32(self.height);
		out.put(&self.prev);
		out.put(&self.merkle_root);
		out.put_u32(self.time);
		out.put_u32(self.bits);
		out.put_u128(self.chainwork);
		out.put_u32(self.period_start);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			height: r.u32()?,
			prev: r.array()?,
			merkle_root: r.array()?,
			time: r.u32()?,
			bits: r.u32()?,
			chainwork: r.u128()?,
			period_start: r.u32()?,
		})
	}
}

/// The light client's view of one chain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct LightClient {
	pub params: BtcParams,
	pub tip: Hash32,
	pub tip_height: u32,
	pub tip_work: u128,
	/// Height of the checkpoint the client started from.
	pub base_height: u32,
}

impl Codec for LightClient {
	fn enc(&self, out: &mut Vec<u8>) {
		self.params.enc(out);
		out.put(&self.tip);
		out.put_u32(self.tip_height);
		out.put_u128(self.tip_work);
		out.put_u32(self.base_height);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			params: BtcParams::dec(r)?,
			tip: r.array()?,
			tip_height: r.u32()?,
			tip_work: r.u128()?,
			base_height: r.u32()?,
		})
	}
}

pub const MAINNET: BtcParams = BtcParams { pow_limit_bits: 0x1d00_ffff, retarget: true };
pub const REGTEST: BtcParams = BtcParams { pow_limit_bits: 0x207f_ffff, retarget: false };

/// Blocks whose timestamps make up a median time past: a block and the ten before it.
pub const MTP_SPAN: usize = 11;

/// The median of up to [`MTP_SPAN`] block timestamps (Bitcoin's rule: sort, take the middle).
pub fn median_time(times: &mut [u32]) -> u32 {
	times.sort_unstable();
	times.get(times.len() / 2).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;
	use alloc::{string::String, vec};
	use std::collections::BTreeMap;

	fn fixtures() -> BTreeMap<String, String> {
		include_str!("../tests/fixtures/btc_mainnet.txt")
			.lines()
			.filter_map(|l| l.split_once('='))
			.map(|(k, v)| (k.into(), v.into()))
			.collect()
	}

	fn h80(s: &str) -> [u8; 80] {
		hex::decode(s).unwrap().try_into().unwrap()
	}

	/// Explorers print hashes byte-reversed.
	fn rev32(s: &str) -> Hash32 {
		let mut b: [u8; 32] = hex::decode(s).unwrap().try_into().unwrap();
		b.reverse();
		b
	}

	#[test]
	fn u256_arithmetic() {
		let a = U256([5, 0, 0, 1]);
		assert_eq!(a.shl(64).0, [0, 5, 0, 0]);
		assert_eq!(a.shr(64).0, [0, 0, 1, 0]);
		assert_eq!(a.shl(1).0, [10, 0, 0, 2]);
		assert_eq!(a.shr(1).0, [2, 0, 1 << 63, 0]);
		assert_eq!(a.shl(256), U256::ZERO);
		assert_eq!(U256::from_u64(100).div(&U256::from_u64(7)), U256::from_u64(14));
		assert_eq!(U256::MAX.div(&U256::MAX), U256::from_u64(1));
		assert_eq!(U256::MAX.div_u32(1), U256::MAX);
		assert_eq!(U256::from_u64(u64::MAX).checked_mul_u32(2).unwrap().0, [u64::MAX - 1, 1, 0, 0]);
		assert!(U256::MAX.checked_mul_u32(2).is_none());
		let (s, o) = U256::MAX.overflowing_add(&U256::from_u64(1));
		assert!(o && s.is_zero());
		assert_eq!(U256::from_u64(5).wrapping_sub(&U256::from_u64(3)), U256::from_u64(2));
		assert!(U256([0, 0, 0, 1]) > U256([u64::MAX, u64::MAX, u64::MAX, 0]));
		assert_eq!(U256([0, 0, 0, 1]).bits(), 193);
	}

	#[test]
	fn compact_encoding_matches_bitcoin() {
		// Vectors from Bitcoin Core's arith_uint256 tests.
		assert_eq!(target_from_compact(0), None);
		assert_eq!(target_from_compact(0x0080_0000), None);
		assert_eq!(target_from_compact(0x0180_0000), None);
		assert_eq!(target_from_compact(0x0492_3456), None); // negative
		assert_eq!(target_from_compact(0xff12_3456), None); // overflow
		assert_eq!(target_from_compact(0x0112_3456).unwrap(), U256::from_u64(0x12));
		assert_eq!(compact_from_target(&U256::from_u64(0x12)), 0x0112_0000);
		assert_eq!(target_from_compact(0x0212_3456).unwrap(), U256::from_u64(0x1234));
		assert_eq!(compact_from_target(&U256::from_u64(0x1234)), 0x0212_3400);
		assert_eq!(target_from_compact(0x0312_3456).unwrap(), U256::from_u64(0x12_3456));
		assert_eq!(compact_from_target(&U256::from_u64(0x12_3456)), 0x0312_3456);
		assert_eq!(target_from_compact(0x0412_3456).unwrap(), U256::from_u64(0x1234_5600));
		assert_eq!(compact_from_target(&U256::from_u64(0x1234_5600)), 0x0412_3456);
		assert_eq!(compact_from_target(&U256::from_u64(0x80)), 0x0200_8000);
		assert_eq!(target_from_compact(0x2012_3456).unwrap(), U256::from_u64(0x12_3456).shl(8 * 29));
		for bits in [0x1d00_ffffu32, 0x1703_4219, 0x207f_ffff, 0x1b04_04cb] {
			assert_eq!(compact_from_target(&target_from_compact(bits).unwrap()), bits);
		}
	}

	#[test]
	fn work_matches_known_values() {
		// Difficulty 1: 2^256 / (0xffff * 2^208 + 1) = 0x100010001
		assert_eq!(work_from_compact(0x1d00_ffff), Some(0x1_0001_0001));
		// Regtest: target ~ 2^255, work 2.
		assert_eq!(work_from_compact(0x207f_ffff), Some(2));
		assert_eq!(work_from_target(&U256::MAX), 1);
		assert_eq!(work_from_target(&U256::from_u64(1)), u128::MAX);
	}

	#[test]
	fn genesis_header() {
		let raw = h80("0100000000000000000000000000000000000000000000000000000000000000000000003ba3edfd7a7b12b27ac72c3e67768f617fc81bc3888a51323a9fb8aa4b1e5e4a29ab5f49ffff001d1dac2b7c");
		let h = Header::parse(&raw);
		assert_eq!(h.serialize(), raw);
		assert_eq!(h.time, 1231006505);
		assert_eq!(h.bits, 0x1d00ffff);
		assert_eq!(
			header_hash(&raw),
			rev32("000000000019d6689c085ae165831e934ff763ae46a2a6c172b3f1b60a8ce26f")
		);
		assert!(check_pow(&header_hash(&raw), h.bits, &MAINNET));
		// One flipped nonce bit ruins the proof-of-work.
		let mut bad = raw;
		bad[76] ^= 1;
		assert!(!check_pow(&header_hash(&bad), h.bits, &MAINNET));
		// Claiming an easier target than the chain allows is refused.
		assert!(!check_pow(&[0u8; 32], 0x207f_ffff, &MAINNET));
		assert!(check_pow(&[0u8; 32], 0x207f_ffff, &REGTEST));
	}

	#[test]
	fn mainnet_headers_link_and_retarget() {
		let fx = fixtures();
		let retarget: u32 = fx["retarget_height"].parse().unwrap();
		let first: u32 = fx["run_first_height"].parse().unwrap();
		let period_start = Header::parse(&h80(&fx["period_start_header"]));
		assert_eq!(
			header_hash(&h80(&fx["period_start_header"])),
			rev32(&fx["period_start_hash"])
		);
		let run: Vec<[u8; 80]> = fx["run_headers"].split(',').map(h80).collect();
		assert_eq!(header_hash(&run[0]), rev32(&fx["run_first_hash"]));

		let mut prev = Header::parse(&run[0]);
		let mut prev_hash = header_hash(&run[0]);
		let mut pstart = period_start.time;
		let mut changed = false;
		for (i, raw) in run.iter().enumerate().skip(1) {
			let height = first + i as u32;
			let h = Header::parse(raw);
			assert_eq!(h.prev, prev_hash, "linkage at {height}");
			assert!(check_pow(&header_hash(raw), h.bits, &MAINNET));
			let expect = next_bits(&MAINNET, height - 1, prev.bits, prev.time, pstart);
			assert_eq!(h.bits, expect, "difficulty at {height}");
			if height == retarget {
				assert_ne!(h.bits, prev.bits);
				pstart = h.time;
				changed = true;
			}
			prev_hash = header_hash(raw);
			prev = h;
		}
		assert!(changed);
	}

	#[test]
	fn mainnet_receipts_verify() {
		let fx = fixtures();
		let run: Vec<[u8; 80]> = fx["run_headers"].split(',').map(h80).collect();
		let first: u32 = fx["run_first_height"].parse().unwrap();
		let height: u32 = fx["proof_block_height"].parse().unwrap();
		let header = Header::parse(&run[(height - first) as usize]);
		assert_eq!(header.hash(), rev32(&fx["proof_block_hash"]));

		for label in ["a", "b", "c"] {
			let txid = rev32(&fx[&format!("proof_{label}_txid")]);
			let pos: u32 = fx[&format!("proof_{label}_pos")].parse().unwrap();
			let branch: Vec<Hash32> = fx[&format!("proof_{label}_merkle")].split(',').map(rev32).collect();
			let raw = hex::decode(&fx[&format!("proof_{label}_tx_hex")]).unwrap();
			let stripped = strip_witness(&raw).unwrap();
			assert_eq!(sha256d(&stripped), txid, "txid of {label}");
			assert_eq!(strip_witness(&stripped).unwrap(), stripped);
			assert_eq!(merkle_root(&txid, pos, &branch), Some(header.merkle_root));
			// Wrong position, wrong sibling, or an index that overruns the path all fail.
			assert_ne!(merkle_root(&txid, pos ^ 1, &branch), Some(header.merkle_root));
			let mut bad = branch.clone();
			bad[0][0] ^= 1;
			assert_ne!(merkle_root(&txid, pos, &bad), Some(header.merkle_root));
			assert_eq!(merkle_root(&txid, pos | 1 << 20, &branch), None);
			let outs = parse_outputs(&stripped).unwrap();
			assert!(!outs.is_empty());
			// The witness serialization is not what receipts are about.
			if raw != stripped {
				assert!(parse_outputs(&raw).is_err());
			}
		}
	}

	#[test]
	fn local_merkle_tree_and_transactions() {
		let tag = [7u8; 32];
		let script = vec![0x00, 0x14, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];
		let tx = build_tx(1, &[(50_000, script.clone()), (0, tag_script(&tag))]);
		let outs = parse_outputs(&tx).unwrap();
		assert_eq!(outs.len(), 2);
		assert_eq!(outs[0], TxOut { value: 50_000, script: &script });
		assert_eq!(outs[1].script, &tag_script(&tag)[..]);
		assert!(parse_outputs(&tx[..tx.len() - 1]).is_err());
		assert!(parse_outputs(&[0u8; 64]).is_err());

		for n in 1..9usize {
			let txids: Vec<Hash32> = (0..n).map(|i| sha256d(&[i as u8])).collect();
			for i in 0..n {
				let (root, branch) = merkle_branch(&txids, i);
				assert_eq!(merkle_root(&txids[i], i as u32, &branch), Some(root), "n={n} i={i}");
			}
		}
	}

	#[test]
	fn regtest_mining() {
		let h = mine([1; 32], [2; 32], 1_700_000_000, REGTEST.pow_limit_bits);
		assert!(check_pow(&h.hash(), h.bits, &REGTEST));
		assert!(!check_pow(&h.hash(), h.bits, &MAINNET));
		assert_eq!(next_bits(&REGTEST, 2015, h.bits, 5, 1), h.bits);
	}

	#[test]
	fn retarget_is_clamped() {
		let bits = 0x1b04_04cb;
		let t = target_from_compact(bits).unwrap();
		// Blocks came 100x too fast: difficulty rises by at most 4x.
		let fast = next_bits(&MAINNET, 2015, bits, 1_000_000 + TARGET_TIMESPAN / 100, 1_000_000);
		assert_eq!(target_from_compact(fast).unwrap(), target_from_compact(compact_from_target(&t.div_u32(4))).unwrap());
		// Blocks came 100x too slow: difficulty falls by at most 4x.
		let slow = next_bits(&MAINNET, 2015, bits, 1_000_000 + TARGET_TIMESPAN * 100, 1_000_000);
		assert_eq!(slow, compact_from_target(&t.checked_mul_u32(4).unwrap()));
		// A timestamp before the period start counts as the minimum.
		assert_eq!(next_bits(&MAINNET, 2015, bits, 5, 1_000_000), fast);
		// Never easier than the limit.
		assert_eq!(next_bits(&MAINNET, 2015, 0x1d00_ffff, 1_000_000 + TARGET_TIMESPAN * 4, 1_000_000), 0x1d00_ffff);
		// Off the boundary nothing changes.
		assert_eq!(next_bits(&MAINNET, 2014, bits, 9, 1), bits);
	}
}
