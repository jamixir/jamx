//! Wire formats.
//!
//! * A **payload** is what a bundler hands to JAM as the work-item payload: a list of
//!   [`Item`]s (signed operations, order batches, Bitcoin headers and receipts).
//! * Refine checks everything that can be checked without state and turns the payload into
//!   a compact list of [`Record`]s, the work output.
//! * Accumulate applies the records to state.
//!
//! Signatures never reach accumulate: refine verifies them and the record carries only the
//! key that signed.

use crate::{
	codec::{dec_vec, enc_vec, Codec, DecodeError, DecodeResult, Reader, Writer},
	crypto::{self, Hash32, PubKey, Sig},
	types::*,
};
use alloc::vec::Vec;

/// Domain separator for every signature this service accepts.
pub const DOMAIN: &[u8] = b"jamx/v1";
const MSG_OP: u8 = 1;
const MSG_ORDER: u8 = 2;

pub const PAYLOAD_VERSION: u8 = 1;
pub const OUTPUT_VERSION: u8 = 1;

/// Largest fee a batch may charge, in basis points.
pub const MAX_FEE_BPS: u16 = 100;
pub const MAX_BATCH_ORDERS: usize = 4096;
/// Most headers in one item. Their digests must fit in one work output (72 bytes each), and
/// accumulate takes only as many as the item's gas pays for.
pub const MAX_HEADERS_PER_ITEM: usize = 512;
pub const MAX_BRANCH: usize = 32;
pub const MAX_TX_LEN: usize = 100_000;
pub const MAX_CANCEL: usize = 64;

/// The bytes a key signs to authorize an operation. Binding the service identifier stops a
/// signature from being replayed against another instance of this code.
pub fn op_message(service: u32, body: &[u8]) -> Vec<u8> {
	let mut m = Vec::with_capacity(DOMAIN.len() + 5 + body.len());
	m.put(DOMAIN);
	m.put_u32(service);
	m.put_u8(MSG_OP);
	m.put(body);
	m
}

/// The bytes a key signs to place an order in the market `base`/`quote`.
pub fn order_message(service: u32, base: AssetId, quote: AssetId, order: &[u8]) -> Vec<u8> {
	let mut m = Vec::with_capacity(DOMAIN.len() + 13 + order.len());
	m.put(DOMAIN);
	m.put_u32(service);
	m.put_u8(MSG_ORDER);
	m.put_u32(base);
	m.put_u32(quote);
	m.put(order);
	m
}

/// The tag an off-chain payment must carry to be tied to one deal.
pub fn deal_tag(service: u32, deal: DealId) -> Hash32 {
	crypto::blake2b(&[b"jamx/deal", &service.to_le_bytes(), &deal.to_le_bytes()])
}

// ---------------------------------------------------------------------------------------
// Operations
// ---------------------------------------------------------------------------------------

/// What an issuer may do to a holder of its own asset, and nothing else.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum IssuerAction {
	Allow = 0,
	Disallow = 1,
	Freeze = 2,
	Unfreeze = 3,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AssetKindCfg {
	Token,
	External {
		cfg: ExternalCfg,
		/// Chain parameters and starting header; required for the Bitcoin rail.
		btc: Option<(BtcParams, BtcCheckpoint)>,
	},
}

/// An operation authorized by a signature.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Op {
	Transfer { asset: AssetId, to: AccountId, amount: u64 },
	/// Spend the given nonces without doing anything: cancels the orders that carry them.
	Cancel { nonces: Vec<u64> },
	/// Spend every nonce below `floor`: cancels everything older in one go.
	BumpNonce { floor: u64 },
	AssetCreate {
		symbol: [u8; 8],
		decimals: u8,
		flags: u8,
		kind: AssetKindCfg,
		attestors: Vec<AccountId>,
		threshold: u8,
	},
	Mint { asset: AssetId, to: AccountId, amount: u64 },
	Burn { asset: AssetId, amount: u64 },
	IssuerSet { asset: AssetId, target: AccountId, action: IssuerAction, reason: Hash32 },
	/// An attestor states the reserves it has verified behind `asset`.
	Attest { asset: AssetId, reserves: u64 },
	Policy(PolicyChange),
	PolicyCancel { index: u8 },
	Freeze { target: AccountId },
	Unfreeze { target: AccountId },
	RecoveryApprove { target: AccountId, new_owner: PubKey },
	RecoveryCancel,
	/// The paying side of a deal lets the locked funds go without waiting for a proof.
	DealRelease { deal: DealId },
	OracleAttest { deal: DealId, verdict: Verdict },
	GovProposeUpgrade { code_hash: Hash32, code_len: u32, min_item_gas: u64, min_memo_gas: u64 },
	GovCancelUpgrade,
	GovSetAdmin { admin: Option<AccountId> },
	/// Start (`on`) or stop accepting an asset. An account holds only assets it has accepted:
	/// nobody else can add an entry to its record. Signing an order that buys an asset
	/// accepts it too.
	Accept { asset: AssetId, on: bool },
}

