//! State records and their canonical encodings.
//!
//! Design rule: **everything one account owns lives in one storage item**. Host calls
//! dominate accumulate gas (a read or a write costs ~2.4k gas under GP 0.8.0), so a
//! settlement that touches an account costs one read and one write no matter how many
//! assets or orders are involved.

use crate::codec::{dec_vec, enc_vec, Codec, DecodeError, DecodeResult, Reader, Writer};
use alloc::{boxed::Box, vec::Vec};
use core::cmp::Ordering;

pub use crate::crypto::{Hash32, PubKey, Sig};

/// An account is identified forever by the Ed25519 key that first controlled it. The keys
/// allowed to act for it can change (second device, recovery); the identifier does not.
pub type AccountId = [u8; 32];
pub type AssetId = u32;
pub type Slot = u32;
pub type DealId = u64;

/// Slots per day at six seconds per slot.
pub const DAY_SLOTS: Slot = 14_400;
/// Default freeze length when a policy does not set one: seven days.
pub const DEFAULT_FREEZE_SLOTS: Slot = 7 * DAY_SLOTS;
/// Longest freeze a policy may ask for. The same figure is the guardian's cool-down, so an
/// unbounded value would let whoever holds a key switch the guardian off for good.
pub const MAX_FREEZE_SLOTS: Slot = 30 * DAY_SLOTS;
/// The security delay an account gets when it first adds a protection without choosing one.
/// A guardian, a limit or a recovery group that the owner key can undo at once protects
/// nothing against a thief who holds that key.
pub const DEFAULT_DELAY: Slot = DAY_SLOTS;
/// Seconds per slot, and the Unix time of slot zero (the JAM Common Era: 12:00 UTC on
/// 1 January 2025). Together they give accumulate a wall clock.
pub const SLOT_SECONDS: u64 = 6;
pub const JAM_EPOCH_UNIX: u64 = 1_735_732_800;

pub const MAX_KEYS: usize = 8;
pub const MAX_GUARDIANS: usize = 8;
pub const MAX_LIMITS: usize = 8;
pub const MAX_PENDING: usize = 4;
pub const MAX_OPEN_ORDERS: usize = 16;
/// Most assets an account may accept. Only the account itself adds entries (by accepting an
/// asset or by signing an order that buys it), so nobody else can make its record grow.
/// Refunds are exempt (see [`Account::credit_back`]).
pub const MAX_BALANCES: usize = 64;
pub const MAX_ATTESTORS: usize = 8;
pub const MAX_ORACLES: usize = 8;
pub const MAX_SCRIPT_LEN: usize = 64;
/// Width of the replay-protection window, in nonces.
pub const NONCE_WINDOW: u64 = 256;
/// How far ahead of the window floor a nonce may jump.
pub const NONCE_MAX_JUMP: u64 = 1 << 20;

/// Result of applying one instruction. Stored in receipts so clients can see what happened.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Status {
	Ok = 0,
	Decode = 2,
	Unauthorized = 3,
	Nonce = 4,
	Frozen = 5,
	Insufficient = 6,
	Limit = 7,
	NotFound = 8,
	NotAllowed = 9,
	Cap = 10,
	Invalid = 11,
	Expired = 12,
	TooEarly = 13,
	Exists = 14,
	Overflow = 15,
	NoMatch = 16,
	Unconfirmed = 17,
	Mismatch = 18,
	OutOfGas = 19,
	/// The service has not yet seen enough of an outside chain to open deals on it.
	Stale = 20,
}

pub type Res<T = ()> = Result<T, Status>;

impl From<DecodeError> for Status {
	fn from(_: DecodeError) -> Self {
		Status::Decode
	}
}

// ---------------------------------------------------------------------------------------
// Prices
// ---------------------------------------------------------------------------------------

/// A price as an exact ratio: `num` atoms of the quote asset per `den` atoms of the base
/// asset. Ratios avoid any per-market tick or decimal configuration.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Price {
	pub num: u64,
	pub den: u64,
}

impl Price {
	pub fn new(num: u64, den: u64) -> Self {
		Self { num, den }
	}
	pub fn valid(&self) -> bool {
		self.num > 0 && self.den > 0
	}
	/// Total ordering by value (cross-multiplication in 128 bits; never overflows).
	pub fn cmp_value(&self, other: &Price) -> Ordering {
		(self.num as u128 * other.den as u128).cmp(&(other.num as u128 * self.den as u128))
	}
	pub fn same_value(&self, other: &Price) -> bool {
		self.cmp_value(other) == Ordering::Equal
	}
	/// `floor(base * num / den)`: what a seller of `base` receives.
	pub fn quote_floor(&self, base: u64) -> Option<u64> {
		mul_div_floor(base, self.num, self.den)
	}
	/// `ceil(base * num / den)`: what a buyer of `base` pays.
	pub fn quote_ceil(&self, base: u64) -> Option<u64> {
		match (base.checked_mul(self.num), self.den) {
			// The common case fits in 64 bits, where a division is one instruction.
			(Some(n), d) => Some(n / d + (n % d != 0) as u64),
			(None, d) => {
				let n = base as u128 * self.num as u128;
				u64::try_from((n + d as u128 - 1) / d as u128).ok()
			},
		}
	}
	/// The largest `base` such that `quote_ceil(base) <= quote`.
	pub fn base_for_quote(&self, quote: u64) -> u64 {
		mul_div_floor(quote, self.den, self.num).unwrap_or(u64::MAX)
	}
}

/// `floor(a * b / d)`, or `None` if it does not fit in 64 bits. `d` must not be zero.
pub fn mul_div_floor(a: u64, b: u64, d: u64) -> Option<u64> {
	match a.checked_mul(b) {
		Some(n) => Some(n / d),
		None => u64::try_from(a as u128 * b as u128 / d as u128).ok(),
	}
}

impl Codec for Price {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.num);
		out.put_var(self.den);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { num: r.var()?, den: r.var()? })
	}
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Side {
	/// Wants the base asset, pays the quote asset.
	Buy,
	/// Gives the base asset, wants the quote asset.
	Sell,
}

// ---------------------------------------------------------------------------------------
// Account
// ---------------------------------------------------------------------------------------

