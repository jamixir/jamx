//! Storage access for accumulate.
//!
//! [`Ctx`] is a write-back cache over the service's key/value store with a journal:
//!
//! * every record is read from the host at most once per invocation and written at most
//!   once, at [`Ctx::flush`];
//! * each instruction runs between [`Ctx::begin`] and either nothing (success) or
//!   [`Ctx::rollback`] (failure), so a failed instruction leaves no trace however far it got.

use crate::{
	codec::Codec,
	crypto::Hash32,
	types::*,
};
use alloc::{boxed::Box, collections::BTreeMap, vec::Vec};

/// The services the JAM host gives to accumulate, reduced to what this service uses.
pub trait Host {
/// Read the value stored under `key` into `buf`, replacing its contents. Returns
	/// `false` if there is none. Taking the buffer from the caller lets one allocation
	/// serve every read of an invocation.
	fn read(&mut self, key: &[u8], buf: &mut Vec<u8>) -> bool;
	/// Returns `false` if the write was refused because the service's balance does not
	/// cover the storage deposit.
	fn write(&mut self, key: &[u8], value: &[u8]) -> bool;
	fn remove(&mut self, key: &[u8]);
	/// Gas left.
	fn gas(&mut self) -> u64;
	/// Commit everything written so far, even if the invocation later runs out of gas.
	fn checkpoint(&mut self);
	fn upgrade(&mut self, code_hash: &Hash32, min_item_gas: u64, min_memo_gas: u64);
	/// Ask the chain to make a preimage available. `false` if refused.
	fn solicit(&mut self, hash: &Hash32, len: u32) -> bool;
	fn preimage_available(&mut self, hash: &Hash32) -> bool;
	fn log(&mut self, _msg: &str) {}
}

/// Storage keys: one prefix byte per kind of record.
pub mod key {
	use super::*;

	pub const CONFIG: &[u8] = b"c";

	fn with(prefix: u8, parts: &[&[u8]]) -> Vec<u8> {
		let mut k = Vec::with_capacity(1 + parts.iter().map(|p| p.len()).sum::<usize>());
		k.push(prefix);
		for p in parts {
			k.extend_from_slice(p);
		}
		k
	}
	pub fn account(id: &AccountId) -> Vec<u8> {
		with(b'a', &[id])
	}
	pub fn asset(id: AssetId) -> Vec<u8> {
		with(b't', &[&id.to_le_bytes()])
	}
	pub fn deal(id: DealId) -> Vec<u8> {
		with(b'd', &[&id.to_le_bytes()])
	}
	/// Light-client state of an external asset.
	pub fn chain(asset: AssetId) -> Vec<u8> {
		with(b'l', &[&asset.to_le_bytes()])
	}
	pub fn header(asset: AssetId, hash: &Hash32) -> Vec<u8> {
		with(b'h', &[&asset.to_le_bytes(), hash])
	}
	/// Best-chain index: height to block hash.
	pub fn height(asset: AssetId, height: u32) -> Vec<u8> {
		with(b'n', &[&asset.to_le_bytes(), &height.to_le_bytes()])
	}
	/// Marks a transaction output as already used to settle a deal.
	pub fn spent(asset: AssetId, txid: &Hash32, vout: u32) -> Vec<u8> {
		with(b'u', &[&asset.to_le_bytes(), txid, &vout.to_le_bytes()])
	}
	/// The one open deal that an untagged payment to `delivery` can settle.
	pub fn untagged(asset: AssetId, delivery: &[u8]) -> Vec<u8> {
		with(b'x', &[&asset.to_le_bytes(), delivery])
	}
	pub fn freeze_log(asset: AssetId, seq: u32) -> Vec<u8> {
		with(b'f', &[&asset.to_le_bytes(), &seq.to_le_bytes()])
	}
	pub fn market(base: AssetId, quote: AssetId) -> Vec<u8> {
		with(b'm', &[&base.to_le_bytes(), &quote.to_le_bytes()])
	}
	pub fn receipt(slot_in_ring: u8) -> Vec<u8> {
		with(b'r', &[&[slot_in_ring]])
	}
}