/// What an oracle says about a deal's outside payment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Verdict {
	NotPaid = 0,
	Paid = 1,
	/// The destination the buyer named cannot be paid. Nobody is penalised.
	Void = 2,
}

/// A signed operation as the signer sees it: who acts, a replay-protection nonce, until
/// when the signature is good, and what to do.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Envelope {
	pub account: AccountId,
	pub nonce: u64,
	/// Last slot at which the operation may be applied. A signature that was never used
	/// must not stay usable for ever.
	pub expiry: Slot,
	pub op: Op,
}

/// Operations anybody may trigger; they only do what state already allows.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PlainOp {
	DealTimeout { deal: DealId },
	PolicyExec { account: AccountId, index: u8 },
	RecoveryExec { account: AccountId },
	GovExecUpgrade,
}

impl Codec for AssetKindCfg {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			AssetKindCfg::Token => out.put_u8(0),
			AssetKindCfg::External { cfg, btc } => {
				out.put_u8(1);
				cfg.enc(out);
				match btc {
					None => out.put_u8(0),
					Some((p, c)) => {
						out.put_u8(1);
						p.enc(out);
						c.enc(out);
					},
				}
			},
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			0 => AssetKindCfg::Token,
			1 => AssetKindCfg::External {
				cfg: ExternalCfg::dec(r)?,
				btc: if r.bool()? { Some((BtcParams::dec(r)?, BtcCheckpoint::dec(r)?)) } else { None },
			},
			_ => return Err(DecodeError),
		})
	}
}

impl Codec for Op {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			Op::Transfer { asset, to, amount } => {
				out.put_u8(1);
				out.put_var(*asset as u64);
				out.put(to);
				out.put_var(*amount);
			},
			Op::Cancel { nonces } => {
				out.put_u8(2);
				enc_vec(nonces, out);
			},
			Op::BumpNonce { floor } => {
				out.put_u8(3);
				out.put_var(*floor);
			},
			Op::AssetCreate { symbol, decimals, flags, kind, attestors, threshold } => {
				out.put_u8(4);
				out.put(symbol);
				out.put_u8(*decimals);
				out.put_u8(*flags);
				kind.enc(out);
				enc_vec(attestors, out);
				out.put_u8(*threshold);
			},
			Op::Mint { asset, to, amount } => {
				out.put_u8(5);
				out.put_var(*asset as u64);
				out.put(to);
				out.put_var(*amount);
			},
			Op::Burn { asset, amount } => {
				out.put_u8(6);
				out.put_var(*asset as u64);
				out.put_var(*amount);
			},
			Op::IssuerSet { asset, target, action, reason } => {
				out.put_u8(7);
				out.put_var(*asset as u64);
				out.put(target);
				out.put_u8(*action as u8);
				out.put(reason);
			},
			Op::Attest { asset, reserves } => {
				out.put_u8(8);
				out.put_var(*asset as u64);
				out.put_var(*reserves);
			},
			Op::Policy(c) => {
				out.put_u8(9);
				c.enc(out);
			},
			Op::PolicyCancel { index } => {
				out.put_u8(10);
				out.put_u8(*index);
			},
			Op::Freeze { target } => {
				out.put_u8(11);
				out.put(target);
			},
			Op::Unfreeze { target } => {
				out.put_u8(12);
				out.put(target);
			},
			Op::RecoveryApprove { target, new_owner } => {
				out.put_u8(13);
				out.put(target);
				out.put(new_owner);
			},
			Op::RecoveryCancel => out.put_u8(14),
			Op::DealRelease { deal } => {
				out.put_u8(15);
				out.put_var(*deal);
			},
			Op::OracleAttest { deal, verdict } => {
				out.put_u8(16);
				out.put_var(*deal);
				out.put_u8(*verdict as u8);
			},
			Op::GovProposeUpgrade { code_hash, code_len, min_item_gas, min_memo_gas } => {
				out.put_u8(17);
				out.put(code_hash);
				out.put_u32(*code_len);
				out.put_var(*min_item_gas);
				out.put_var(*min_memo_gas);
			},
			Op::GovCancelUpgrade => out.put_u8(18),
			Op::GovSetAdmin { admin } => {
				out.put_u8(19);
				enc_opt(admin, out);
			},
			Op::Accept { asset, on } => {
				out.put_u8(20);
				out.put_var(*asset as u64);
				out.put_bool(*on);
			},
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			1 => Op::Transfer { asset: r.var_u32()?, to: r.array()?, amount: r.var()? },
			2 => Op::Cancel { nonces: dec_vec(r, MAX_CANCEL)? },
			3 => Op::BumpNonce { floor: r.var()? },
			4 => Op::AssetCreate {
				symbol: r.array()?,
				decimals: r.u8()?,
				flags: r.u8()?,
				kind: AssetKindCfg::dec(r)?,
				attestors: dec_vec(r, MAX_ATTESTORS)?,
				threshold: r.u8()?,
			},
			5 => Op::Mint { asset: r.var_u32()?, to: r.array()?, amount: r.var()? },
			6 => Op::Burn { asset: r.var_u32()?, amount: r.var()? },
			7 => Op::IssuerSet {
				asset: r.var_u32()?,
				target: r.array()?,
				action: match r.u8()? {
					0 => IssuerAction::Allow,
					1 => IssuerAction::Disallow,
					2 => IssuerAction::Freeze,
					3 => IssuerAction::Unfreeze,
					_ => return Err(DecodeError),
				},
				reason: r.array()?,
			},
			8 => Op::Attest { asset: r.var_u32()?, reserves: r.var()? },
			9 => Op::Policy(PolicyChange::dec(r)?),
			10 => Op::PolicyCancel { index: r.u8()? },
			11 => Op::Freeze { target: r.array()? },
			12 => Op::Unfreeze { target: r.array()? },
			13 => Op::RecoveryApprove { target: r.array()?, new_owner: r.array()? },
			14 => Op::RecoveryCancel,
			15 => Op::DealRelease { deal: r.var()? },
			16 => Op::OracleAttest {
				deal: r.var()?,
				verdict: match r.u8()? {
					0 => Verdict::NotPaid,
					1 => Verdict::Paid,
					2 => Verdict::Void,
					_ => return Err(DecodeError),
				},
			},
			17 => Op::GovProposeUpgrade {
				code_hash: r.array()?,
				code_len: r.u32()?,
				min_item_gas: r.var()?,
				min_memo_gas: r.var()?,
			},
			18 => Op::GovCancelUpgrade,
			19 => Op::GovSetAdmin { admin: dec_opt(r)? },
			20 => Op::Accept { asset: r.var_u32()?, on: r.bool()? },
			_ => return Err(DecodeError),
		})
	}
}