/// The balance may not be moved out: frozen by the asset's issuer (logged on-chain).
pub const BAL_FROZEN: u8 = 1;
/// The account passed the issuer's checks and may hold a permissioned asset.
pub const BAL_ALLOWED: u8 = 2;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Bal {
	pub asset: AssetId,
	pub free: u64,
	pub flags: u8,
}

/// A resting order that was partly filled in an earlier batch.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct OpenOrder {
	pub nonce: u64,
	/// First eight bytes of the order hash: binds the entry to one specific signed order.
	pub tag: [u8; 8],
	pub filled: u64,
	pub expiry: Slot,
}

/// May do anything the account can do.
pub const ROLE_OWNER: u8 = 0;
/// May place and cancel orders only (a phone, a trading bot).
pub const ROLE_TRADER: u8 = 1;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct KeyEntry {
	pub key: PubKey,
	pub role: u8,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoveryCfg {
	/// Accounts that can together install a new owner key.
	pub guardians: Vec<AccountId>,
	pub threshold: u8,
	/// Slots between reaching the threshold and the new key taking effect.
	pub delay: Slot,
}

/// One guardian's choice of replacement key.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Vote {
	/// Index into [`RecoveryCfg::guardians`].
	pub guardian: u8,
	pub key: PubKey,
}

/// A recovery in progress. Each guardian holds one vote, so a single guardian cannot
/// disturb the others' count.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct RecoveryReq {
	pub votes: Vec<Vote>,
	/// The key that reached the threshold. Meaningful once `eta` is set.
	pub new_owner: PubKey,
	/// Zero until the threshold is reached; then the slot at which the key takes effect.
	pub eta: Slot,
}

impl RecoveryReq {
	pub fn count(&self, key: &PubKey) -> usize {
		self.votes.iter().filter(|v| &v.key == key).count()
	}
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Limit {
	pub asset: AssetId,
	pub per_day: u64,
	pub spent: u64,
	pub window_start: Slot,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum PolicyChange {
	AddKey { key: PubKey, role: u8 },
	RemoveKey { key: PubKey },
	SetGuardian(Option<AccountId>),
	SetRecovery(Option<RecoveryCfg>),
	SetLimit { asset: AssetId, per_day: Option<u64> },
	SetDelay(Slot),
	SetFreezeSlots(Slot),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Pending {
	pub eta: Slot,
	pub change: PolicyChange,
}

/// The account's own rules. Absent for a plain account, which behaves as a single owner key
/// equal to the account identifier and no protections.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Policy {
	pub keys: Vec<KeyEntry>,
	/// "A trusted friend who can freeze but not spend."
	pub guardian: Option<AccountId>,
	pub recovery: Option<RecoveryCfg>,
	/// Delay applied to every change that weakens the account (see [`Policy::weakens`]).
	pub delay: Slot,
	/// How long a freeze lasts, and the cool-down before the next one.
	pub freeze_slots: Slot,
	/// The account is frozen while `now < frozen_until`.
	pub frozen_until: Slot,
	pub limits: Vec<Limit>,
	pub pending: Vec<Pending>,
	pub recovery_req: Option<RecoveryReq>,
}

impl Policy {
	pub fn initial(id: &AccountId) -> Self {
		Self {
			keys: alloc::vec![KeyEntry { key: *id, role: ROLE_OWNER }],
			guardian: None,
			recovery: None,
			delay: 0,
			freeze_slots: DEFAULT_FREEZE_SLOTS,
			frozen_until: 0,
			limits: Vec::new(),
			pending: Vec::new(),
			recovery_req: None,
		}
	}

	pub fn is_frozen(&self, now: Slot) -> bool {
		now < self.frozen_until
	}

	pub fn limit(&self, asset: AssetId) -> Option<&Limit> {
		self.limits.iter().find(|l| l.asset == asset)
	}

	/// Does this change make the account easier to take over or drain? Such changes wait out
	/// the security delay, during which a guardian can freeze and the owner can cancel.
	pub fn weakens(&self, c: &PolicyChange) -> bool {
		match c {
			PolicyChange::AddKey { .. } | PolicyChange::RemoveKey { .. } => true,
			PolicyChange::SetGuardian(_) => self.guardian.is_some(),
			// Naming who may install a new owner key is a way into the account, and removing
			// them takes away the owner's way back: neither is ever harmless.
			PolicyChange::SetRecovery(_) => true,
			PolicyChange::SetLimit { asset, per_day } => match (self.limit(*asset), per_day) {
				(None, _) => false,
				(Some(cur), Some(new)) => *new > cur.per_day,
				(Some(_), None) => true,
			},
			PolicyChange::SetDelay(d) => *d < self.delay,
			// A shorter freeze gives a thief less to wait out; a longer one keeps the guardian
			// in its cool-down for longer. Either direction can serve an attacker.
			PolicyChange::SetFreezeSlots(s) => *s != self.freeze_slots,
		}
	}

	/// A first protection on an account with no security delay brings the default delay
	/// with it. The owner can choose another value, but lowering it is itself delayed.
	fn default_delay(&mut self) {
		if self.delay == 0 {
			self.delay = DEFAULT_DELAY.min(self.freeze_slots - 1);
		}
	}

	/// Apply a change now. Validity is checked here; whether it had to wait is the caller's job.
	pub fn apply(&mut self, c: PolicyChange, now: Slot) -> Res {
		match c {
			PolicyChange::AddKey { key, role } => {
				if role > ROLE_TRADER || self.keys.len() >= MAX_KEYS {
					return Err(Status::Invalid);
				}
				if self.keys.iter().any(|k| k.key == key) {
					return Err(Status::Exists);
				}
				self.keys.push(KeyEntry { key, role });
			},
			PolicyChange::RemoveKey { key } => {
				let Some(i) = self.keys.iter().position(|k| k.key == key) else {
					return Err(Status::NotFound);
				};
				let owners = self.keys.iter().filter(|k| k.role == ROLE_OWNER).count();
				if self.keys[i].role == ROLE_OWNER && owners <= 1 {
					return Err(Status::Invalid);
				}
				self.keys.remove(i);
			},
			PolicyChange::SetGuardian(g) => {
				if g.is_some() {
					self.default_delay();
				}
				self.guardian = g;
			},
			PolicyChange::SetRecovery(r) => {
				if let Some(cfg) = &r {
					let n = cfg.guardians.len();
					if n == 0 || n > MAX_GUARDIANS || cfg.threshold == 0 || cfg.threshold as usize > n {
						return Err(Status::Invalid);
					}
					for (i, g) in cfg.guardians.iter().enumerate() {
						if cfg.guardians[..i].contains(g) {
							return Err(Status::Invalid);
						}
					}
				}
				if r.is_some() {
					self.default_delay();
				}
				self.recovery = r;
				self.recovery_req = None;
			},
			PolicyChange::SetLimit { asset, per_day } => {
				let pos = self.limits.iter().position(|l| l.asset == asset);
				match (pos, per_day) {
					(Some(i), Some(v)) => self.limits[i].per_day = v,
					(Some(i), None) => {
						self.limits.remove(i);
					},
					(None, Some(v)) => {
						if self.limits.len() >= MAX_LIMITS {
							return Err(Status::Invalid);
						}
						self.default_delay();
						self.limits.push(Limit { asset, per_day: v, spent: 0, window_start: now });
					},
					(None, None) => {},
				}
			},
			// The delay must be shorter than the guardian's cool-down. Otherwise a hostile
			// guardian could freeze again before its own removal had waited out the delay,
			// and the owner could never be rid of it.
			PolicyChange::SetDelay(d) => {
				if d >= self.freeze_slots {
					return Err(Status::Invalid);
				}
				self.delay = d;
			},
			PolicyChange::SetFreezeSlots(s) => {
				if s == 0 || s > MAX_FREEZE_SLOTS || s <= self.delay {
					return Err(Status::Invalid);
				}
				self.freeze_slots = s;
			},
		}
		Ok(())
	}

	/// Record an outflow against the daily limit of `asset`, if there is one.
	pub fn spend(&mut self, asset: AssetId, amount: u64, now: Slot) -> Res {
		let Some(l) = self.limits.iter_mut().find(|l| l.asset == asset) else { return Ok(()) };
		if now >= l.window_start.saturating_add(DAY_SLOTS) {
			l.window_start = now;
			l.spent = 0;
		}
		let total = l.spent.checked_add(amount).ok_or(Status::Limit)?;
		if total > l.per_day {
			return Err(Status::Limit);
		}
		l.spent = total;
		Ok(())
	}

	/// How much of `asset` may still leave today.
	pub fn spendable(&self, asset: AssetId, now: Slot) -> u64 {
		match self.limit(asset) {
			None => u64::MAX,
			Some(l) if now >= l.window_start.saturating_add(DAY_SLOTS) => l.per_day,
			Some(l) => l.per_day.saturating_sub(l.spent),
		}
	}
}

/// What a signer is trying to do, for authorization purposes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Perm {
	/// Place or cancel orders.
	Trade,
	/// Anything else.
	Full,
}

#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Account {
	/// Nonces below this are spent.
	pub nonce_floor: u64,
	/// Bit `i`: nonce `nonce_floor + i` is spent.
	pub used: [u64; 4],
	/// Sorted by asset.
	pub balances: Vec<Bal>,
	pub open: Vec<OpenOrder>,
	/// Boxed: most accounts have none, and the record is moved around a great deal.
	pub policy: Option<Box<Policy>>,
}

