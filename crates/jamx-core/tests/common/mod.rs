//! A tiny in-memory "chain" for scenario tests: payloads go through the real refine and the
//! real accumulate, against an in-memory host.
#![allow(dead_code)]

use jamx_core::{
	accumulate::{Genesis, Receipt, Session, RECEIPT_RING},
	btc::{self, HeaderRec, LightClient},
	codec::Codec,
	crypto::{blake2b, ed25519_public, Hash32},
	ops::*,
	refine::refine,
	state::{key, mock::MockHost},
	types::*,
};
use std::collections::BTreeMap;

pub const SVC: u32 = 42;

#[derive(Clone, Copy)]
pub struct Actor {
	pub seed: [u8; 32],
	pub id: AccountId,
}

pub fn actor(n: u8) -> Actor {
	let seed = [n; 32];
	Actor { seed, id: ed25519_public(&seed) }
}

pub struct World {
	pub host: MockHost,
	pub slot: Slot,
	pub genesis: Genesis,
	nonces: BTreeMap<AccountId, u64>,
}

pub fn ok(statuses: &[u8]) -> bool {
	statuses.iter().all(|s| *s == Status::Ok as u8)
}

impl World {
	pub fn new() -> Self {
		Self::with_genesis(Genesis::default())
	}

	pub fn with_genesis(genesis: Genesis) -> Self {
		Self { host: MockHost::default(), slot: 1000, genesis, nonces: BTreeMap::new() }
	}

	pub fn nonce(&mut self, id: &AccountId) -> u64 {
		let n = self.nonces.entry(*id).or_insert(0);
		*n += 1;
		*n - 1
	}

	/// Run a payload through refine and accumulate. Returns one status per record.
	pub fn submit(&mut self, items: Vec<Item>) -> Vec<u8> {
		let payload = encode_payload(&items);
		let output = refine(SVC, &payload);
		let mut s = Session::new(&mut self.host, SVC, self.slot, &self.genesis);
		let st = s.work(&blake2b(&[&payload]), &output, u64::MAX);
		s.finish();
		self.check_invariants();
		st
	}

	pub fn sign(&mut self, who: &Actor, op: Op) -> Item {
		let nonce = self.nonce(&who.id);
		Item::Signed(SignedOp::sign(&who.seed, SVC, &Envelope { account: who.id, nonce, expiry: Slot::MAX, op }))
	}

	/// Sign as `key` on behalf of the account `account` (a second device, a new owner key).
	pub fn sign_as(&mut self, key: &Actor, account: &AccountId, op: Op) -> Item {
		let nonce = self.nonce(account);
		Item::Signed(SignedOp::sign(&key.seed, SVC, &Envelope { account: *account, nonce, expiry: Slot::MAX, op }))
	}

	/// Submit one signed operation and return its status.
	pub fn op(&mut self, who: &Actor, op: Op) -> u8 {
		let item = self.sign(who, op);
		let st = self.submit(vec![item]);
		assert_eq!(st.len(), 1, "the operation was dropped by refine");
		st[0]
	}

	pub fn op_as(&mut self, key: &Actor, account: &AccountId, op: Op) -> u8 {
		let item = self.sign_as(key, account, op);
		let st = self.submit(vec![item]);
		assert_eq!(st.len(), 1);
		st[0]
	}

	pub fn plain(&mut self, op: PlainOp) -> u8 {
		self.submit(vec![Item::Plain(op)])[0]
	}

	pub fn must(&mut self, who: &Actor, op: Op) {
		let dbg = format!("{op:?}");
		assert_eq!(self.op(who, op), Status::Ok as u8, "{dbg}");
	}

	// ---- reading state ----

	pub fn account(&self, id: &AccountId) -> Account {
		self.host
			.store
			.get(&key::account(id))
			.map(|b| Account::from_bytes(b).unwrap())
			.unwrap_or_default()
	}

	pub fn bal(&self, who: &Actor, asset: AssetId) -> u64 {
		self.account(&who.id).free(asset)
	}

	pub fn asset(&self, id: AssetId) -> Asset {
		Asset::from_bytes(&self.host.store[&key::asset(id)]).unwrap()
	}

	pub fn config(&self) -> Config {
		Config::from_bytes(&self.host.store[key::CONFIG]).unwrap()
	}

	pub fn deal(&self, id: DealId) -> Option<Deal> {
		self.host.store.get(&key::deal(id)).map(|b| Deal::from_bytes(b).unwrap())
	}