impl Codec for Envelope {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.account);
		out.put_var(self.nonce);
		out.put_u32(self.expiry);
		self.op.enc(out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { account: r.array()?, nonce: r.var()?, expiry: r.u32()?, op: Op::dec(r)? })
	}
}

impl Codec for PlainOp {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			PlainOp::DealTimeout { deal } => {
				out.put_u8(1);
				out.put_var(*deal);
			},
			PlainOp::PolicyExec { account, index } => {
				out.put_u8(2);
				out.put(account);
				out.put_u8(*index);
			},
			PlainOp::RecoveryExec { account } => {
				out.put_u8(3);
				out.put(account);
			},
			PlainOp::GovExecUpgrade => out.put_u8(4),
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			1 => PlainOp::DealTimeout { deal: r.var()? },
			2 => PlainOp::PolicyExec { account: r.array()?, index: r.u8()? },
			3 => PlainOp::RecoveryExec { account: r.array()? },
			4 => PlainOp::GovExecUpgrade,
			_ => return Err(DecodeError),
		})
	}
}

// ---------------------------------------------------------------------------------------
// Orders
// ---------------------------------------------------------------------------------------

/// Fill what can be filled in the batch the order first trades in, then expire.
pub const ORDER_IOC: u8 = 1;
/// (Buyer of an external asset.) Do not require the payment to carry the deal tag.
pub const ORDER_NO_TAG: u8 = 2;
const ORDER_USER_FLAGS: u8 = ORDER_IOC | ORDER_NO_TAG;

/// A signed intention with a limit price. It lives off-chain until a bundler includes it in
/// a batch; the chain only remembers how much of it has been filled.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Order {
	pub account: AccountId,
	pub nonce: u64,
	pub side: Side,
	/// Most base to trade, in atoms.
	pub qty: u64,
	/// Worst acceptable price: a ceiling for buys, a floor for sells.
	pub limit: Price,
	/// Last slot in which the order may trade.
	pub expiry: Slot,
	pub max_fee_bps: u16,
	pub flags: u8,
	/// Buyers of an external asset say where it must be delivered (a Bitcoin output script).
	pub delivery: Vec<u8>,
}