impl Account {
	/// Nothing worth storing: the record can be deleted.
	pub fn is_empty(&self) -> bool {
		self.nonce_floor == 0 &&
			self.used == [0; 4] &&
			self.balances.is_empty() &&
			self.open.is_empty() &&
			self.policy.is_none()
	}

	pub fn authorized(&self, id: &AccountId, signer: &PubKey, perm: Perm) -> bool {
		match &self.policy {
			None => signer == id,
			Some(p) => p
				.keys
				.iter()
				.any(|k| &k.key == signer && (k.role == ROLE_OWNER || perm == Perm::Trade)),
		}
	}

	pub fn is_frozen(&self, now: Slot) -> bool {
		self.policy.as_ref().is_some_and(|p| p.is_frozen(now))
	}

	pub fn policy_mut(&mut self, id: &AccountId) -> &mut Policy {
		self.policy.get_or_insert_with(|| Box::new(Policy::initial(id)))
	}

	// ---- nonces ----

	fn bit(&self, off: u64) -> bool {
		self.used[(off / 64) as usize] >> (off % 64) & 1 == 1
	}

	pub fn nonce_usable(&self, n: u64) -> bool {
		if n < self.nonce_floor {
			return false;
		}
		let off = n - self.nonce_floor;
		off < NONCE_MAX_JUMP && (off >= NONCE_WINDOW || !self.bit(off))
	}

	/// Mark a nonce spent. Jumping past the window slides it, which spends every unused
	/// nonce that falls out of the bottom.
	pub fn nonce_use(&mut self, n: u64) -> Res {
		if !self.nonce_usable(n) {
			return Err(Status::Nonce);
		}
		let off = n - self.nonce_floor;
		if off >= NONCE_WINDOW {
			self.slide(off - (NONCE_WINDOW - 1));
		}
		let off = n - self.nonce_floor;
		self.used[(off / 64) as usize] |= 1 << (off % 64);
		Ok(())
	}

	/// Raise the floor by `by`, spending everything below the new floor.
	pub fn slide(&mut self, by: u64) {
		self.nonce_floor = self.nonce_floor.saturating_add(by);
		if by >= NONCE_WINDOW {
			self.used = [0; 4];
		} else {
			let words = (by / 64) as usize;
			let bits = (by % 64) as u32;
			let mut out = [0u64; 4];
			for i in 0..4 - words {
				let lo = self.used[i + words] >> bits;
				let hi = if bits > 0 && i + words + 1 < 4 {
					self.used[i + words + 1] << (64 - bits)
				} else {
					0
				};
				out[i] = lo | hi;
			}
			self.used = out;
		}
		let floor = self.nonce_floor;
		self.open.retain(|o| o.nonce >= floor);
	}

	// ---- balances ----

	pub fn bal(&self, asset: AssetId) -> Option<&Bal> {
		self.balances.binary_search_by_key(&asset, |b| b.asset).ok().map(|i| &self.balances[i])
	}