	pub fn deals(&self) -> Vec<(DealId, Deal)> {
		self.host
			.store
			.iter()
			.filter(|(k, _)| k[0] == b'd')
			.map(|(k, v)| {
				(u64::from_le_bytes(k[1..9].try_into().unwrap()), Deal::from_bytes(v).unwrap())
			})
			.collect()
	}

	pub fn chain(&self, asset: AssetId) -> LightClient {
		LightClient::from_bytes(&self.host.store[&key::chain(asset)]).unwrap()
	}

	pub fn header(&self, asset: AssetId, hash: &Hash32) -> Option<HeaderRec> {
		self.host.store.get(&key::header(asset, hash)).map(|b| HeaderRec::from_bytes(b).unwrap())
	}

	pub fn market(&self, base: AssetId, quote: AssetId) -> Option<Market> {
		self.host.store.get(&key::market(base, quote)).map(|b| Market::from_bytes(b).unwrap())
	}

	pub fn last_receipt(&self) -> Receipt {
		let seq = self.config().receipt_seq - 1;
		Receipt::from_bytes(&self.host.store[&key::receipt((seq % RECEIPT_RING) as u8)]).unwrap()
	}

	/// The books must balance: for every token, what accounts hold plus what deals have
	/// locked equals the supply. Nothing is ever created or lost by trading.
	pub fn check_invariants(&self) {
		let mut held: BTreeMap<AssetId, u128> = BTreeMap::new();
		for (k, v) in &self.host.store {
			match k[0] {
				b'a' => {
					let a = Account::from_bytes(v).expect("account decodes");
					assert!(!a.is_empty(), "empty accounts are not stored");
					for b in &a.balances {
						*held.entry(b.asset).or_default() += b.free as u128;
					}
					assert!(a.open.len() <= MAX_OPEN_ORDERS);
				},
				b'd' => {
					let d = Deal::from_bytes(v).expect("deal decodes");
					*held.entry(d.token).or_default() += d.amount as u128 + d.bond as u128;
				},
				_ => {},
			}
		}
		for (k, v) in &self.host.store {
			if k[0] == b't' {
				let id = u32::from_le_bytes(k[1..5].try_into().unwrap());
				let a = Asset::from_bytes(v).expect("asset decodes");
				assert_eq!(
					held.get(&id).copied().unwrap_or(0),
					a.supply as u128,
					"asset {id}: holdings must equal supply"
				);
				assert!(a.supply <= a.mint_cap() || !a.attestors.is_empty());
			}
		}
		for id in held.keys() {
			assert!(self.host.store.contains_key(&key::asset(*id)), "balance in unknown asset {id}");
		}
	}

	// ---- shortcuts ----

	/// Create a plain token issued by `issuer` and return its id.
	pub fn token(&mut self, issuer: &Actor, symbol: &str, flags: u8) -> AssetId {
		let id = self.next_asset();
		self.must(
			issuer,
			Op::AssetCreate {
				symbol: sym(symbol),
				decimals: 2,
				flags,
				kind: AssetKindCfg::Token,
				attestors: vec![],
				threshold: 0,
			},
		);
		id
	}

	pub fn next_asset(&self) -> AssetId {
		self.host.store.get(key::CONFIG).map_or(1, |b| Config::from_bytes(b).unwrap().next_asset)
	}

	/// Mint to `to`, who first accepts the asset if it has not yet (an account holds only
	/// what it has accepted).
	pub fn mint(&mut self, issuer: &Actor, asset: AssetId, to: &Actor, amount: u64) {
		self.accept(to, asset);
		self.must(issuer, Op::Mint { asset, to: to.id, amount });
	}

	pub fn accept(&mut self, who: &Actor, asset: AssetId) {
		if !self.account(&who.id).accepts(asset) {
			self.must(who, Op::Accept { asset, on: true });
		}
	}

	pub fn order(
		&mut self,
		who: &Actor,
		base: AssetId,
		quote: AssetId,
		side: Side,
		qty: u64,
		limit: (u64, u64),
	) -> SignedOrder {
		self.order_with(who, base, quote, side, qty, limit, 0, vec![])
	}