impl Codec for Order {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.account);
		out.put_var(self.nonce);
		out.put_u8(matches!(self.side, Side::Sell) as u8);
		out.put_var(self.qty);
		self.limit.enc(out);
		out.put_u32(self.expiry);
		out.put_u16(self.max_fee_bps);
		out.put_u8(self.flags);
		out.put_var_bytes(&self.delivery);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		let o = Self {
			account: r.array()?,
			nonce: r.var()?,
			side: if r.bool()? { Side::Sell } else { Side::Buy },
			qty: r.var()?,
			limit: Price::dec(r)?,
			expiry: r.u32()?,
			max_fee_bps: r.u16()?,
			flags: r.u8()?,
			delivery: r.var_bytes()?.to_vec(),
		};
		if o.flags & !ORDER_USER_FLAGS != 0 || o.delivery.len() > MAX_SCRIPT_LEN {
			return Err(DecodeError);
		}
		Ok(o)
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignedOrder {
	pub signer: PubKey,
	pub sig: Sig,
	pub order: Order,
}

impl SignedOrder {
	pub fn sign(seed: &[u8; 32], service: u32, base: AssetId, quote: AssetId, order: Order) -> Self {
		let msg = order_message(service, base, quote, &order.to_bytes());
		Self { signer: crypto::ed25519_public(seed), sig: crypto::ed25519_sign(seed, &msg), order }
	}
}

impl Codec for SignedOrder {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.signer);
		out.put(&self.sig);
		self.order.enc(out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { signer: r.array()?, sig: r.array()?, order: Order::dec(r)? })
	}
}

// ---------------------------------------------------------------------------------------
// Payload items
// ---------------------------------------------------------------------------------------

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SignedOp {
	pub signer: PubKey,
	pub sig: Sig,
	/// An encoded [`Envelope`].
	pub body: Vec<u8>,
}

impl SignedOp {
	pub fn sign(seed: &[u8; 32], service: u32, env: &Envelope) -> Self {
		let body = env.to_bytes();
		let sig = crypto::ed25519_sign(seed, &op_message(service, &body));
		Self { signer: crypto::ed25519_public(seed), sig, body }
	}
}

/// One round of the batch auction for the market `base`/`quote`, assembled by anybody.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Batch {
	pub base: AssetId,
	pub quote: AssetId,
	/// Who receives the fee and the rounding dust: whoever built the batch.
	pub fee_account: AccountId,
	pub fee_bps: u16,
	pub orders: Vec<SignedOrder>,
}

/// A receipt showing that a transaction sits in a Bitcoin block.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BtcProof {
	pub deal: DealId,
	pub block_hash: Hash32,
	/// The transaction in its non-witness serialization (the form its txid is computed from).
	pub tx: Vec<u8>,
	/// Which output pays the deal.
	pub vout: u32,
	/// Position of the transaction in the block.
	pub index: u32,
	/// Merkle siblings from the transaction up to the root.
	pub branch: Vec<Hash32>,
	/// The block's coinbase transaction (non-witness serialization) and its Merkle path.
	/// Every transaction of a block sits at the same depth, and only a real coinbase passes
	/// for one, so its path fixes the depth of the tree. Without that, a 64-byte transaction
	/// could be read as an inner node and a transaction that never existed "proved" below it.
	pub coinbase: Vec<u8>,
	pub coinbase_branch: Vec<Hash32>,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Item {
	Signed(SignedOp),
	Batch(Batch),
	/// Raw 80-byte headers, each building on the one before.
	BtcHeaders { asset: AssetId, headers: Vec<[u8; 80]> },
	BtcProof(BtcProof),
	Plain(PlainOp),
}

impl Item {
	fn enc_body(&self, out: &mut Vec<u8>) -> u8 {
		match self {
			Item::Signed(s) => {
				out.put(&s.signer);
				out.put(&s.sig);
				out.put(&s.body);
				1
			},
			Item::Batch(b) => {
				out.put_u32(b.base);
				out.put_u32(b.quote);
				out.put(&b.fee_account);
				out.put_u16(b.fee_bps);
				enc_vec(&b.orders, out);
				2
			},
			Item::BtcHeaders { asset, headers } => {
				out.put_u32(*asset);
				out.put_var(headers.len() as u64);
				for h in headers {
					out.put(h);
				}
				3
			},
			Item::BtcProof(p) => {
				out.put_var(p.deal);
				out.put(&p.block_hash);
				out.put_var_bytes(&p.tx);
				out.put_u32(p.vout);
				out.put_u32(p.index);
				enc_vec(&p.branch, out);
				out.put_var_bytes(&p.coinbase);
				enc_vec(&p.coinbase_branch, out);
				4
			},
			Item::Plain(p) => {
				p.enc(out);
				5
			},
		}
	}