	fn bal_slot(&mut self, asset: AssetId, capped: bool) -> Res<&mut Bal> {
		match self.balances.binary_search_by_key(&asset, |b| b.asset) {
			Ok(i) => Ok(&mut self.balances[i]),
			Err(i) => {
				if capped && self.balances.len() >= MAX_BALANCES {
					return Err(Status::Invalid);
				}
				self.balances.insert(i, Bal { asset, free: 0, flags: 0 });
				Ok(&mut self.balances[i])
			},
		}
	}

	/// The entry for `asset`, added if the account has room for one more. Adding an entry is
	/// the account's own act: callers reach this only on the strength of its signature.
	pub fn bal_mut(&mut self, asset: AssetId) -> Res<&mut Bal> {
		self.bal_slot(asset, true)
	}

	/// The entry for `asset` if the account has accepted it.
	pub fn bal_existing(&mut self, asset: AssetId) -> Option<&mut Bal> {
		self.balances.binary_search_by_key(&asset, |b| b.asset).ok().map(|i| &mut self.balances[i])
	}

	/// Has the account accepted `asset`?
	pub fn accepts(&self, asset: AssetId) -> bool {
		self.bal(asset).is_some()
	}

	/// Can this account take delivery of `asset` by its own choice (it has accepted it, or
	/// has room to)?
	pub fn can_hold(&self, asset: AssetId) -> bool {
		self.balances.len() < MAX_BALANCES || self.bal(asset).is_some()
	}

	/// Stop accepting `asset`. Only an empty, unfrozen entry can go: a balance cannot be
	/// thrown away by mistake, and an issuer's freeze cannot be shed by leaving and returning.
	pub fn unaccept(&mut self, asset: AssetId) -> Res {
		let i = self.balances.binary_search_by_key(&asset, |b| b.asset).map_err(|_| Status::NotFound)?;
		let b = &self.balances[i];
		if b.free != 0 {
			return Err(Status::Invalid);
		}
		if b.flags & BAL_FROZEN != 0 {
			return Err(Status::Frozen);
		}
		self.balances.remove(i);
		Ok(())
	}

	pub fn free(&self, asset: AssetId) -> u64 {
		self.bal(asset).map_or(0, |b| b.free)
	}

	pub fn bal_flags(&self, asset: AssetId) -> u8 {
		self.bal(asset).map_or(0, |b| b.flags)
	}

	/// Add to the balance, accepting the asset if need be. For amounts the account asked
	/// for (the proceeds of its own order). What others send goes through
	/// [`crate::state::Ctx::credit`], which never adds an entry.
	pub fn credit(&mut self, asset: AssetId, amount: u64) -> Res {
		let b = self.bal_mut(asset)?;
		b.free = b.free.checked_add(amount).ok_or(Status::Overflow)?;
		Ok(())
	}

	/// Return funds the account held before. Exempt from the cap on the number of assets:
	/// money that was locked must always be able to come back.
	pub fn credit_back(&mut self, asset: AssetId, amount: u64) -> Res {
		let b = self.bal_slot(asset, false)?;
		b.free = b.free.checked_add(amount).ok_or(Status::Overflow)?;
		Ok(())
	}

	pub fn debit(&mut self, asset: AssetId, amount: u64) -> Res {
		if amount == 0 {
			return Ok(());
		}
		let b = self.bal_existing(asset).ok_or(Status::Insufficient)?;
		b.free = b.free.checked_sub(amount).ok_or(Status::Insufficient)?;
		Ok(())
	}

	pub fn open_order(&self, nonce: u64) -> Option<&OpenOrder> {
		self.open.iter().find(|o| o.nonce == nonce)
	}

	pub fn drop_expired_orders(&mut self, now: Slot) {
		self.open.retain(|o| o.expiry >= now);
	}
}

// ---- encodings ----

impl Codec for Bal {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.asset as u64);
		out.put_var(self.free);
		out.put_u8(self.flags);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { asset: r.var_u32()?, free: r.var()?, flags: r.u8()? })
	}
}

impl Codec for OpenOrder {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.nonce);
		out.put(&self.tag);
		out.put_var(self.filled);
		out.put_u32(self.expiry);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { nonce: r.var()?, tag: r.array()?, filled: r.var()?, expiry: r.u32()? })
	}
}

impl Codec for KeyEntry {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.key);
		out.put_u8(self.role);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { key: r.array()?, role: r.u8()? })
	}
}

impl Codec for RecoveryCfg {
	fn enc(&self, out: &mut Vec<u8>) {
		enc_vec(&self.guardians, out);
		out.put_u8(self.threshold);
		out.put_u32(self.delay);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { guardians: dec_vec(r, MAX_GUARDIANS)?, threshold: r.u8()?, delay: r.u32()? })
	}
}

impl Codec for Vote {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_u8(self.guardian);
		out.put(&self.key);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { guardian: r.u8()?, key: r.array()? })
	}
}

impl Codec for RecoveryReq {
	fn enc(&self, out: &mut Vec<u8>) {
		enc_vec(&self.votes, out);
		out.put(&self.new_owner);
		out.put_u32(self.eta);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { votes: dec_vec(r, MAX_GUARDIANS)?, new_owner: r.array()?, eta: r.u32()? })
	}
}

impl Codec for Limit {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.asset as u64);
		out.put_var(self.per_day);
		out.put_var(self.spent);
		out.put_u32(self.window_start);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { asset: r.var_u32()?, per_day: r.var()?, spent: r.var()?, window_start: r.u32()? })
	}
}

pub fn enc_opt<T: Codec>(v: &Option<T>, out: &mut Vec<u8>) {
	match v {
		None => out.put_u8(0),
		Some(x) => {
			out.put_u8(1);
			x.enc(out);
		},
	}
}

pub fn dec_opt<T: Codec>(r: &mut Reader) -> DecodeResult<Option<T>> {
	Ok(if r.bool()? { Some(T::dec(r)?) } else { None })
}