#[derive(Clone)]
struct Cached<T> {
	val: T,
	dirty: bool,
	/// The instruction that last journaled this entry.
	epoch: u32,
}

/// A cached account. Accounts are the hot path, so they live in a vector with a small
/// hash index and handlers address them by position instead of by 32-byte key.
#[derive(Clone)]
struct AcctEntry {
	id: AccountId,
	val: Account,
	dirty: bool,
	epoch: u32,
	/// Next entry in the same hash bucket.
	next: u32,
}

const BUCKETS: usize = 256;
const NIL: u32 = u32::MAX;

enum Undo {
	Account(usize, Account, bool),
	// The rarely journaled kinds are boxed so that the common one stays small to move.
	Asset(AssetId, Box<Cached<Option<Asset>>>),
	Config(Box<Cached<Config>>),
	Raw(Vec<u8>, Cached<Option<Vec<u8>>>),
}

pub struct Ctx<'a, H: Host> {
	pub host: &'a mut H,
	pub service: u32,
	pub now: Slot,
	accounts: Vec<AcctEntry>,
	buckets: [u32; BUCKETS],
	assets: BTreeMap<AssetId, Cached<Option<Asset>>>,
	config: Option<Cached<Config>>,
	raw: BTreeMap<Vec<u8>, Cached<Option<Vec<u8>>>>,
	undo: Vec<Undo>,
	/// Reused for every host read.
	scratch: Vec<u8>,
	epoch: u32,
	/// Number of records the next flush will write.
	dirty: usize,
	/// Whether the stored configuration existed when first loaded.
	config_was_stored: bool,
	/// The host's gas reading below which the current work item must not go: what is under
	/// it belongs to the other items accumulated in the same invocation.
	pub gas_floor: u64,
}

/// A stored record failed to decode. State written by this code always decodes, so this
/// means corruption; stopping is the only safe answer.
fn corrupt() -> ! {
	panic!("jamx: corrupt state record")
}

impl<'a, H: Host> Ctx<'a, H> {
	pub fn new(host: &'a mut H, service: u32, now: Slot) -> Self {
		Self {
			host,
			service,
			now,
			accounts: Vec::with_capacity(32),
			buckets: [NIL; BUCKETS],
			assets: BTreeMap::new(),
			config: None,
			raw: BTreeMap::new(),
			undo: Vec::with_capacity(32),
			scratch: Vec::with_capacity(512),
			epoch: 1,
			dirty: 0,
			config_was_stored: false,
			gas_floor: 0,
		}
	}

	/// Gas the current work item may still spend.
	pub fn gas_left(&mut self) -> u64 {
		self.host.gas().saturating_sub(self.gas_floor)
	}

	// ---- journal ----

	/// Start an instruction.
	pub fn begin(&mut self) {
		self.epoch += 1;
		self.undo.clear();
	}

	/// Undo everything since [`Ctx::begin`].
	pub fn rollback(&mut self) {
		while let Some(u) = self.undo.pop() {
			match u {
				Undo::Account(ix, val, dirty) => {
					let e = &mut self.accounts[ix];
					e.val = val;
					e.dirty = dirty;
					e.epoch = 0;
				},
				Undo::Asset(id, c) => {
					self.assets.insert(id, *c);
				},
				Undo::Config(c) => self.config = Some(*c),
				Undo::Raw(k, c) => {
					self.raw.insert(k, c);
				},
			}
		}
		self.dirty = self.accounts.iter().filter(|c| c.dirty).count() +
			self.assets.values().filter(|c| c.dirty).count() +
			self.raw.values().filter(|c| c.dirty).count() +
			self.config.as_ref().is_some_and(|c| c.dirty) as usize;
	}

	// ---- accounts ----