	pub fn dec_body(tag: u8, r: &mut Reader) -> DecodeResult<Self> {
		let item = match tag {
			1 => Item::Signed(SignedOp { signer: r.array()?, sig: r.array()?, body: r.rest().to_vec() }),
			2 => Item::Batch(Batch {
				base: r.u32()?,
				quote: r.u32()?,
				fee_account: r.array()?,
				fee_bps: r.u16()?,
				orders: dec_vec(r, MAX_BATCH_ORDERS)?,
			}),
			3 => {
				let asset = r.u32()?;
				let n = r.var_usize()?;
				if n > MAX_HEADERS_PER_ITEM || n * 80 != r.remaining() {
					return Err(DecodeError);
				}
				let mut headers = Vec::with_capacity(n);
				for _ in 0..n {
					headers.push(r.array()?);
				}
				Item::BtcHeaders { asset, headers }
			},
			4 => Item::BtcProof(BtcProof {
				deal: r.var()?,
				block_hash: r.array()?,
				tx: {
					let t = r.var_bytes()?;
					if t.len() > MAX_TX_LEN {
						return Err(DecodeError);
					}
					t.to_vec()
				},
				vout: r.u32()?,
				index: r.u32()?,
				branch: dec_vec(r, MAX_BRANCH)?,
				coinbase: {
					let t = r.var_bytes()?;
					if t.len() > MAX_TX_LEN {
						return Err(DecodeError);
					}
					t.to_vec()
				},
				coinbase_branch: dec_vec(r, MAX_BRANCH)?,
			}),
			5 => Item::Plain(PlainOp::dec(r)?),
			_ => return Err(DecodeError),
		};
		r.finish()?;
		Ok(item)
	}
}

/// Serialize items into a work-item payload. Each item is framed (`tag`, length, body) so
/// that a malformed one can be skipped without losing the rest.
pub fn encode_payload(items: &[Item]) -> Vec<u8> {
	let mut out = alloc::vec![PAYLOAD_VERSION];
	let mut body = Vec::new();
	for it in items {
		body.clear();
		let tag = it.enc_body(&mut body);
		out.put_u8(tag);
		out.put_var_bytes(&body);
	}
	out
}

/// Iterate the frames of a payload: `(tag, body)`.
pub fn payload_frames(payload: &[u8]) -> DecodeResult<Vec<(u8, &[u8])>> {
	let mut r = Reader::new(payload);
	if r.u8()? != PAYLOAD_VERSION {
		return Err(DecodeError);
	}
	let mut v = Vec::new();
	while !r.is_empty() {
		let tag = r.u8()?;
		v.push((tag, r.var_bytes()?));
	}
	Ok(v)
}

// ---------------------------------------------------------------------------------------
// Records: the refine output
// ---------------------------------------------------------------------------------------

const REC_SIGNER_DIFFERS: u8 = 0x80;
const REC_HAS_DELIVERY: u8 = 0x40;

/// An order whose signature refine has verified.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OrderRec {
	pub signer: PubKey,
	pub account: AccountId,
	pub nonce: u64,
	pub qty: u64,
	pub limit: Price,
	pub expiry: Slot,
	pub max_fee_bps: u16,
	pub flags: u8,
	/// First eight bytes of the order hash. Zero for immediate-or-cancel orders, which never
	/// rest and so never need it.
	pub tag: [u8; 8],
	pub delivery: Vec<u8>,
}

impl Codec for OrderRec {
	fn enc(&self, out: &mut Vec<u8>) {
		let mut f = self.flags & ORDER_USER_FLAGS;
		if self.signer != self.account {
			f |= REC_SIGNER_DIFFERS;
		}
		if !self.delivery.is_empty() {
			f |= REC_HAS_DELIVERY;
		}
		out.put_u8(f);
		out.put(&self.account);
		if f & REC_SIGNER_DIFFERS != 0 {
			out.put(&self.signer);
		}
		out.put_var(self.nonce);
		out.put_var(self.qty);
		self.limit.enc(out);
		out.put_u32(self.expiry);
		out.put_var(self.max_fee_bps as u64);
		if f & ORDER_IOC == 0 {
			out.put(&self.tag);
		}
		if f & REC_HAS_DELIVERY != 0 {
			out.put_var_bytes(&self.delivery);
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		let f = r.u8()?;
		let account: AccountId = r.array()?;
		let signer = if f & REC_SIGNER_DIFFERS != 0 { r.array()? } else { account };
		Ok(Self {
			signer,
			account,
			nonce: r.var()?,
			qty: r.var()?,
			limit: Price::dec(r)?,
			expiry: r.u32()?,
			max_fee_bps: u16::try_from(r.var()?).map_err(|_| DecodeError)?,
			flags: f & ORDER_USER_FLAGS,
			tag: if f & ORDER_IOC == 0 { r.array()? } else { [0; 8] },
			delivery: if f & REC_HAS_DELIVERY != 0 { r.var_bytes()?.to_vec() } else { Vec::new() },
		})
	}
}

/// One auction round as refine leaves it: the crossing book. Every order is authenticated,
/// each side is sorted best price first, and orders that could not trade against anything
/// in the batch have been removed. The clearing price is fixed by accumulate, over the
/// orders that turn out to be funded.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BatchRec {
	pub base: AssetId,
	pub quote: AssetId,
	pub fee_account: AccountId,
	pub fee_bps: u16,
	pub bids: Vec<OrderRec>,
	pub asks: Vec<OrderRec>,
}