impl Codec for PolicyChange {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			PolicyChange::AddKey { key, role } => {
				out.put_u8(0);
				out.put(key);
				out.put_u8(*role);
			},
			PolicyChange::RemoveKey { key } => {
				out.put_u8(1);
				out.put(key);
			},
			PolicyChange::SetGuardian(g) => {
				out.put_u8(2);
				enc_opt(g, out);
			},
			PolicyChange::SetRecovery(c) => {
				out.put_u8(3);
				enc_opt(c, out);
			},
			PolicyChange::SetLimit { asset, per_day } => {
				out.put_u8(4);
				out.put_var(*asset as u64);
				enc_opt(per_day, out);
			},
			PolicyChange::SetDelay(d) => {
				out.put_u8(5);
				out.put_u32(*d);
			},
			PolicyChange::SetFreezeSlots(s) => {
				out.put_u8(6);
				out.put_u32(*s);
			},
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			0 => PolicyChange::AddKey { key: r.array()?, role: r.u8()? },
			1 => PolicyChange::RemoveKey { key: r.array()? },
			2 => PolicyChange::SetGuardian(dec_opt(r)?),
			3 => PolicyChange::SetRecovery(dec_opt(r)?),
			4 => PolicyChange::SetLimit { asset: r.var_u32()?, per_day: dec_opt(r)? },
			5 => PolicyChange::SetDelay(r.u32()?),
			6 => PolicyChange::SetFreezeSlots(r.u32()?),
			_ => return Err(DecodeError),
		})
	}
}

impl Codec for Pending {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_u32(self.eta);
		self.change.enc(out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { eta: r.u32()?, change: PolicyChange::dec(r)? })
	}
}

impl Codec for Policy {
	fn enc(&self, out: &mut Vec<u8>) {
		enc_vec(&self.keys, out);
		enc_opt(&self.guardian, out);
		enc_opt(&self.recovery, out);
		out.put_u32(self.delay);
		out.put_u32(self.freeze_slots);
		out.put_u32(self.frozen_until);
		enc_vec(&self.limits, out);
		enc_vec(&self.pending, out);
		enc_opt(&self.recovery_req, out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			keys: dec_vec(r, MAX_KEYS)?,
			guardian: dec_opt(r)?,
			recovery: dec_opt(r)?,
			delay: r.u32()?,
			freeze_slots: r.u32()?,
			frozen_until: r.u32()?,
			limits: dec_vec(r, MAX_LIMITS)?,
			pending: dec_vec(r, MAX_PENDING)?,
			recovery_req: dec_opt(r)?,
		})
	}
}

impl<T: Codec> Codec for Box<T> {
	fn enc(&self, out: &mut Vec<u8>) {
		(**self).enc(out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		T::dec(r).map(Box::new)
	}
}

impl Codec for Account {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.nonce_floor);
		for w in self.used {
			out.put_var(w);
		}
		// Empty entries are kept: they record which assets the account accepts.
		out.put_var(self.balances.len() as u64);
		for b in &self.balances {
			b.enc(out);
		}
		enc_vec(&self.open, out);
		enc_opt(&self.policy, out);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		let nonce_floor = r.var()?;
		let used = [r.var()?, r.var()?, r.var()?, r.var()?];
		// A trade usually adds one asset to the account: allocate room for it now, so the
		// list does not have to be reallocated when it arrives.
		let n = r.var_usize()?;
		if n > u16::MAX as usize || n > r.remaining() {
			return Err(DecodeError);
		}
		let mut balances: Vec<Bal> = Vec::with_capacity(n + 2);
		for _ in 0..n {
			balances.push(Bal::dec(r)?);
		}
		if balances.windows(2).any(|w| w[0].asset >= w[1].asset) {
			return Err(DecodeError);
		}
		Ok(Self { nonce_floor, used, balances, open: dec_vec(r, MAX_OPEN_ORDERS)?, policy: dec_opt(r)? })
	}
}

// ---------------------------------------------------------------------------------------
// Assets
// ---------------------------------------------------------------------------------------

/// Only accounts the issuer has allowed may hold the asset (a bank's token carries the
/// bank's identity checks with it).
pub const ASSET_PERMISSIONED: u8 = 1;
/// The issuer may freeze a holder's balance. Every use is appended to a public log.
pub const ASSET_FREEZABLE: u8 = 2;

/// Parameters of a Bitcoin-style proof-of-work header chain.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BtcParams {
	/// Easiest allowed target, in compact form (`0x1d00ffff` on mainnet).
	pub pow_limit_bits: u32,
	/// Difficulty retargets every 2016 blocks (off on regtest).
	pub retarget: bool,
}

/// The header the light client starts from. It is public and anyone can check it against
/// the real chain before trusting an asset built on it.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BtcCheckpoint {
	pub height: u32,
	pub hash: Hash32,
	pub time: u32,
	pub bits: u32,
	/// Timestamp of the first block of this block's difficulty period.
	pub period_start: u32,
	pub chainwork: u128,
}

/// How an asset that lives outside JAM proves that it moved.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Rail {
	/// Bitcoin-style: block headers plus a Merkle receipt, checked by arithmetic alone.
	///
	/// A deal's clock is the outside chain's own: its *median time past*, the median of the
	/// timestamps of a block and the ten before it, which is what Bitcoin itself uses for
	/// time locks. The seller has `pay_secs` from the moment the deal opens: the payment
	/// must be in a block whose median time past is no later than that. The deal can be
	/// closed as unpaid only once the block `confirmations + grace_blocks` below the best
	/// tip the service has been shown is past that moment by the same clock.
	///
	/// Median time past never goes backwards, so once it has passed the limit no valid
	/// payment can still be mined. And it only advances with headers: if nobody carries
	/// them in, no time passes for the deal, so a seller who paid in time cannot lose to
	/// silence. Whoever wants the refund must bring the very headers that prove a payment.
	BtcSpv { confirmations: u8, pay_secs: u32, grace_blocks: u16 },
	/// A named set of oracles or arbiters attests that an off-chain payment (for instance a
	/// Pix transfer) arrived. Their reach is one trade at a time.
	Oracle { oracles: Vec<AccountId>, threshold: u8 },
}