	#[allow(clippy::too_many_arguments)]
	pub fn order_with(
		&mut self,
		who: &Actor,
		base: AssetId,
		quote: AssetId,
		side: Side,
		qty: u64,
		limit: (u64, u64),
		flags: u8,
		delivery: Vec<u8>,
	) -> SignedOrder {
		let nonce = self.nonce(&who.id);
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
				expiry: self.slot + 10_000,
				max_fee_bps: MAX_FEE_BPS,
				flags,
				delivery,
			},
		)
	}

	pub fn batch(
		&mut self,
		base: AssetId,
		quote: AssetId,
		builder: &Actor,
		fee_bps: u16,
		orders: Vec<SignedOrder>,
	) -> u8 {
		let st = self.submit(vec![Item::Batch(Batch { base, quote, fee_account: builder.id, fee_bps, orders })]);
		st.first().copied().unwrap_or(Status::NoMatch as u8)
	}
}

pub fn sym(s: &str) -> [u8; 8] {
	let mut out = [0u8; 8];
	out[..s.len()].copy_from_slice(s.as_bytes());
	out
}

// ---- a local Bitcoin regtest chain ----

pub struct Block {
	pub header: btc::Header,
	pub txs: Vec<Vec<u8>>,
}

impl Block {
	pub fn hash(&self) -> Hash32 {
		self.header.hash()
	}
	pub fn txids(&self) -> Vec<Hash32> {
		self.txs.iter().map(|t| jamx_core::crypto::sha256d(t)).collect()
	}
	/// The receipt for transaction `index` of this block.
	pub fn proof(&self, deal: DealId, index: usize, vout: u32) -> BtcProof {
		let (_, branch) = btc::merkle_branch(&self.txids(), index);
		let (_, coinbase_branch) = btc::merkle_branch(&self.txids(), 0);
		BtcProof {
			deal,
			block_hash: self.hash(),
			tx: self.txs[index].clone(),
			vout,
			index: index as u32,
			branch,
			coinbase: self.txs[0].clone(),
			coinbase_branch,
		}
	}
}

pub struct Regtest {
	pub blocks: Vec<Block>,
	salt: u32,
	/// Timestamp the next block will carry; each block moves it on by ten minutes.
	pub clock: u32,
}

/// Wall-clock time of a JAM slot, as the service derives it.
pub fn wall_clock(slot: Slot) -> u32 {
	(JAM_EPOCH_UNIX + slot as u64 * SLOT_SECONDS) as u32
}

impl Regtest {
	/// A chain whose first block is dated at JAM's slot zero. Ten blocks later its clock
	/// reads what JAM's does at slot 1000, where every test world starts.
	pub fn new() -> Self {
		let mut r = Self { blocks: vec![], salt: 0, clock: wall_clock(0) };
		r.mine_on([0; 32], vec![]);
		r
	}

	/// Height of the best block.
	pub fn height(&self) -> usize {
		self.blocks.len() - 1
	}

	pub fn tip(&self) -> Hash32 {
		self.blocks.last().unwrap().hash()
	}

	fn coinbase(&mut self) -> Vec<u8> {
		self.salt += 1;
		btc::build_coinbase(self.salt, &[(50_0000_0000, vec![0x51])])
	}

	/// Mine a block with the given transactions (after a coinbase) on top of `prev`.
	pub fn mine_on(&mut self, prev: Hash32, txs: Vec<Vec<u8>>) -> usize {
		let mut all = vec![self.coinbase()];
		all.extend(txs);
		let txids: Vec<Hash32> = all.iter().map(|t| jamx_core::crypto::sha256d(t)).collect();
		let (root, _) = btc::merkle_branch(&txids, 0);
		let time = self.clock;
		self.clock += 600;
		let header = btc::mine(prev, root, time, btc::REGTEST.pow_limit_bits);
		self.blocks.push(Block { header, txs: all });
		self.blocks.len() - 1
	}

	pub fn mine(&mut self, txs: Vec<Vec<u8>>) -> usize {
		let tip = self.tip();
		self.mine_on(tip, txs)
	}

	pub fn mine_empty(&mut self, n: usize) {
		for _ in 0..n {
			self.mine(vec![]);
		}
	}

	pub fn checkpoint(&self) -> BtcCheckpoint {
		let g = &self.blocks[0];
		BtcCheckpoint {
			height: 0,
			hash: g.hash(),
			time: g.header.time,
			bits: g.header.bits,
			period_start: g.header.time,
			chainwork: 0,
		}
	}

	pub fn headers(&self, from: usize) -> Vec<[u8; 80]> {
		self.blocks[from..].iter().map(|b| b.header.serialize()).collect()
	}
}

/// A P2WPKH-shaped output script for tests.
pub fn script(n: u8) -> Vec<u8> {
	let mut s = vec![0x00, 0x14];
	s.extend_from_slice(&[n; 20]);
	s
}