/// What accumulate needs to know about a header whose proof-of-work refine has checked.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct HeaderDigest {
	pub hash: Hash32,
	pub merkle_root: Hash32,
	pub time: u32,
	pub bits: u32,
}

impl Codec for HeaderDigest {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.hash);
		out.put(&self.merkle_root);
		out.put_u32(self.time);
		out.put_u32(self.bits);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { hash: r.array()?, merkle_root: r.array()?, time: r.u32()?, bits: r.u32()? })
	}
}

/// A payment refine found in a transaction and tied by a Merkle path to `merkle_root`.
/// Whether that root belongs to a sufficiently buried block is for accumulate to decide.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PaymentRec {
	pub deal: DealId,
	pub block_hash: Hash32,
	pub merkle_root: Hash32,
	pub txid: Hash32,
	pub vout: u32,
	pub value: u64,
	pub script: Vec<u8>,
	pub tag: TagState,
}

/// What a transaction says about which deal it pays.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum TagState {
	/// No tag: it can settle a deal whose buyer waived the tag.
	None = 0,
	/// It carries this deal's tag.
	This = 1,
	/// It carries a tag, and not this deal's: it was made for another deal and cannot
	/// settle this one, whatever this one's buyer waived.
	Other = 2,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Record {
	Op { signer: PubKey, body: Vec<u8> },
	Batch(BatchRec),
	BtcHeaders { asset: AssetId, prev: Hash32, headers: Vec<HeaderDigest> },
	BtcPayment(PaymentRec),
	Plain(PlainOp),
}

impl Record {
	fn enc_body(&self, out: &mut Vec<u8>) -> u8 {
		match self {
			Record::Op { signer, body } => {
				out.put(signer);
				out.put(body);
				1
			},
			Record::Batch(b) => {
				out.put_var(b.base as u64);
				out.put_var(b.quote as u64);
				out.put(&b.fee_account);
				out.put_var(b.fee_bps as u64);
				enc_vec(&b.bids, out);
				enc_vec(&b.asks, out);
				2
			},
			Record::BtcHeaders { asset, prev, headers } => {
				out.put_var(*asset as u64);
				out.put(prev);
				enc_vec(headers, out);
				3
			},
			Record::BtcPayment(p) => {
				out.put_var(p.deal);
				out.put(&p.block_hash);
				out.put(&p.merkle_root);
				out.put(&p.txid);
				out.put_var(p.vout as u64);
				out.put_var(p.value);
				out.put_var_bytes(&p.script);
				out.put_u8(p.tag as u8);
				4
			},
			Record::Plain(p) => {
				p.enc(out);
				5
			},
		}
	}

	pub fn dec_body(tag: u8, r: &mut Reader) -> DecodeResult<Self> {
		let rec = match tag {
			1 => Record::Op { signer: r.array()?, body: r.rest().to_vec() },
			2 => Record::Batch(BatchRec {
				base: r.var_u32()?,
				quote: r.var_u32()?,
				fee_account: r.array()?,
				fee_bps: u16::try_from(r.var()?).map_err(|_| DecodeError)?,
				bids: dec_vec(r, MAX_BATCH_ORDERS)?,
				asks: dec_vec(r, MAX_BATCH_ORDERS)?,
			}),
			3 => Record::BtcHeaders {
				asset: r.var_u32()?,
				prev: r.array()?,
				headers: dec_vec(r, MAX_HEADERS_PER_ITEM)?,
			},
			4 => Record::BtcPayment(PaymentRec {
				deal: r.var()?,
				block_hash: r.array()?,
				merkle_root: r.array()?,
				txid: r.array()?,
				vout: r.var_u32()?,
				value: r.var()?,
				script: r.var_bytes()?.to_vec(),
				tag: match r.u8()? {
					0 => TagState::None,
					1 => TagState::This,
					2 => TagState::Other,
					_ => return Err(DecodeError),
				},
			}),
			5 => Record::Plain(PlainOp::dec(r)?),
			_ => return Err(DecodeError),
		};
		r.finish()?;
		Ok(rec)
	}
}

/// Append one framed record to a refine output under construction.
pub fn push_record(out: &mut Vec<u8>, rec: &Record) {
	if out.is_empty() {
		out.put_u8(OUTPUT_VERSION);
	}
	let mut body = Vec::new();
	let tag = rec.enc_body(&mut body);
	out.put_u8(tag);
	out.put_var_bytes(&body);
}