	/// Position of an account in the cache, loading it from the host on first use.
	pub fn account_ix(&mut self, id: &AccountId) -> usize {
		let b = id[0] as usize;
		let mut i = self.buckets[b];
		while i != NIL {
			let e = &self.accounts[i as usize];
			if &e.id == id {
				return i as usize;
			}
			i = e.next;
		}
		let mut k = [0u8; 33];
		k[0] = b'a';
		k[1..].copy_from_slice(id);
		let val = if self.host.read(&k, &mut self.scratch) {
			Account::from_bytes(&self.scratch).unwrap_or_else(|_| corrupt())
		} else {
			Account::default()
		};
		let ix = self.accounts.len();
		self.accounts.push(AcctEntry { id: *id, val, dirty: false, epoch: 0, next: self.buckets[b] });
		self.buckets[b] = ix as u32;
		ix
	}

	/// Make room for `n` more accounts and journal entries, so that a large batch does not
	/// pay for the cache growing under it.
	pub fn reserve(&mut self, n: usize) {
		self.accounts.reserve(n);
		self.undo.reserve(n);
	}

	/// How many accounts are cached; positions run from zero to this.
	pub fn account_count(&self) -> usize {
		self.accounts.len()
	}

	pub fn acct(&self, ix: usize) -> &Account {
		&self.accounts[ix].val
	}

	/// Mutable access by position. The first change in each instruction is journaled.
	pub fn acct_mut(&mut self, ix: usize) -> &mut Account {
		let epoch = self.epoch;
		let e = &mut self.accounts[ix];
		if e.epoch != epoch {
			self.undo.push(Undo::Account(ix, e.val.clone(), e.dirty));
			e.epoch = epoch;
		}
		if !e.dirty {
			e.dirty = true;
			self.dirty += 1;
		}
		&mut e.val
	}

	pub fn account(&mut self, id: &AccountId) -> &Account {
		let ix = self.account_ix(id);
		&self.accounts[ix].val
	}

	pub fn account_mut(&mut self, id: &AccountId) -> &mut Account {
		let ix = self.account_ix(id);
		self.acct_mut(ix)
	}

	// ---- assets ----

	fn load_asset(&mut self, id: AssetId) {
		if !self.assets.contains_key(&id) {
			let val = self
				.host
				.read(&key::asset(id), &mut self.scratch)
				.then(|| Asset::from_bytes(&self.scratch).unwrap_or_else(|_| corrupt()));
			self.assets.insert(id, Cached { val, dirty: false, epoch: 0 });
		}
	}

	pub fn asset(&mut self, id: AssetId) -> Option<&Asset> {
		self.load_asset(id);
		self.assets[&id].val.as_ref()
	}

	fn asset_slot(&mut self, id: AssetId) -> &mut Cached<Option<Asset>> {
		self.load_asset(id);
		let epoch = self.epoch;
		let Some(c) = self.assets.get_mut(&id) else { corrupt() };
		if c.epoch != epoch {
			self.undo.push(Undo::Asset(id, Box::new(c.clone())));
			c.epoch = epoch;
		}
		if !c.dirty {
			c.dirty = true;
			self.dirty += 1;
		}
		c
	}

	pub fn asset_mut(&mut self, id: AssetId) -> Option<&mut Asset> {
		if self.asset(id).is_none() {
			return None;
		}
		self.asset_slot(id).val.as_mut()
	}

	pub fn asset_insert(&mut self, id: AssetId, asset: Asset) {
		self.asset_slot(id).val = Some(asset);
	}

	// ---- configuration ----

	fn load_config(&mut self) {
		if self.config.is_none() {
			self.config_was_stored = self.host.read(key::CONFIG, &mut self.scratch);
			let val = if self.config_was_stored {
				Config::from_bytes(&self.scratch).unwrap_or_else(|_| corrupt())
			} else {
				Config::default()
			};
			self.config = Some(Cached { val, dirty: false, epoch: 0 });
		}
	}

	/// `true` if the service has never stored its configuration: this is its first run.
	pub fn is_fresh(&mut self) -> bool {
		self.load_config();
		!self.config_was_stored
	}

	pub fn config(&mut self) -> &Config {
		self.load_config();
		match &self.config {
			Some(c) => &c.val,
			None => corrupt(),
		}
	}