/// Terms shared by every deal on an external asset.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct ExternalCfg {
	pub rail: Rail,
	/// Deposit posted by the side that must deliver off-chain, in basis points of the trade.
	pub bond_bps: u16,
	/// Oracle rails: slots from the creation of a deal until it can be timed out in the
	/// buyer's favour. Bitcoin rails: slots after which a deal that is still open is
	/// cancelled with nobody penalised, as a way out should the header chain ever stall.
	pub claim_window: Slot,
	/// Smallest deal, in atoms of the external asset.
	pub min_deal: u64,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum AssetKind {
	/// Balances are records in this service.
	Token,
	/// The asset lives elsewhere and never enters JAM. It is only traded through deals.
	External(ExternalCfg),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Asset {
	pub issuer: AccountId,
	pub symbol: [u8; 8],
	pub decimals: u8,
	pub flags: u8,
	pub kind: AssetKind,
	pub supply: u64,
	/// Independent parties who attest to the reserves behind the asset. Empty: no mint cap.
	pub attestors: Vec<AccountId>,
	pub threshold: u8,
	/// Latest attested reserves per attestor, same order as `attestors`.
	pub attested: Vec<u64>,
	/// Number of entries in the public freeze log.
	pub freeze_log: u32,
}

impl Asset {
	/// The most that may exist: the largest figure at least `threshold` attestors vouch for.
	pub fn mint_cap(&self) -> u64 {
		if self.attestors.is_empty() {
			return u64::MAX;
		}
		let mut v = self.attested.clone();
		v.sort_unstable_by(|a, b| b.cmp(a));
		v.get((self.threshold as usize).saturating_sub(1)).copied().unwrap_or(0)
	}
	pub fn is_token(&self) -> bool {
		matches!(self.kind, AssetKind::Token)
	}
	pub fn external(&self) -> Option<&ExternalCfg> {
		match &self.kind {
			AssetKind::External(c) => Some(c),
			AssetKind::Token => None,
		}
	}
}

impl Codec for BtcParams {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_u32(self.pow_limit_bits);
		out.put_bool(self.retarget);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { pow_limit_bits: r.u32()?, retarget: r.bool()? })
	}
}

impl Codec for BtcCheckpoint {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_u32(self.height);
		out.put(&self.hash);
		out.put_u32(self.time);
		out.put_u32(self.bits);
		out.put_u32(self.period_start);
		out.put_u128(self.chainwork);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			height: r.u32()?,
			hash: r.array()?,
			time: r.u32()?,
			bits: r.u32()?,
			period_start: r.u32()?,
			chainwork: r.u128()?,
		})
	}
}

impl Codec for Rail {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			Rail::BtcSpv { confirmations, pay_secs, grace_blocks } => {
				out.put_u8(0);
				out.put_u8(*confirmations);
				out.put_u32(*pay_secs);
				out.put_u16(*grace_blocks);
			},
			Rail::Oracle { oracles, threshold } => {
				out.put_u8(1);
				enc_vec(oracles, out);
				out.put_u8(*threshold);
			},
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			0 => Rail::BtcSpv { confirmations: r.u8()?, pay_secs: r.u32()?, grace_blocks: r.u16()? },
			1 => Rail::Oracle { oracles: dec_vec(r, MAX_ORACLES)?, threshold: r.u8()? },
			_ => return Err(DecodeError),
		})
	}
}

impl Codec for ExternalCfg {
	fn enc(&self, out: &mut Vec<u8>) {
		self.rail.enc(out);
		out.put_u16(self.bond_bps);
		out.put_u32(self.claim_window);
		out.put_var(self.min_deal);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { rail: Rail::dec(r)?, bond_bps: r.u16()?, claim_window: r.u32()?, min_deal: r.var()? })
	}
}

impl Codec for AssetKind {
	fn enc(&self, out: &mut Vec<u8>) {
		match self {
			AssetKind::Token => out.put_u8(0),
			AssetKind::External(c) => {
				out.put_u8(1);
				c.enc(out);
			},
		}
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(match r.u8()? {
			0 => AssetKind::Token,
			1 => AssetKind::External(ExternalCfg::dec(r)?),
			_ => return Err(DecodeError),
		})
	}
}

impl Codec for Asset {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.issuer);
		out.put(&self.symbol);
		out.put_u8(self.decimals);
		out.put_u8(self.flags);
		self.kind.enc(out);
		out.put_var(self.supply);
		enc_vec(&self.attestors, out);
		out.put_u8(self.threshold);
		enc_vec(&self.attested, out);
		out.put_u32(self.freeze_log);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		let a = Self {
			issuer: r.array()?,
			symbol: r.array()?,
			decimals: r.u8()?,
			flags: r.u8()?,
			kind: AssetKind::dec(r)?,
			supply: r.var()?,
			attestors: dec_vec(r, MAX_ATTESTORS)?,
			threshold: r.u8()?,
			attested: dec_vec(r, MAX_ATTESTORS)?,
			freeze_log: r.u32()?,
		};
		if a.attested.len() != a.attestors.len() {
			return Err(DecodeError);
		}
		Ok(a)
	}
}

// ---------------------------------------------------------------------------------------
// Deals: conditional payments released on proof
// ---------------------------------------------------------------------------------------

/// The paying side requires the off-chain payment to carry the deal's tag (an `OP_RETURN`
/// output on Bitcoin), so that no other payment to the same address can be passed off as it.
pub const DEAL_REQUIRE_TAG: u8 = 1;
/// A Bitcoin deal that any untagged payment to its address can settle. At most one such
/// deal is open per address at a time, so the payment cannot be claimed for another.
pub const DEAL_UNTAGGED: u8 = 2;

/// How a deal ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Outcome {
	/// The outside leg happened: the payee receives the locked amount and its bond back.
	Paid,
	/// It did not: the payer is refunded and receives the payee's bond.
	Unpaid,
	/// It could not be established either way (an address that cannot be paid, a header
	/// chain that stalled): each side gets back what it put in.
	Void,
}

/// One trade between two people in which one leg happens outside JAM.
///
/// `payer` locked `amount` of `token` here. `payee` must deliver `ext_amount` of the external
/// asset to `delivery` and prove it; then the locked amount is released to the payee. If the
/// time for that runs out, the payer is refunded and also receives the payee's bond.
/// Nothing in a deal can reach anybody else's money.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Deal {
	pub ext_asset: AssetId,
	pub token: AssetId,
	pub payer: AccountId,
	pub payee: AccountId,
	pub ext_amount: u64,
	pub amount: u64,
	pub bond: u64,
	/// Where the external asset must arrive: a Bitcoin output script, or an opaque payment
	/// reference for oracle rails.
	pub delivery: Vec<u8>,
	pub created: Slot,
	/// Oracle rails: the last slot at which a verdict is accepted. Bitcoin rails: the slot
	/// after which the deal may be cancelled without penalty.
	pub deadline: Slot,
	/// The proving block must be above this height (set to the light client's tip at creation).
	pub min_height: u32,
	/// ... and its median time past must be no later than this Unix time (Bitcoin rails;
	/// zero otherwise).
	pub pay_until: u32,
	pub flags: u8,
	/// Oracle votes, eight bits per verdict: bit `i` says oracle `i` saw the payment, bit
	/// `8 + i` that it did not, bit `16 + i` that the payment cannot be made at all.
	pub votes: u32,
}