pub fn encode_records(recs: &[Record]) -> Vec<u8> {
	let mut out = Vec::new();
	for r in recs {
		push_record(&mut out, r);
	}
	out
}

/// Split a refine output into `(tag, body)` frames. An empty output has no frames.
pub fn output_frames(output: &[u8]) -> DecodeResult<Vec<(u8, &[u8])>> {
	if output.is_empty() {
		return Ok(Vec::new());
	}
	let mut r = Reader::new(output);
	if r.u8()? != OUTPUT_VERSION {
		return Err(DecodeError);
	}
	let mut v = Vec::new();
	while !r.is_empty() {
		let tag = r.u8()?;
		v.push((tag, r.var_bytes()?));
	}
	Ok(v)
}

#[cfg(test)]
mod tests {
	use super::*;
	use alloc::vec;

	fn sample_ops() -> Vec<Op> {
		vec![
			Op::Transfer { asset: 3, to: [1; 32], amount: 500 },
			Op::Cancel { nonces: vec![1, 2, 99] },
			Op::BumpNonce { floor: 10 },
			Op::AssetCreate {
				symbol: *b"BRLA\0\0\0\0",
				decimals: 2,
				flags: ASSET_PERMISSIONED | ASSET_FREEZABLE,
				kind: AssetKindCfg::Token,
				attestors: vec![[7; 32], [8; 32]],
				threshold: 2,
			},
			Op::AssetCreate {
				symbol: *b"BTC\0\0\0\0\0",
				decimals: 8,
				flags: 0,
				kind: AssetKindCfg::External {
					cfg: ExternalCfg {
						rail: Rail::BtcSpv { confirmations: 6, pay_secs: 7_200, grace_blocks: 12 },
						bond_bps: 1000,
						claim_window: 1200,
						min_deal: 1000,
					},
					btc: Some((
						BtcParams { pow_limit_bits: 0x1d00ffff, retarget: true },
						BtcCheckpoint {
							height: 840_000,
							hash: [9; 32],
							time: 1_713_571_767,
							bits: 0x17034219,
							period_start: 1_712_000_000,
							chainwork: 1u128 << 95,
						},
					)),
				},
				attestors: vec![],
				threshold: 0,
			},
			Op::AssetCreate {
				symbol: *b"PIX\0\0\0\0\0",
				decimals: 2,
				flags: 0,
				kind: AssetKindCfg::External {
					cfg: ExternalCfg {
						rail: Rail::Oracle { oracles: vec![[4; 32]], threshold: 1 },
						bond_bps: 500,
						claim_window: 300,
						min_deal: 1,
					},
					btc: None,
				},
				attestors: vec![],
				threshold: 0,
			},
			Op::Mint { asset: 1, to: [2; 32], amount: u64::MAX },
			Op::Burn { asset: 1, amount: 9 },
			Op::IssuerSet { asset: 1, target: [3; 32], action: IssuerAction::Freeze, reason: [5; 32] },
			Op::Attest { asset: 1, reserves: 1_000_000 },
			Op::Policy(PolicyChange::SetGuardian(Some([6; 32]))),
			Op::PolicyCancel { index: 2 },
			Op::Freeze { target: [1; 32] },
			Op::Unfreeze { target: [1; 32] },
			Op::RecoveryApprove { target: [1; 32], new_owner: [2; 32] },
			Op::RecoveryCancel,
			Op::DealRelease { deal: 77 },
			Op::OracleAttest { deal: 77, verdict: Verdict::Paid },
			Op::OracleAttest { deal: 77, verdict: Verdict::Void },
			Op::Accept { asset: 3, on: true },
			Op::Accept { asset: 3, on: false },
			Op::GovProposeUpgrade { code_hash: [8; 32], code_len: 1234, min_item_gas: 10, min_memo_gas: 20 },
			Op::GovCancelUpgrade,
			Op::GovSetAdmin { admin: None },
			Op::GovSetAdmin { admin: Some([1; 32]) },
		]
	}

	#[test]
	fn ops_roundtrip() {
		for op in sample_ops() {
			let env = Envelope { account: [0xaa; 32], nonce: 12345, expiry: 99, op };
			let bytes = env.to_bytes();
			assert_eq!(Envelope::from_bytes(&bytes).unwrap(), env);
			// Trailing garbage and truncation are both rejected.
			let mut longer = bytes.clone();
			longer.push(0);
			assert!(Envelope::from_bytes(&longer).is_err());
			assert!(Envelope::from_bytes(&bytes[..bytes.len() - 1]).is_err());
		}
		for p in [
			PlainOp::DealTimeout { deal: 5 },
			PlainOp::PolicyExec { account: [1; 32], index: 0 },
			PlainOp::RecoveryExec { account: [1; 32] },
			PlainOp::GovExecUpgrade,
		] {
			assert_eq!(PlainOp::from_bytes(&p.to_bytes()).unwrap(), p);
		}
	}