	pub fn config_mut(&mut self) -> &mut Config {
		self.load_config();
		let epoch = self.epoch;
		let Some(c) = self.config.as_mut() else { corrupt() };
		if c.epoch != epoch {
			self.undo.push(Undo::Config(Box::new(c.clone())));
			c.epoch = epoch;
		}
		if !c.dirty {
			c.dirty = true;
			self.dirty += 1;
		}
		&mut c.val
	}

	// ---- everything else ----

	fn load_raw(&mut self, k: &[u8]) {
		if !self.raw.contains_key(k) {
			let val = self.host.read(k, &mut self.scratch).then(|| self.scratch.clone());
			self.raw.insert(k.to_vec(), Cached { val, dirty: false, epoch: 0 });
		}
	}

	pub fn get_raw(&mut self, k: &[u8]) -> Option<&[u8]> {
		self.load_raw(k);
		self.raw[k].val.as_deref()
	}

	pub fn exists(&mut self, k: &[u8]) -> bool {
		self.get_raw(k).is_some()
	}

	pub fn get<T: Codec>(&mut self, k: &[u8]) -> Option<T> {
		self.get_raw(k).map(|b| T::from_bytes(b).unwrap_or_else(|_| corrupt()))
	}

	fn set_raw(&mut self, k: &[u8], v: Option<Vec<u8>>) {
		self.load_raw(k);
		let epoch = self.epoch;
		let Some(c) = self.raw.get_mut(k) else { corrupt() };
		if c.epoch != epoch {
			self.undo.push(Undo::Raw(k.to_vec(), c.clone()));
			c.epoch = epoch;
		}
		if !c.dirty {
			c.dirty = true;
			self.dirty += 1;
		}
		c.val = v;
	}

	pub fn put_raw(&mut self, k: &[u8], v: Vec<u8>) {
		self.set_raw(k, Some(v));
	}

	pub fn put<T: Codec>(&mut self, k: &[u8], v: &T) {
		self.set_raw(k, Some(v.to_bytes()));
	}

	pub fn del(&mut self, k: &[u8]) {
		self.set_raw(k, None);
	}

	// ---- flushing ----

	/// Number of records that the next flush will write.
	/// How many records the current instruction has changed so far (what a rollback would
	/// have to restore).
	pub fn undo_len(&self) -> usize {
		self.undo.len()
	}

	pub fn dirty_count(&self) -> usize {
		self.dirty
	}

	/// Write every changed record to the host. Returns `false` if the host refused a write
	/// (storage deposit not covered); the caller must then abort the invocation so that the
	/// host discards the partial writes.
	#[must_use]
	pub fn flush(&mut self) -> bool {
		self.undo.clear();
		self.dirty = 0;
		let mut k = [0u8; 33];
		k[0] = b'a';
		let mut buf = Vec::with_capacity(256);
		for c in self.accounts.iter_mut().filter(|c| c.dirty) {
			k[1..].copy_from_slice(&c.id);
			if c.val.is_empty() {
				self.host.remove(&k);
			} else {
				buf.clear();
				c.val.enc(&mut buf);
				if !self.host.write(&k, &buf) {
					return false;
				}
			}
			c.dirty = false;
		}
		for (id, c) in self.assets.iter_mut().filter(|(_, c)| c.dirty) {
			let k = key::asset(*id);
			match &c.val {
				None => self.host.remove(&k),
				Some(a) =>
					if !self.host.write(&k, &a.to_bytes()) {
						return false;
					},
			}
			c.dirty = false;
		}
		for (k, c) in self.raw.iter_mut().filter(|(_, c)| c.dirty) {
			match &c.val {
				None => self.host.remove(k),
				Some(v) =>
					if !self.host.write(k, v) {
						return false;
					},
			}
			c.dirty = false;
		}
		if let Some(c) = self.config.as_mut().filter(|c| c.dirty) {
			if !self.host.write(key::CONFIG, &c.val.to_bytes()) {
				return false;
			}
			c.dirty = false;
			self.config_was_stored = true;
		}
		true
	}

	// ---- money movement with the rules attached ----