impl Codec for Deal {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(self.ext_asset as u64);
		out.put_var(self.token as u64);
		out.put(&self.payer);
		out.put(&self.payee);
		out.put_var(self.ext_amount);
		out.put_var(self.amount);
		out.put_var(self.bond);
		out.put_var_bytes(&self.delivery);
		out.put_u32(self.created);
		out.put_u32(self.deadline);
		out.put_u32(self.min_height);
		out.put_u32(self.pay_until);
		out.put_u8(self.flags);
		out.put_u32(self.votes);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			ext_asset: r.var_u32()?,
			token: r.var_u32()?,
			payer: r.array()?,
			payee: r.array()?,
			ext_amount: r.var()?,
			amount: r.var()?,
			bond: r.var()?,
			delivery: {
				let d = r.var_bytes()?;
				if d.len() > MAX_SCRIPT_LEN {
					return Err(DecodeError);
				}
				d.to_vec()
			},
			created: r.u32()?,
			deadline: r.u32()?,
			min_height: r.u32()?,
			pay_until: r.u32()?,
			flags: r.u8()?,
			votes: r.u32()?,
		})
	}
}

// ---------------------------------------------------------------------------------------
// Service-wide configuration
// ---------------------------------------------------------------------------------------

/// A code upgrade that has been announced and is waiting out its public delay.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PendingUpgrade {
	pub code_hash: Hash32,
	pub code_len: u32,
	pub min_item_gas: u64,
	pub min_memo_gas: u64,
	pub eta: Slot,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Config {
	/// The account allowed to announce upgrades. `None`: the code can never change.
	pub admin: Option<AccountId>,
	/// Slots between announcing an upgrade and being able to apply it.
	pub upgrade_delay: Slot,
	pub next_asset: AssetId,
	pub next_deal: DealId,
	pub upgrade: Option<PendingUpgrade>,
	pub receipt_seq: u32,
}

impl Default for Config {
	fn default() -> Self {
		Self { admin: None, upgrade_delay: 0, next_asset: 1, next_deal: 1, upgrade: None, receipt_seq: 0 }
	}
}

impl Codec for PendingUpgrade {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.code_hash);
		out.put_u32(self.code_len);
		out.put_var(self.min_item_gas);
		out.put_var(self.min_memo_gas);
		out.put_u32(self.eta);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			code_hash: r.array()?,
			code_len: r.u32()?,
			min_item_gas: r.var()?,
			min_memo_gas: r.var()?,
			eta: r.u32()?,
		})
	}
}

impl Codec for Config {
	fn enc(&self, out: &mut Vec<u8>) {
		enc_opt(&self.admin, out);
		out.put_u32(self.upgrade_delay);
		out.put_var(self.next_asset as u64);
		out.put_var(self.next_deal);
		enc_opt(&self.upgrade, out);
		out.put_u32(self.receipt_seq);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self {
			admin: dec_opt(r)?,
			upgrade_delay: r.u32()?,
			next_asset: r.var_u32()?,
			next_deal: r.var()?,
			upgrade: dec_opt(r)?,
			receipt_seq: r.u32()?,
		})
	}
}

/// Last cleared price and running totals for one market. Informational.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Market {
	pub last_price: Price,
	pub last_slot: Slot,
	pub batches: u64,
	pub volume: u128,
}

impl Codec for Market {
	fn enc(&self, out: &mut Vec<u8>) {
		self.last_price.enc(out);
		out.put_u32(self.last_slot);
		out.put_var(self.batches);
		out.put_u128(self.volume);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { last_price: Price::dec(r)?, last_slot: r.u32()?, batches: r.var()?, volume: r.u128()? })
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn price_math() {
		let p = Price::new(3, 2); // 1.5 quote per base
		assert_eq!(p.quote_floor(5), Some(7));
		assert_eq!(p.quote_ceil(5), Some(8));
		assert_eq!(p.quote_ceil(4), Some(6));
		assert_eq!(p.base_for_quote(8), 5);
		assert_eq!(p.base_for_quote(7), 4);
		for q in 0..200u64 {
			let b = p.base_for_quote(q);
			assert!(p.quote_ceil(b).unwrap() <= q);
			assert!(p.quote_ceil(b + 1).unwrap() > q);
		}
		assert_eq!(Price::new(1, 3).cmp_value(&Price::new(2, 6)), Ordering::Equal);
		assert_eq!(Price::new(u64::MAX, 1).cmp_value(&Price::new(u64::MAX, 2)), Ordering::Greater);
		assert_eq!(Price::new(u64::MAX, 1).quote_ceil(2), None);
	}

	#[test]
	fn nonce_window() {
		let mut a = Account::default();
		assert!(a.nonce_usable(0));
		a.nonce_use(0).unwrap();
		assert!(!a.nonce_usable(0));
		assert_eq!(a.nonce_use(0), Err(Status::Nonce));
		a.nonce_use(5).unwrap();
		a.nonce_use(255).unwrap();
		assert!(a.nonce_usable(3));
		// Jumping ahead slides the window; skipped nonces below the new floor die.
		a.nonce_use(300).unwrap();
		assert_eq!(a.nonce_floor, 45);
		assert!(!a.nonce_usable(3));
		assert!(!a.nonce_usable(44));
		assert!(a.nonce_usable(45));
		assert!(!a.nonce_usable(255));
		assert!(!a.nonce_usable(300));
		assert!(a.nonce_usable(299));
		// Far jumps are refused.
		assert!(!a.nonce_usable(45 + NONCE_MAX_JUMP));
		// A big slide clears everything.
		a.slide(1000);
		assert_eq!(a.used, [0; 4]);
		assert!(a.nonce_usable(1045));
		assert!(!a.nonce_usable(1044));
	}