	fn sample_order(side: Side) -> Order {
		Order {
			account: [3; 32],
			nonce: 42,
			side,
			qty: 1_000_000,
			limit: Price::new(60_000_000, 100_000_000),
			expiry: 1000,
			max_fee_bps: 10,
			flags: 0,
			delivery: vec![0x00, 0x14, 1, 2, 3],
		}
	}

	#[test]
	fn payload_roundtrip() {
		let seed = [5u8; 32];
		let env = Envelope { account: crypto::ed25519_public(&seed), nonce: 0, expiry: 9, op: Op::RecoveryCancel };
		let items = vec![
			Item::Signed(SignedOp::sign(&seed, 9, &env)),
			Item::Batch(Batch {
				base: 1,
				quote: 2,
				fee_account: [4; 32],
				fee_bps: 5,
				orders: vec![
					SignedOrder::sign(&seed, 9, 1, 2, sample_order(Side::Buy)),
					SignedOrder::sign(&seed, 9, 1, 2, sample_order(Side::Sell)),
				],
			}),
			Item::BtcHeaders { asset: 3, headers: vec![[7u8; 80], [8u8; 80]] },
			Item::BtcProof(BtcProof {
				deal: 1,
				block_hash: [1; 32],
				tx: vec![1, 2, 3, 4],
				vout: 1,
				index: 5,
				branch: vec![[2; 32], [3; 32]],
				coinbase: vec![9, 9],
				coinbase_branch: vec![[4; 32], [5; 32]],
			}),
			Item::Plain(PlainOp::GovExecUpgrade),
		];
		let payload = encode_payload(&items);
		let frames = payload_frames(&payload).unwrap();
		assert_eq!(frames.len(), items.len());
		for ((tag, body), item) in frames.into_iter().zip(&items) {
			assert_eq!(&Item::dec_body(tag, &mut Reader::new(body)).unwrap(), item);
		}
		assert!(payload_frames(&[2]).is_err());
		assert!(payload_frames(&[]).is_err());
	}

	#[test]
	fn records_roundtrip() {
		let rec = |ioc: bool, diff: bool, delivery: Vec<u8>| OrderRec {
			signer: if diff { [9; 32] } else { [3; 32] },
			account: [3; 32],
			nonce: 7,
			qty: 100,
			limit: Price::new(3, 2),
			expiry: 50,
			max_fee_bps: 10,
			flags: if ioc { ORDER_IOC } else { 0 },
			tag: if ioc { [0; 8] } else { [1, 2, 3, 4, 5, 6, 7, 8] },
			delivery,
		};
		let recs = vec![
			Record::Op { signer: [1; 32], body: vec![1, 2, 3] },
			Record::Batch(BatchRec {
				base: 1,
				quote: 2,
				fee_account: [4; 32],
				fee_bps: 30,
				bids: vec![rec(false, false, vec![]), rec(true, true, vec![1, 2])],
				asks: vec![rec(true, false, vec![])],
			}),
			Record::BtcHeaders {
				asset: 3,
				prev: [5; 32],
				headers: vec![HeaderDigest { hash: [1; 32], merkle_root: [2; 32], time: 3, bits: 4 }],
			},
			Record::BtcPayment(PaymentRec {
				deal: 9,
				block_hash: [1; 32],
				merkle_root: [2; 32],
				txid: [3; 32],
				vout: 1,
				value: 5000,
				script: vec![0x51],
				tag: TagState::This,
			}),
			Record::Plain(PlainOp::DealTimeout { deal: 3 }),
		];
		let out = encode_records(&recs);
		let frames = output_frames(&out).unwrap();
		assert_eq!(frames.len(), recs.len());
		for ((tag, body), r) in frames.into_iter().zip(&recs) {
			assert_eq!(&Record::dec_body(tag, &mut Reader::new(body)).unwrap(), r);
		}
		assert!(output_frames(&[]).unwrap().is_empty());
	}

	#[test]
	fn messages_are_domain_separated() {
		let body = [1u8, 2, 3];
		assert_ne!(op_message(1, &body), op_message(2, &body));
		assert_ne!(order_message(1, 1, 2, &body), order_message(1, 2, 1, &body));
		assert_ne!(op_message(1, &body)[..], order_message(1, 0, 0, &body)[..]);
		assert_ne!(deal_tag(1, 1), deal_tag(1, 2));
		assert_ne!(deal_tag(1, 1), deal_tag(2, 1));
	}
}