	/// Take `amount` of `asset` out of `id`'s free balance, enforcing the account freeze,
	/// the issuer freeze and the account's own daily limit.
	pub fn debit(&mut self, id: &AccountId, asset: AssetId, amount: u64) -> Res {
		let now = self.now;
		let a = self.account_mut(id);
		if a.is_frozen(now) || a.bal_flags(asset) & BAL_FROZEN != 0 {
			return Err(Status::Frozen);
		}
		if let Some(p) = a.policy.as_mut() {
			p.spend(asset, amount, now)?;
		}
		a.debit(asset, amount)
	}

	/// Send `amount` of `asset` to `id`. The account must have accepted the asset (nobody
	/// but its owner adds entries to its record), and the asset's allowlist, if it has one,
	/// must include it.
	pub fn credit(&mut self, id: &AccountId, asset: AssetId, amount: u64) -> Res {
		let permissioned =
			self.asset(asset).ok_or(Status::NotFound)?.flags & ASSET_PERMISSIONED != 0;
		match self.account(id).bal(asset) {
			Some(b) if !permissioned || b.flags & BAL_ALLOWED != 0 => {},
			_ => return Err(Status::NotAllowed),
		}
		self.account_mut(id).credit(asset, amount)
	}

	/// Return funds to an account that held them before (refunds, releases). Not subject
	/// to the allowlist: money that was lawfully locked must always be able to come back.
	pub fn credit_back(&mut self, id: &AccountId, asset: AssetId, amount: u64) -> Res {
		self.account_mut(id).credit_back(asset, amount)
	}
}

/// An in-memory host for tests and tools.
#[cfg(feature = "std")]
pub mod mock {
	use super::*;
	use std::collections::BTreeMap;

	#[derive(Default, Clone)]
	pub struct MockHost {
		pub store: BTreeMap<Vec<u8>, Vec<u8>>,
		/// State as of the last checkpoint.
		pub committed: BTreeMap<Vec<u8>, Vec<u8>>,
		pub reads: u64,
		pub writes: u64,
		/// Refuse writes once the store holds this many bytes.
		pub capacity: Option<usize>,
		pub upgraded_to: Option<(Hash32, u64, u64)>,
		pub solicited: Vec<(Hash32, u32)>,
		pub preimages: Vec<Hash32>,
		pub logs: Vec<String>,
	}

	impl MockHost {
		pub fn bytes(&self) -> usize {
			self.store.iter().map(|(k, v)| k.len() + v.len()).sum()
		}
	}

	impl Host for MockHost {
		fn read(&mut self, key: &[u8], buf: &mut Vec<u8>) -> bool {
			self.reads += 1;
			buf.clear();
			match self.store.get(key) {
				Some(v) => {
					buf.extend_from_slice(v);
					true
				},
				None => false,
			}
		}
		fn write(&mut self, key: &[u8], value: &[u8]) -> bool {
			if let Some(cap) = self.capacity {
				if self.bytes() + key.len() + value.len() > cap {
					return false;
				}
			}
			self.writes += 1;
			self.store.insert(key.to_vec(), value.to_vec());
			true
		}
		fn remove(&mut self, key: &[u8]) {
			self.store.remove(key);
		}
		fn gas(&mut self) -> u64 {
			u64::MAX
		}
		fn checkpoint(&mut self) {
			self.committed = self.store.clone();
		}
		fn upgrade(&mut self, code_hash: &Hash32, min_item_gas: u64, min_memo_gas: u64) {
			self.upgraded_to = Some((*code_hash, min_item_gas, min_memo_gas));
		}
		fn solicit(&mut self, hash: &Hash32, len: u32) -> bool {
			self.solicited.push((*hash, len));
			true
		}
		fn preimage_available(&mut self, hash: &Hash32) -> bool {
			self.preimages.contains(hash)
		}
		fn log(&mut self, msg: &str) {
			self.logs.push(msg.into());
		}
	}
}

#[cfg(test)]
mod tests {
	use super::{mock::MockHost, *};