	#[test]
	fn slide_matches_naive() {
		// Compare the word-wise shift against a bit-by-bit model.
		for by in [1u64, 7, 63, 64, 65, 127, 128, 129, 200, 255] {
			let mut a = Account::default();
			let marks = [0u64, 1, 63, 64, 100, 128, 191, 192, 254, 255];
			for m in marks {
				a.nonce_use(m).unwrap();
			}
			a.slide(by);
			for off in 0..NONCE_WINDOW {
				let n = a.nonce_floor + off;
				let expect_used = marks.contains(&n);
				assert_eq!(!a.nonce_usable(n), expect_used, "by={by} n={n}");
			}
		}
	}

	#[test]
	fn account_roundtrip() {
		let id = [9u8; 32];
		let mut a = Account::default();
		assert!(a.is_empty());
		a.credit(7, 100).unwrap();
		a.credit(3, 5).unwrap();
		a.credit(900, u64::MAX).unwrap();
		assert_eq!(a.credit(900, 1), Err(Status::Overflow));
		assert_eq!(a.debit(3, 6), Err(Status::Insufficient));
		a.nonce_use(4).unwrap();
		a.open.push(OpenOrder { nonce: 9, tag: [1; 8], filled: 10, expiry: 99 });
		let p = a.policy_mut(&id);
		p.apply(PolicyChange::AddKey { key: [2; 32], role: ROLE_TRADER }, 0).unwrap();
		p.apply(PolicyChange::SetGuardian(Some([3; 32])), 0).unwrap();
		p.apply(
			PolicyChange::SetRecovery(Some(RecoveryCfg {
				guardians: alloc::vec![[4; 32], [5; 32]],
				threshold: 2,
				delay: 10,
			})),
			0,
		)
		.unwrap();
		p.apply(PolicyChange::SetLimit { asset: 7, per_day: Some(50) }, 0).unwrap();
		p.pending.push(Pending { eta: 5, change: PolicyChange::SetDelay(1) });
		p.recovery_req = Some(RecoveryReq { votes: alloc::vec![Vote { guardian: 1, key: [6; 32] }], new_owner: [6; 32], eta: 0 });
		let bytes = a.to_bytes();
		assert_eq!(Account::from_bytes(&bytes).unwrap(), a);
		assert!(!a.is_empty());
		// An emptied balance is still stored (the account accepts the asset) until the
		// account says otherwise; a balance that holds something cannot be dropped.
		a.debit(3, 5).unwrap();
		let back = Account::from_bytes(&a.to_bytes()).unwrap();
		assert_eq!(back.bal(3).map(|b| b.free), Some(0));
		assert_eq!(back.free(7), 100);
		assert_eq!(a.unaccept(7), Err(Status::Invalid));
		assert_eq!(a.unaccept(3), Ok(()));
		assert_eq!(a.unaccept(3), Err(Status::NotFound));
		assert!(Account::from_bytes(&a.to_bytes()).unwrap().bal(3).is_none());
	}

	#[test]
	fn authorization_roles() {
		let id = [1u8; 32];
		let mut a = Account::default();
		assert!(a.authorized(&id, &id, Perm::Full));
		assert!(!a.authorized(&id, &[2; 32], Perm::Trade));
		a.policy_mut(&id).apply(PolicyChange::AddKey { key: [2; 32], role: ROLE_TRADER }, 0).unwrap();
		assert!(a.authorized(&id, &[2; 32], Perm::Trade));
		assert!(!a.authorized(&id, &[2; 32], Perm::Full));
		assert!(a.authorized(&id, &id, Perm::Full));
		// The last owner key cannot be removed.
		assert_eq!(
			a.policy_mut(&id).apply(PolicyChange::RemoveKey { key: id }, 0),
			Err(Status::Invalid)
		);
	}

	#[test]
	fn daily_limit() {
		let id = [1u8; 32];
		let mut a = Account::default();
		let p = a.policy_mut(&id);
		p.apply(PolicyChange::SetLimit { asset: 1, per_day: Some(100) }, 1000).unwrap();
		p.spend(1, 60, 1000).unwrap();
		assert_eq!(p.spend(1, 41, 1001), Err(Status::Limit));
		assert_eq!(p.spendable(1, 1001), 40);
		p.spend(1, 40, 1001).unwrap();
		// Other assets are unlimited.
		p.spend(2, u64::MAX, 1001).unwrap();
		// A new day resets the window.
		assert_eq!(p.spendable(1, 1000 + DAY_SLOTS), 100);
		p.spend(1, 100, 1000 + DAY_SLOTS).unwrap();
		assert_eq!(p.spend(1, 1, 1000 + DAY_SLOTS + 5), Err(Status::Limit));
		// Raising weakens, lowering does not.
		assert!(p.weakens(&PolicyChange::SetLimit { asset: 1, per_day: Some(101) }));
		assert!(!p.weakens(&PolicyChange::SetLimit { asset: 1, per_day: Some(99) }));
		assert!(p.weakens(&PolicyChange::SetLimit { asset: 1, per_day: None }));
		assert!(!p.weakens(&PolicyChange::SetLimit { asset: 2, per_day: Some(5) }));
	}

	#[test]
	fn mint_cap_is_threshold_th_largest() {
		let mut a = Asset {
			issuer: [0; 32],
			symbol: *b"BRL\0\0\0\0\0",
			decimals: 2,
			flags: 0,
			kind: AssetKind::Token,
			supply: 0,
			attestors: alloc::vec![[1; 32], [2; 32], [3; 32]],
			threshold: 2,
			attested: alloc::vec![100, 0, 0],
			freeze_log: 0,
		};
		assert_eq!(a.mint_cap(), 0); // one attestor is not enough
		a.attested = alloc::vec![100, 80, 0];
		assert_eq!(a.mint_cap(), 80);
		a.attested = alloc::vec![100, 80, 90];
		assert_eq!(a.mint_cap(), 90);
		a.attestors.clear();
		a.attested.clear();
		assert_eq!(a.mint_cap(), u64::MAX);
		assert_eq!(Asset::from_bytes(&a.to_bytes()).unwrap(), a);
	}
}