	#[test]
	fn journal_rolls_back_every_kind_of_record() {
		let mut host = MockHost::default();
		let mut ctx = Ctx::new(&mut host, 1, 10);
		let a = [1u8; 32];

		ctx.begin();
		ctx.account_mut(&a).credit(1, 100).unwrap();
		ctx.put_raw(b"x", vec![1]);
		ctx.config_mut().next_deal = 7;
		assert!(ctx.flush());

		ctx.begin();
		ctx.account_mut(&a).debit(1, 60).unwrap();
		ctx.account_mut(&a).credit(2, 5).unwrap();
		ctx.put_raw(b"x", vec![2]);
		ctx.put_raw(b"y", vec![3]);
		ctx.del(b"x");
		ctx.config_mut().next_deal = 9;
		ctx.rollback();

		assert_eq!(ctx.account(&a).free(1), 100);
		assert_eq!(ctx.account(&a).free(2), 0);
		assert_eq!(ctx.get_raw(b"x"), Some(&[1u8][..]));
		assert_eq!(ctx.get_raw(b"y"), None);
		assert_eq!(ctx.config().next_deal, 7);
		assert!(ctx.flush());
		assert_eq!(host.store.get(&b"x"[..]), Some(&vec![1]));
		assert!(!host.store.contains_key(&b"y"[..]));
	}

	#[test]
	fn records_are_read_and_written_once() {
		let mut host = MockHost::default();
		let a = [1u8; 32];
		{
			let mut ctx = Ctx::new(&mut host, 1, 10);
			for _ in 0..50 {
				ctx.begin();
				ctx.account_mut(&a).credit(1, 1).unwrap();
			}
			assert_eq!(ctx.dirty_count(), 1);
			assert!(ctx.flush());
			assert_eq!(ctx.dirty_count(), 0);
		}
		assert_eq!((host.reads, host.writes), (1, 1));
	}

	#[test]
	fn empty_accounts_are_deleted() {
		let mut host = MockHost::default();
		let a = [1u8; 32];
		let mut ctx = Ctx::new(&mut host, 1, 10);
		ctx.account_mut(&a).credit(1, 5).unwrap();
		assert!(ctx.flush());
		// An emptied balance stays: it records that the account accepts the asset.
		ctx.account_mut(&a).debit(1, 5).unwrap();
		assert!(ctx.flush());
		assert_eq!(ctx.host.store.len(), 1);
		// Once the account stops accepting it, nothing is left and the record goes.
		ctx.account_mut(&a).unaccept(1).unwrap();
		assert!(ctx.flush());
		assert!(host.store.is_empty());
	}

	#[test]
	fn refused_write_is_reported() {
		let mut host = MockHost { capacity: Some(10), ..Default::default() };
		let mut ctx = Ctx::new(&mut host, 1, 10);
		ctx.account_mut(&[1; 32]).credit(1, 5).unwrap();
		assert!(!ctx.flush());
	}

	#[test]
	fn debit_enforces_freezes_and_limits() {
		let mut host = MockHost::default();
		let mut ctx = Ctx::new(&mut host, 1, 100);
		let a = [1u8; 32];
		ctx.account_mut(&a).credit(1, 1000).unwrap();
		ctx.debit(&a, 1, 10).unwrap();
		// Issuer freeze.
		ctx.account_mut(&a).bal_mut(1).unwrap().flags |= BAL_FROZEN;
		assert_eq!(ctx.debit(&a, 1, 10), Err(Status::Frozen));
		ctx.account_mut(&a).bal_mut(1).unwrap().flags = 0;
		// Daily limit.
		ctx.account_mut(&a).policy_mut(&a).apply(PolicyChange::SetLimit { asset: 1, per_day: Some(15) }, 100).unwrap();
		ctx.debit(&a, 1, 10).unwrap();
		assert_eq!(ctx.debit(&a, 1, 10), Err(Status::Limit));
		// Account freeze.
		ctx.account_mut(&a).policy_mut(&a).frozen_until = 200;
		assert_eq!(ctx.debit(&a, 1, 1), Err(Status::Frozen));
	}
}
