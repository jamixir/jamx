//! The stateful half: applying refine's records to the ledger.
//!
//! Accumulate trusts refine for everything refine could check (signatures, proof-of-work,
//! Merkle paths, the sorting of a batch) and checks here everything that needs state:
//! who may act for an account, whether a nonce is spent, whether funds exist and may move,
//! whether a header is the one consensus expects, whether a block is buried deep enough.
//!
//! Each record is applied atomically. A record that fails leaves no trace and is reported
//! in the receipt; the records around it are unaffected.

use crate::{
	auction::{self, Quote},
	btc::{self, HeaderRec, LightClient},
	codec::{Codec, DecodeResult, Reader, Writer},
	crypto::{Hash32, PubKey},
	ops::*,
	state::{key, Ctx, Host},
	types::*,
};
use alloc::{vec, vec::Vec};
use core::cmp::Ordering;

/// Receipts kept on-chain, as a ring.
pub const RECEIPT_RING: u32 = 64;
/// Deepest fork of the tracked chain the light client will follow.
pub const MAX_REORG: u32 = 144;
/// Most deals one auction round may open.
pub const MAX_DEALS_PER_BATCH: usize = 64;
/// Statuses kept per receipt.
pub const MAX_RECEIPT_STATUSES: usize = 255;

// Gas budgeting. The figures are measured on polkajam 0.1.29 (GP 0.8.0), whose provisional
// gas model prices every memory access at 25 and every branch at 20; see docs/GAS.md.

//
// JAM runs one accumulate invocation per service per block and pools the gas of all the
// work items in it. Each item is therefore held to the gas it declared for itself
// ([`Session::work`]): an item that asks for more work than it paid for fails alone, with
// an `OutOfGas` status, instead of spending the gas of the items behind it.

/// Gas kept back for writing one changed record at the end of the work item.
const GAS_PER_FLUSHED_RECORD: u64 = 6_000;
/// Gas kept back for restoring one record if the instruction in progress has to be undone.
const GAS_PER_UNDO: u64 = 2_000;
/// Gas kept back for a work item's own receipt and checkpoint. An item that does not bring
/// even this much is ignored.
pub const GAS_COMMIT: u64 = 40_000;
/// Gas a simple record may need (a few reads plus computation).
const GAS_PER_RECORD: u64 = 60_000;
/// Gas budgeted per order when settling a batch, from the point where the batch has been
/// decoded. Measured on a 200-order round: 28,600 per order in all, of which about 4,000
/// is decoding; the margin is for accounts with policies and many balances. This is only
/// a quick way to refuse a batch that plainly cannot fit: the loops below check as they go.
const GAS_PER_ORDER: u64 = 30_000;
/// The most one step of a loop may cost (loading the largest possible account and working
/// on it; storing a header and moving the tip). Loops check that this much is left before
/// each step, so they stop cleanly rather than run dry.
const GAS_PER_STEP: u64 = 60_000;

/// Is there gas for another step of `need`, on top of what finishing the work item takes?
fn gas_check<H: Host>(ctx: &mut Ctx<H>, need: u64) -> Res {
	let reserve = GAS_COMMIT +
		(ctx.dirty_count() as u64 + 2) * GAS_PER_FLUSHED_RECORD +
		ctx.undo_len() as u64 * GAS_PER_UNDO;
	if ctx.gas_left() < reserve + need {
		return Err(Status::OutOfGas);
	}
	Ok(())
}

/// Why a record was not applied.
pub struct Fail {
	pub status: Status,
	/// The record was a signed operation whose signature, authority and nonce were good.
	/// Its nonce stays spent, so that the same bytes cannot be presented again later, when
	/// they might succeed and mean something else.
	pub keep: bool,
}

impl From<Status> for Fail {
	fn from(status: Status) -> Self {
		Self { status, keep: false }
	}
}

impl From<crate::codec::DecodeError> for Fail {
	fn from(_: crate::codec::DecodeError) -> Self {
		Status::Decode.into()
	}
}

/// Development aid: with the `profile` feature, report the gas spent since the last probe.
#[cfg(feature = "profile")]
macro_rules! probe {
	($ctx:expr, $last:ident, $label:expr) => {{
		let now = $ctx.host.gas();
		$ctx.host.log(&alloc::format!("profile:   {} {}", $label, $last.saturating_sub(now)));
		$last = $ctx.host.gas();
	}};
}
#[cfg(not(feature = "profile"))]
macro_rules! probe {
	($ctx:expr, $last:ident, $label:expr) => {};
}

/// Values fixed when the service is built. They are part of the code, so the code hash
/// commits to them: nobody can race to claim the administrator role after deployment.
#[derive(Clone, Debug, Default)]
pub struct Genesis {
	pub admin: Option<AccountId>,
	pub upgrade_delay: Slot,
}

/// What happened to the records of one work item.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Receipt {
	/// Hash of the work-item payload, which the submitter knows.
	pub payload_hash: Hash32,
	pub slot: Slot,
	/// One [`Status`] per record, in order. A refine failure is a single `0xff`.
	pub statuses: Vec<u8>,
}

impl Codec for Receipt {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(&self.payload_hash);
		out.put_u32(self.slot);
		out.put_var_bytes(&self.statuses);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		Ok(Self { payload_hash: r.array()?, slot: r.u32()?, statuses: r.var_bytes()?.to_vec() })
	}
}

/// One accumulate invocation.
pub struct Session<'a, H: Host> {
	pub ctx: Ctx<'a, H>,
}

impl<'a, H: Host> Session<'a, H> {
	pub fn new(host: &'a mut H, service: u32, now: Slot, genesis: &Genesis) -> Self {
		let mut ctx = Ctx::new(host, service, now);
		if ctx.is_fresh() {
			let c = ctx.config_mut();
			c.admin = genesis.admin;
			c.upgrade_delay = genesis.upgrade_delay;
		}
		Self { ctx }
	}

	/// Apply the output of one refined work item and commit it. `gas_limit` is the
	/// accumulate gas the item declared: it may spend that much and no more.
	pub fn work(&mut self, payload_hash: &Hash32, output: &[u8], gas_limit: u64) -> Vec<u8> {
		let have = self.ctx.host.gas();
		let budget = gas_limit.min(have);
		if budget < GAS_COMMIT {
			return Vec::new();
		}
		self.ctx.gas_floor = have - budget;
		let statuses = apply_output(&mut self.ctx, output);
		self.commit(payload_hash, &statuses);
		self.ctx.gas_floor = 0;
		statuses
	}

	/// Record that a work item's refinement failed (it produced no output to apply).
	pub fn failed_work(&mut self, payload_hash: &Hash32, gas_limit: u64) {
		if gas_limit.min(self.ctx.host.gas()) >= GAS_COMMIT {
			self.commit(payload_hash, &[0xff]);
		}
	}

	/// A transfer from another service. The service keeps the funds (they pay for its
	/// storage); memos carry no instructions.
	pub fn transfer(&mut self, _source: u32, _amount: u64, _memo: &[u8]) {}

	fn commit(&mut self, payload_hash: &Hash32, statuses: &[u8]) {
		self.ctx.begin();
		let seq = self.ctx.config().receipt_seq;
		let receipt = Receipt {
			payload_hash: *payload_hash,
			slot: self.ctx.now,
			statuses: statuses[..statuses.len().min(MAX_RECEIPT_STATUSES)].to_vec(),
		};
		self.ctx.put(&key::receipt((seq % RECEIPT_RING) as u8), &receipt);
		self.ctx.config_mut().receipt_seq = seq.wrapping_add(1);
		self.flush();
	}

	fn flush(&mut self) {
		#[cfg(feature = "profile")]
		let (gas_before, dirty) = (self.ctx.host.gas(), self.ctx.dirty_count());
		let flushed = self.ctx.flush();
		#[cfg(feature = "profile")]
		{
			let used = gas_before - self.ctx.host.gas();
			self.ctx.host.log(&alloc::format!("profile: flush of {dirty} records used {used} gas"));
		}
		if !flushed {
			// The host refused a write for lack of deposit. Abort: the host then discards
			// everything since the last checkpoint, so no half-applied work item survives.
			panic!("jamx: storage deposit exhausted");
		}
		self.ctx.host.checkpoint();
	}

	pub fn finish(mut self) {
		self.flush();
	}
}

/// Apply every record of a refine output. Returns one status byte per record.
pub fn apply_output<H: Host>(ctx: &mut Ctx<H>, output: &[u8]) -> Vec<u8> {
	let Ok(frames) = output_frames(output) else { return vec![Status::Decode as u8] };
	let mut statuses = Vec::with_capacity(frames.len());
	for (tag, body) in frames {
		ctx.begin();
		if gas_check(ctx, GAS_PER_RECORD).is_err() {
			// Not enough gas to be sure of finishing: stop cleanly instead of losing
			// everything to an out-of-gas abort. Skipped operations can be resubmitted.
			statuses.push(Status::OutOfGas as u8);
			continue;
		}
		#[cfg(feature = "profile")]
		let gas_before = ctx.host.gas();
		#[cfg(feature = "profile")]
		let mut last = gas_before;
		let decoded = Record::dec_body(tag, &mut Reader::new(body)).map_err(Fail::from);
		probe!(ctx, last, "decode");
		let res = decoded.and_then(|rec| apply_record(ctx, rec));
		#[cfg(feature = "profile")]
		{
			let used = gas_before - ctx.host.gas();
			ctx.host.log(&alloc::format!("profile: record tag {tag} len {} used {used} gas", body.len()));
		}
		statuses.push(match res {
			Ok(()) => Status::Ok as u8,
			Err(f) => {
				if !f.keep {
					ctx.rollback();
				}
				f.status as u8
			},
		});
	}
	statuses
}

pub fn apply_record<H: Host>(ctx: &mut Ctx<H>, rec: Record) -> Result<(), Fail> {
	match rec {
		Record::Op { signer, body } => apply_op(ctx, &signer, Envelope::from_bytes(&body)?),
		Record::Batch(b) => Ok(settle_batch(ctx, &b)?),
		Record::BtcHeaders { asset, prev, headers } => Ok(add_headers(ctx, asset, prev, &headers)?),
		Record::BtcPayment(p) => Ok(claim_payment(ctx, &p)?),
		Record::Plain(p) => Ok(apply_plain(ctx, p)?),
	}
}

// ---------------------------------------------------------------------------------------
// Signed operations
// ---------------------------------------------------------------------------------------

fn has_duplicates(v: &[AccountId]) -> bool {
	v.iter().enumerate().any(|(i, x)| v[..i].contains(x))
}

/// Apply a signed operation. Authority, freshness and the nonce are checked first; if they
/// hold, the nonce is spent whether or not the operation then succeeds.
fn apply_op<H: Host>(ctx: &mut Ctx<H>, signer: &PubKey, env: Envelope) -> Result<(), Fail> {
	let now = ctx.now;
	let id = env.account;
	if env.expiry < now {
		return Err(Status::Expired.into());
	}
	// A trading key may only take orders back. Freezing is the owner's or the guardian's:
	// a frozen owner cannot veto a recovery, so the power to freeze is a step towards the
	// power to take the account.
	let perm = match env.op {
		Op::Cancel { .. } | Op::BumpNonce { .. } => Perm::Trade,
		_ => Perm::Full,
	};
	{
		let a = ctx.account(&id);
		if !a.authorized(&id, signer, perm) {
			return Err(Status::Unauthorized.into());
		}
		// A frozen account can still do the one thing that only keeps it as it is: drop
		// changes that are waiting to weaken it. Otherwise whoever controlled the freeze
		// could hold the owner still until such a change came due.
		if a.is_frozen(now) && !matches!(env.op, Op::PolicyCancel { .. }) {
			return Err(Status::Frozen.into());
		}
	}
	ctx.account_mut(&id).nonce_use(env.nonce)?;
	match run_op(ctx, &id, env.op) {
		Ok(()) => Ok(()),
		Err(Status::OutOfGas) => Err(Status::OutOfGas.into()),
		Err(status) => {
			ctx.rollback();
			let _ = ctx.account_mut(&id).nonce_use(env.nonce);
			Err(Fail { status, keep: true })
		},
	}
}

fn run_op<H: Host>(ctx: &mut Ctx<H>, id: &AccountId, op: Op) -> Res {
	let now = ctx.now;
	let id = *id;
	match op {
		Op::Transfer { asset, to, amount } => {
			if amount == 0 || to == id {
				return Err(Status::Invalid);
			}
			if !ctx.asset(asset).ok_or(Status::NotFound)?.is_token() {
				return Err(Status::Invalid);
			}
			ctx.debit(&id, asset, amount)?;
			ctx.credit(&to, asset, amount)
		},

		Op::Cancel { nonces } => {
			let a = ctx.account_mut(&id);
			for n in nonces {
				if n >= a.nonce_floor.saturating_add(NONCE_WINDOW) {
					// Out of reach of the window: saying "done" would be a lie, since
					// the order would still trade. `BumpNonce` cancels that far ahead.
					return Err(Status::Nonce);
				}
				a.open.retain(|o| o.nonce != n);
				if n >= a.nonce_floor {
					let _ = a.nonce_use(n);
				}
			}
			Ok(())
		},

		Op::BumpNonce { floor } => {
			let a = ctx.account_mut(&id);
			if floor <= a.nonce_floor || floor - a.nonce_floor > NONCE_MAX_JUMP {
				return Err(Status::Invalid);
			}
			a.slide(floor - a.nonce_floor);
			Ok(())
		},

		Op::AssetCreate { symbol, decimals, flags, kind, attestors, threshold } =>
			create_asset(ctx, &id, symbol, decimals, flags, kind, attestors, threshold),

		Op::Mint { asset, to, amount } => {
			let a = ctx.asset(asset).ok_or(Status::NotFound)?;
			if a.issuer != id {
				return Err(Status::Unauthorized);
			}
			if !a.is_token() || amount == 0 {
				return Err(Status::Invalid);
			}
			let supply = a.supply.checked_add(amount).ok_or(Status::Overflow)?;
			if supply > a.mint_cap() {
				return Err(Status::Cap);
			}
			ctx.credit(&to, asset, amount)?;
			ctx.asset_mut(asset).ok_or(Status::NotFound)?.supply = supply;
			Ok(())
		},

		Op::Burn { asset, amount } => {
			if amount == 0 || !ctx.asset(asset).ok_or(Status::NotFound)?.is_token() {
				return Err(Status::Invalid);
			}
			ctx.debit(&id, asset, amount)?;
			let a = ctx.asset_mut(asset).ok_or(Status::NotFound)?;
			a.supply = a.supply.checked_sub(amount).ok_or(Status::Overflow)?;
			Ok(())
		},

		Op::IssuerSet { asset, target, action, reason } => {
			let a = ctx.asset(asset).ok_or(Status::NotFound)?;
			if a.issuer != id {
				return Err(Status::Unauthorized);
			}
			let (needs, set, bit) = match action {
				IssuerAction::Allow => (ASSET_PERMISSIONED, true, BAL_ALLOWED),
				IssuerAction::Disallow => (ASSET_PERMISSIONED, false, BAL_ALLOWED),
				IssuerAction::Freeze => (ASSET_FREEZABLE, true, BAL_FROZEN),
				IssuerAction::Unfreeze => (ASSET_FREEZABLE, false, BAL_FROZEN),
			};
			if !a.is_token() || a.flags & needs == 0 {
				return Err(Status::Invalid);
			}
			let seq = a.freeze_log;
			// The issuer marks balances; it does not create them. An account that has
			// not accepted the asset has no entry, and cannot be sent the asset either.
			if !ctx.account(&target).accepts(asset) {
				return Err(Status::NotFound);
			}
			let b = ctx.account_mut(&target).bal_mut(asset)?;
			if set {
				b.flags |= bit;
			} else {
				b.flags &= !bit;
			}
			if bit == BAL_FROZEN {
				// The freeze power is logged where everyone can see it.
				let mut entry = Vec::with_capacity(69);
				entry.put(&target);
				entry.put_u8(action as u8);
				entry.put(&reason);
				entry.put_u32(now);
				ctx.put_raw(&key::freeze_log(asset, seq), entry);
				ctx.asset_mut(asset).ok_or(Status::NotFound)?.freeze_log = seq + 1;
			}
			Ok(())
		},

		Op::Attest { asset, reserves } => {
			let a = ctx.asset_mut(asset).ok_or(Status::NotFound)?;
			let i = a.attestors.iter().position(|x| *x == id).ok_or(Status::Unauthorized)?;
			a.attested[i] = reserves;
			Ok(())
		},

		Op::Policy(change) => {
			let p = ctx.account_mut(&id).policy_mut(&id);
			if p.delay > 0 && p.weakens(&change) {
				if p.pending.len() >= MAX_PENDING {
					return Err(Status::Invalid);
				}
				// Refuse now what could never be applied, rather than after the wait.
				p.clone().apply(change.clone(), now)?;
				p.pending.push(Pending { eta: now.saturating_add(p.delay), change });
				Ok(())
			} else {
				p.apply(change, now)
			}
		},

		Op::PolicyCancel { index } => {
			let p = ctx.account_mut(&id).policy.as_mut().ok_or(Status::NotFound)?;
			if index as usize >= p.pending.len() {
				return Err(Status::NotFound);
			}
			p.pending.remove(index as usize);
			Ok(())
		},

		Op::Freeze { target } => {
			let is_guardian =
				ctx.account(&target).policy.as_ref().is_some_and(|p| p.guardian == Some(id));
			if target != id && !is_guardian {
				return Err(Status::Unauthorized);
			}
			let p = ctx.account_mut(&target).policy_mut(&target);
			if now < p.frozen_until {
				return Err(Status::Exists);
			}
			// After a freeze ends, an equal period must pass before the next one, so that
			// freezing cannot be used to lock an owner out for good.
			if p.frozen_until != 0 && now < p.frozen_until.saturating_add(p.freeze_slots) {
				return Err(Status::TooEarly);
			}
			p.frozen_until = now.saturating_add(p.freeze_slots);
			Ok(())
		},

		Op::Unfreeze { target } => {
			// Only the guardian can lift a freeze early. The owner cannot: whoever stole the
			// owner's key would do exactly that.
			let is_guardian =
				ctx.account(&target).policy.as_ref().is_some_and(|p| p.guardian == Some(id));
			if !is_guardian {
				return Err(Status::Unauthorized);
			}
			let p = ctx.account_mut(&target).policy_mut(&target);
			if now >= p.frozen_until {
				return Err(Status::Invalid);
			}
			p.frozen_until = now;
			Ok(())
		},

		Op::RecoveryApprove { target, new_owner } => {
			let cfg = ctx
				.account(&target)
				.policy
				.as_ref()
				.and_then(|p| p.recovery.clone())
				.ok_or(Status::NotFound)?;
			let gi = cfg.guardians.iter().position(|g| *g == id).ok_or(Status::Unauthorized)? as u8;
			let p = ctx.account_mut(&target).policy_mut(&target);
			let req = p.recovery_req.get_or_insert(RecoveryReq {
				votes: Vec::new(),
				new_owner: [0; 32],
				eta: 0,
			});
			if req.eta != 0 {
				return Err(Status::Exists);
			}
			req.votes.retain(|v| v.guardian != gi);
			req.votes.push(Vote { guardian: gi, key: new_owner });
			if req.count(&new_owner) >= cfg.threshold as usize {
				req.new_owner = new_owner;
				req.eta = now.saturating_add(cfg.delay).max(1);
			}
			Ok(())
		},

		Op::RecoveryCancel => {
			let p = ctx.account_mut(&id).policy.as_mut().ok_or(Status::NotFound)?;
			p.recovery_req.take().map(|_| ()).ok_or(Status::NotFound)
		},

		Op::DealRelease { deal } => {
			let d: Deal = ctx.get(&key::deal(deal)).ok_or(Status::NotFound)?;
			if d.payer != id {
				return Err(Status::Unauthorized);
			}
			close_deal(ctx, deal, &d, Outcome::Paid)
		},

		Op::OracleAttest { deal, verdict } => {
			let mut d: Deal = ctx.get(&key::deal(deal)).ok_or(Status::NotFound)?;
			if now > d.deadline {
				return Err(Status::Expired);
			}
			let cfg = ctx
				.asset(d.ext_asset)
				.and_then(|a| a.external().cloned())
				.ok_or(Status::NotFound)?;
			let Rail::Oracle { oracles, threshold } = &cfg.rail else { return Err(Status::Invalid) };
			let i = oracles.iter().position(|o| *o == id).ok_or(Status::Unauthorized)? as u32;
			d.votes &= !(0x01_0101 << i);
			d.votes |= match verdict {
				Verdict::Paid => 1 << i,
				Verdict::NotPaid => 1 << (8 + i),
				Verdict::Void => 1 << (16 + i),
			};
			let votes = d.votes;
			let count = |shift: u32| (votes >> shift & 0xff).count_ones();
			if count(0) >= *threshold as u32 {
				close_deal(ctx, deal, &d, Outcome::Paid)
			} else if count(8) >= *threshold as u32 {
				close_deal(ctx, deal, &d, Outcome::Unpaid)
			} else if count(16) >= *threshold as u32 {
				close_deal(ctx, deal, &d, Outcome::Void)
			} else {
				ctx.put(&key::deal(deal), &d);
				Ok(())
			}
		},

		Op::GovProposeUpgrade { code_hash, code_len, min_item_gas, min_memo_gas } => {
			if ctx.config().admin != Some(id) {
				return Err(Status::Unauthorized);
			}
			let eta = now.saturating_add(ctx.config().upgrade_delay);
			// Ask the chain to hold the new code so that it can be provided and inspected
			// by anyone during the delay.
			let _ = ctx.host.solicit(&code_hash, code_len);
			ctx.config_mut().upgrade =
				Some(PendingUpgrade { code_hash, code_len, min_item_gas, min_memo_gas, eta });
			Ok(())
		},

		Op::GovCancelUpgrade => {
			if ctx.config().admin != Some(id) {
				return Err(Status::Unauthorized);
			}
			ctx.config_mut().upgrade.take().map(|_| ()).ok_or(Status::NotFound)
		},

		Op::GovSetAdmin { admin } => {
			if ctx.config().admin != Some(id) {
				return Err(Status::Unauthorized);
			}
			ctx.config_mut().admin = admin;
			Ok(())
		},

		Op::Accept { asset, on } => {
			if !ctx.asset(asset).ok_or(Status::NotFound)?.is_token() {
				return Err(Status::Invalid);
			}
			let a = ctx.account_mut(&id);
			if on {
				a.bal_mut(asset).map(|_| ())
			} else {
				a.unaccept(asset)
			}
		},
	}
}

#[allow(clippy::too_many_arguments)]
fn create_asset<H: Host>(
	ctx: &mut Ctx<H>,
	issuer: &AccountId,
	symbol: [u8; 8],
	decimals: u8,
	flags: u8,
	kind: AssetKindCfg,
	attestors: Vec<AccountId>,
	threshold: u8,
) -> Res {
	if flags & !(ASSET_PERMISSIONED | ASSET_FREEZABLE) != 0 || has_duplicates(&attestors) {
		return Err(Status::Invalid);
	}
	// An issuer vouching for its own reserves vouches for nothing.
	if attestors.contains(issuer) {
		return Err(Status::Invalid);
	}
	let n = attestors.len();
	if (n == 0 && threshold != 0) || (n > 0 && (threshold == 0 || threshold as usize > n)) {
		return Err(Status::Invalid);
	}
	let id = ctx.config().next_asset;
	let is_token = matches!(kind, AssetKindCfg::Token);
	let kind = match kind {
		AssetKindCfg::Token => AssetKind::Token,
		AssetKindCfg::External { cfg, btc } => {
			if flags != 0 || n != 0 || cfg.bond_bps > 10_000 || cfg.claim_window == 0 || cfg.min_deal == 0 {
				return Err(Status::Invalid);
			}
			match (&cfg.rail, btc) {
				(Rail::BtcSpv { confirmations, pay_secs, grace_blocks }, Some((params, cp))) => {
					let limit = btc::target_from_compact(params.pow_limit_bits).ok_or(Status::Invalid)?;
					let target = btc::target_from_compact(cp.bits).ok_or(Status::Invalid)?;
					if *confirmations == 0 || *pay_secs < MIN_PAY_SECS || *grace_blocks == 0 || target > limit {
						return Err(Status::Invalid);
					}
					// The way out for a stalled header chain must not come before the chain
					// has had ample time to run the deal's whole course: the payment window,
					// then the blocks it takes median time past to catch up, the
					// confirmations and the grace period.
					let blocks = (btc::MTP_SPAN / 2 + 1) as u64 + *confirmations as u64 + *grace_blocks as u64;
					if (cfg.claim_window as u64) < *pay_secs as u64 / SLOT_SECONDS + blocks * MIN_SLOTS_PER_BLOCK {
						return Err(Status::Invalid);
					}
					// Total work is compared and added to: a starting value near the top of
					// the range would leave no room for the chain to grow.
					if cp.chainwork > u128::MAX >> 8 {
						return Err(Status::Invalid);
					}
					ctx.put(
						&key::header(id, &cp.hash),
						&HeaderRec {
							height: cp.height,
							prev: [0; 32],
							merkle_root: [0; 32],
							time: cp.time,
							bits: cp.bits,
							chainwork: cp.chainwork,
							period_start: cp.period_start,
						},
					);
					ctx.put(&key::height(id, cp.height), &cp.hash);
					ctx.put(
						&key::chain(id),
						&LightClient {
							params,
							tip: cp.hash,
							tip_height: cp.height,
							tip_work: cp.chainwork,
							base_height: cp.height,
						},
					);
				},
				(Rail::Oracle { oracles, threshold }, None) => {
					let k = oracles.len();
					if k == 0 || has_duplicates(oracles) || *threshold == 0 || *threshold as usize > k {
						return Err(Status::Invalid);
					}
				},
				_ => return Err(Status::Invalid),
			}
			AssetKind::External(cfg)
		},
	};
	ctx.asset_insert(
		id,
		Asset {
			issuer: *issuer,
			symbol,
			decimals,
			flags,
			kind,
			supply: 0,
			attestors,
			threshold,
			attested: vec![0; n],
			freeze_log: 0,
		},
	);
	ctx.config_mut().next_asset = id.checked_add(1).ok_or(Status::Overflow)?;
	if is_token {
		// The issuer holds its own liability from the start (redemptions arrive there).
		let b = ctx.account_mut(issuer).bal_mut(id)?;
		if flags & ASSET_PERMISSIONED != 0 {
			b.flags |= BAL_ALLOWED;
		}
	}
	Ok(())
}

/// Slots the header chain is given per block when judging whether a deal's fallback
/// deadline is far enough away: twice Bitcoin's average of 100 slots.
pub const MIN_SLOTS_PER_BLOCK: u64 = 200;
/// Shortest payment window an asset may set: one average block interval.
pub const MIN_PAY_SECS: u32 = 600;
/// Gas a receipt or a timeout on a Bitcoin rail may need: a dozen headers are read to
/// compute a median time past.
const GAS_PER_TIME_CHECK: u64 = 150_000;

/// The median time past of a block: the median of its own timestamp and those of the ten
/// blocks before it (fewer if the chain known here does not reach that far back).
fn median_time_past<H: Host>(ctx: &mut Ctx<H>, asset: AssetId, rec: &HeaderRec) -> u32 {
	let mut times = [0u32; btc::MTP_SPAN];
	times[0] = rec.time;
	let mut n = 1;
	let mut prev = rec.prev;
	while n < btc::MTP_SPAN && prev != [0; 32] {
		let Some(parent) = ctx.get::<HeaderRec>(&key::header(asset, &prev)) else { break };
		times[n] = parent.time;
		n += 1;
		prev = parent.prev;
	}
	btc::median_time(&mut times[..n])
}

// ---------------------------------------------------------------------------------------
// Operations anyone may trigger
// ---------------------------------------------------------------------------------------

fn apply_plain<H: Host>(ctx: &mut Ctx<H>, op: PlainOp) -> Res {
	let now = ctx.now;
	match op {
		PlainOp::DealTimeout { deal } => {
			let d: Deal = ctx.get(&key::deal(deal)).ok_or(Status::NotFound)?;
			let cfg = ctx.asset(d.ext_asset).and_then(|a| a.external().cloned()).ok_or(Status::NotFound)?;
			let outcome = match cfg.rail {
				Rail::BtcSpv { confirmations, grace_blocks, .. } => {
					gas_check(ctx, GAS_PER_TIME_CHECK)?;
					// The clock is the outside chain's own. Take the block that lies under
					// enough others to be confirmed and to have left time to present a
					// receipt. If even that block is past the payment window by median
					// time past, no payment made in time can still turn up.
					let lc: LightClient = ctx.get(&key::chain(d.ext_asset)).ok_or(Status::NotFound)?;
					let depth = confirmations as u32 + grace_blocks as u32;
					let marker = lc
						.tip_height
						.checked_sub(depth)
						.filter(|h| *h > lc.base_height)
						.and_then(|h| ctx.get::<Hash32>(&key::height(d.ext_asset, h)))
						.and_then(|hash| ctx.get::<HeaderRec>(&key::header(d.ext_asset, &hash)));
					let window_closed = match marker {
						Some(rec) => median_time_past(ctx, d.ext_asset, &rec) > d.pay_until,
						None => false,
					};
					if window_closed {
						Outcome::Unpaid
					} else if now > d.deadline {
						// Nobody has carried the headers for a very long time. Whatever
						// the reason, the deal is undone without blaming either side.
						Outcome::Void
					} else {
						return Err(Status::TooEarly);
					}
				},
				Rail::Oracle { .. } => {
					if now <= d.deadline {
						return Err(Status::TooEarly);
					}
					Outcome::Unpaid
				},
			};
			close_deal(ctx, deal, &d, outcome)
		},

		PlainOp::PolicyExec { account, index } => {
			let a = ctx.account_mut(&account);
			if a.is_frozen(now) {
				return Err(Status::Frozen);
			}
			let p = a.policy.as_mut().ok_or(Status::NotFound)?;
			let pending = p.pending.get(index as usize).ok_or(Status::NotFound)?;
			if now < pending.eta {
				return Err(Status::TooEarly);
			}
			let change = p.pending.remove(index as usize).change;
			p.apply(change, now)
		},

		PlainOp::RecoveryExec { account } => {
			let p = ctx.account_mut(&account).policy.as_mut().ok_or(Status::NotFound)?;
			let req = p.recovery_req.as_ref().ok_or(Status::NotFound)?;
			if req.eta == 0 || now < req.eta {
				return Err(Status::TooEarly);
			}
			// The new key replaces every old key. Whatever the old keys had queued is
			// dropped, and any freeze is lifted: the account is back in its owner's hands.
			p.keys = vec![KeyEntry { key: req.new_owner, role: ROLE_OWNER }];
			p.pending.clear();
			p.frozen_until = 0;
			p.recovery_req = None;
			Ok(())
		},

		PlainOp::GovExecUpgrade => {
			let up = ctx.config().upgrade.clone().ok_or(Status::NotFound)?;
			if now < up.eta {
				return Err(Status::TooEarly);
			}
			if !ctx.host.preimage_available(&up.code_hash) {
				return Err(Status::NotFound);
			}
			ctx.host.upgrade(&up.code_hash, up.min_item_gas, up.min_memo_gas);
			ctx.config_mut().upgrade = None;
			Ok(())
		},
	}
}

/// End a deal and hand back what it holds, according to how it ended.
fn close_deal<H: Host>(ctx: &mut Ctx<H>, id: DealId, d: &Deal, outcome: Outcome) -> Res {
	let total = d.amount.checked_add(d.bond).ok_or(Status::Overflow)?;
	match outcome {
		Outcome::Paid => ctx.credit_back(&d.payee, d.token, total)?,
		Outcome::Unpaid => ctx.credit_back(&d.payer, d.token, total)?,
		Outcome::Void => {
			ctx.credit_back(&d.payer, d.token, d.amount)?;
			ctx.credit_back(&d.payee, d.token, d.bond)?;
		},
	}
	if d.flags & DEAL_UNTAGGED != 0 {
		ctx.del(&key::untagged(d.ext_asset, &d.delivery));
	}
	ctx.del(&key::deal(id));
	Ok(())
}

// ---------------------------------------------------------------------------------------
// Settlement of an auction round
// ---------------------------------------------------------------------------------------

fn fee_of(amount: u64, bps: u16) -> u64 {
	// `bps <= 10_000`, so the result always fits; the helper stays in 64 bits when it can,
	// where a division is one instruction instead of a library call.
	mul_div_floor(amount, bps as u64, 10_000).unwrap_or(u64::MAX)
}

/// What a buyer of `base` atoms pays in all, fee included.
fn buy_cost(price: &Price, fee_bps: u16, base: u64) -> Option<u64> {
	let q = price.quote_ceil(base)?;
	q.checked_add(fee_of(q, fee_bps))
}

/// The most base a budget of quote atoms buys, fee included.
fn max_buy(price: &Price, fee_bps: u16, budget: u64) -> u64 {
	let net = mul_div_floor(budget, 10_000, 10_000 + fee_bps as u64).unwrap_or(u64::MAX);
	let mut c = price.base_for_quote(net);
	// Rounding can leave room for a few more atoms.
	for _ in 0..4 {
		match c.checked_add(1).and_then(|n| buy_cost(price, fee_bps, n)) {
			Some(x) if x <= budget => c += 1,
			_ => break,
		}
	}
	match buy_cost(price, fee_bps, c) {
		Some(x) if x <= budget => c,
		_ => 0,
	}
}

/// The most base whose bond a budget of quote atoms covers.
fn max_for_bond(price: &Price, bond_bps: u16, budget: u64) -> u64 {
	if bond_bps == 0 {
		return u64::MAX;
	}
	// Largest q with floor(q * bps / 10_000) <= budget.
	let q = match budget.checked_add(1).and_then(|b| b.checked_mul(10_000)) {
		Some(n) => (n - 1) / bond_bps as u64,
		None => u64::try_from(((budget as u128 + 1) * 10_000 - 1) / bond_bps as u128).unwrap_or(u64::MAX),
	};
	price.base_for_quote(q)
}

struct Mkt<'m> {
	base: AssetId,
	quote: AssetId,
	fee_bps: u16,
	base_permissioned: bool,
	quote_permissioned: bool,
	ext: Option<&'m ExternalCfg>,
	/// The base asset is paid on a Bitcoin-style chain.
	btc: bool,
}

/// What state says about one order, established once, before any price is known.
#[derive(Clone, Copy)]
struct Prep {
	/// Position of the account in the cache.
	ix: u32,
	/// Base atoms of the order not yet filled. Zero: the order may not trade at all.
	remaining: u64,
	/// What the account can put up of the asset it gives: its free balance, within its
	/// daily limit.
	avail: u64,
}

/// What a round has already admitted from one account. Two orders of the same account must
/// be able to be marked in either order, whatever the auction then does with them.
#[derive(Clone, Copy, Default)]
struct Seen {
	any: bool,
	/// Lowest and highest nonce admitted.
	lo: u64,
	hi: u64,
	/// New orders admitted that may come to rest, each needing a place in the account's
	/// list of open orders.
	resting: u8,
}

/// Index into the per-account reservations: the asset an order gives up.
fn gives_base(m: &Mkt, side: Side) -> bool {
	side == Side::Sell && m.ext.is_none()
}

/// Check an order against state: may this key trade for this account, is the order still
/// live, may the account give what it offers and receive what it asks for?
///
/// Everything that could make the order fail at settlement is decided here, where refusing
/// it costs nobody else anything. Once the round has a price, a failure would undo the
/// trades of everyone in it.
fn prepare<H: Host>(ctx: &mut Ctx<H>, m: &Mkt, o: &OrderRec, side: Side, seen: &mut Vec<Seen>) -> Prep {
	let now = ctx.now;
	let dead = Prep { ix: 0, remaining: 0, avail: 0 };
	if o.qty == 0 || !o.limit.valid() || o.expiry < now || o.max_fee_bps < m.fee_bps {
		return dead;
	}
	if m.ext.is_some() && side == Side::Buy {
		if o.delivery.is_empty() {
			return dead;
		}
		if m.btc {
			// A destination no wallet can pay would cost the seller its bond for nothing.
			if !btc::is_standard_script(&o.delivery) {
				return dead;
			}
			// Only one deal at a time may be open to untagged payments to an address.
			if o.flags & ORDER_NO_TAG != 0 && ctx.exists(&key::untagged(m.base, &o.delivery)) {
				return dead;
			}
		}
	}
	let ix = ctx.account_ix(&o.account);
	if ix >= seen.len() {
		seen.resize(ix + 1, Seen::default());
	}
	let a = ctx.acct(ix);
	if !a.authorized(&o.account, &o.signer, Perm::Trade) || a.is_frozen(now) {
		return dead;
	}
	let rests = o.flags & ORDER_IOC == 0;
	let (remaining, known) = match a.open_order(o.nonce) {
		Some(e) => {
			if !rests || e.tag != o.tag {
				return dead;
			}
			(o.qty.saturating_sub(e.filled), true)
		},
		None => {
			if !a.nonce_usable(o.nonce) {
				return dead;
			}
			(o.qty, false)
		},
	};
	// Spending a nonce far ahead slides the window and kills what falls out of it. Orders
	// of one account whose nonces are a window apart could kill each other that way,
	// depending on which was marked first.
	let mut s = seen[ix];
	let (lo, hi) = if s.any { (s.lo.min(o.nonce), s.hi.max(o.nonce)) } else { (o.nonce, o.nonce) };
	if hi - lo >= NONCE_WINDOW {
		return dead;
	}
	// A new order that may come to rest needs a place in the list of open orders. Without
	// one its remainder would be lost after its first fill.
	if rests && !known {
		let live = a.open.iter().filter(|e| e.expiry >= now).count();
		if live + s.resting as usize >= MAX_OPEN_ORDERS {
			return dead;
		}
		s.resting += 1;
	}
	// What the order gives up on JAM, and what it would receive here.
	let (give, receives_permissioned) = match (side, m.ext.is_some()) {
		(Side::Buy, false) => (m.quote, m.base_permissioned.then_some(m.base)),
		(Side::Buy, true) => (m.quote, None),
		(Side::Sell, false) => (m.base, m.quote_permissioned.then_some(m.quote)),
		(Side::Sell, true) => (m.quote, m.quote_permissioned.then_some(m.quote)),
	};
	if receives_permissioned.is_some_and(|asset| a.bal_flags(asset) & BAL_ALLOWED == 0) {
		return dead;
	}
	// An account that could not take delivery must not be matched.
	let receives = if side == Side::Buy { m.base } else { m.quote };
	if !(m.ext.is_some() && side == Side::Buy) && !a.can_hold(receives) {
		return dead;
	}
	let (free, flags) = a.bal(give).map_or((0, 0), |b| (b.free, b.flags));
	if flags & BAL_FROZEN != 0 {
		return dead;
	}
	let within_limit = a.policy.as_ref().map_or(u64::MAX, |p| p.spendable(give, now));
	seen[ix] = Seen { any: true, lo, hi, resting: s.resting };
	Prep { ix: ix as u32, remaining, avail: free.min(within_limit) }
}

/// How much of an order can trade at `price`: the lesser of what is left of it and what its
/// account can fund. Funds one order counts on are set aside in `held` (per account: quote,
/// base) so that the next order of the same account cannot count on them too.
fn fund(m: &Mkt, side: Side, p: &Prep, price: &Price, held: &mut [[u64; 2]]) -> u64 {
	if p.remaining == 0 {
		return 0;
	}
	let slot = &mut held[p.ix as usize][gives_base(m, side) as usize];
	let avail = p.avail.saturating_sub(*slot);
	let cap = match (side, m.ext) {
		(Side::Buy, _) => p.remaining.min(max_buy(price, m.fee_bps, avail)),
		(Side::Sell, None) => p.remaining.min(avail),
		(Side::Sell, Some(cfg)) => p.remaining.min(max_for_bond(price, cfg.bond_bps, avail)),
	};
	if cap == 0 || m.ext.is_some_and(|cfg| cap < cfg.min_deal) {
		return 0;
	}
	let uses = match (side, m.ext) {
		(Side::Buy, _) => buy_cost(price, m.fee_bps, cap).unwrap_or(u64::MAX),
		(Side::Sell, None) => cap,
		(Side::Sell, Some(cfg)) => fee_of(price.quote_ceil(cap).unwrap_or(u64::MAX), cfg.bond_bps),
	};
	*slot = slot.saturating_add(uses);
	cap
}

/// Could [`take`] succeed?
fn can_take(a: &Account, asset: AssetId, amount: u64, now: Slot) -> bool {
	a.free(asset) >= amount && a.policy.as_ref().is_none_or(|p| p.spendable(asset, now) >= amount)
}

/// Take `amount` of `asset` from an account that [`prepare`] has already cleared to give
/// it, counting it against the daily limit.
fn take(a: &mut Account, asset: AssetId, amount: u64, now: Slot) -> Res {
	if let Some(p) = a.policy.as_mut() {
		p.spend(asset, amount, now)?;
	}
	a.debit(asset, amount)
}

/// Record that `fill` more of an order has traded: spend its nonce the first time, and
/// remember the remainder if the order rests.
fn mark_fill(a: &mut Account, o: &OrderRec, fill: u64, now: Slot) -> Res {
	if !a.open.is_empty() {
		a.drop_expired_orders(now);
	}
	let prior = a.open_order(o.nonce).map(|e| e.filled);
	if prior.is_none() {
		a.nonce_use(o.nonce)?;
	}
	let filled = prior.unwrap_or(0).checked_add(fill).ok_or(Status::Overflow)?;
	let done = filled >= o.qty || o.flags & ORDER_IOC != 0;
	if prior.is_some() {
		a.open.retain(|e| e.nonce != o.nonce);
	}
	if !done && a.open.len() < MAX_OPEN_ORDERS {
		a.open.push(OpenOrder { nonce: o.nonce, tag: o.tag, filled, expiry: o.expiry });
	}
	Ok(())
}

/// Pay the round's fee and rounding dust to whoever built the batch. If that account may
/// not or cannot hold the asset, the amount is destroyed instead, so that supply always
/// equals the sum of what is held and a bundler's account can never block a round.
fn pay_fee<H: Host>(ctx: &mut Ctx<H>, to: &AccountId, asset: AssetId, amount: u64) -> Res {
	if amount == 0 {
		return Ok(());
	}
	match ctx.credit(to, asset, amount) {
		Err(Status::NotAllowed | Status::Invalid) => {
			let a = ctx.asset_mut(asset).ok_or(Status::NotFound)?;
			a.supply = a.supply.checked_sub(amount).ok_or(Status::Overflow)?;
			Ok(())
		},
		other => other,
	}
}

/// Amounts are whole atoms, so the round's price has to be rounded for each fill: up for a
/// buyer, down for a seller, the difference going to whoever built the batch. That can
/// carry a fill past its order's limit by less than one atom. From this many quote atoms
/// up, that is at most one part in a hundred and is accepted. Below it, where one atom
/// could be most of the fill, the rounding goes the trader's way instead and the builder
/// makes up the atom: nobody can be filled a crumb at a time at a multiple of their limit.
pub const ROUND_FOR_TRADER_BELOW: u64 = 100;

/// What a buyer of `fill` pays at the round's price, before the fee.
fn buyer_pays(price: &Price, limit: &Price, fill: u64) -> Res<u64> {
	let at_price = price.quote_ceil(fill).ok_or(Status::Overflow)?;
	if at_price >= ROUND_FOR_TRADER_BELOW {
		return Ok(at_price);
	}
	Ok(at_price.min(limit.quote_floor(fill).unwrap_or(u64::MAX)))
}

/// What a seller of `fill` receives at the round's price, before the fee.
fn seller_gets(price: &Price, limit: &Price, fill: u64) -> Res<u64> {
	let at_price = price.quote_floor(fill).ok_or(Status::Overflow)?;
	if at_price >= ROUND_FOR_TRADER_BELOW {
		return Ok(at_price);
	}
	Ok(at_price.max(limit.quote_ceil(fill).unwrap_or(0)))
}

/// Hand the builder what a round left over, or take from the builder what it fell short by.
fn settle_builder<H: Host>(ctx: &mut Ctx<H>, b: &BatchRec, paid_in: u128, paid_out: u128) -> Res {
	if paid_in >= paid_out {
		pay_fee(ctx, &b.fee_account, b.quote, u64::try_from(paid_in - paid_out).map_err(|_| Status::Overflow)?)
	} else {
		// Rounding went the traders' way by more than the fees brought in. A builder who
		// cannot cover that has built a round that does not settle.
		let short = u64::try_from(paid_out - paid_in).map_err(|_| Status::Overflow)?;
		ctx.debit(&b.fee_account, b.quote, short)
	}
}

/// Settle one auction round.
///
/// The orders arrive from refine authenticated, sorted and crossing. Here, where balances
/// can be seen, the round is completed:
///
/// 1. each order is checked against state and cut down to what its account can fund;
/// 2. the clearing price is found over those funded quantities alone, so an order that
///    cannot pay has no influence on the price;
/// 3. the traded volume is shared by price priority, pro rata at the margin;
/// 4. every buyer pays that one price and every seller receives it.
///
/// For two tokens the swap is atomic: balances move directly between the traders' own
/// accounts. When the base asset lives outside JAM, each matched pair becomes a [`Deal`].
pub fn settle_batch<H: Host>(ctx: &mut Ctx<H>, b: &BatchRec) -> Res {
	let now = ctx.now;
	if b.base == b.quote || b.fee_bps > MAX_FEE_BPS {
		return Err(Status::Invalid);
	}
	let orders = (b.bids.len() + b.asks.len()) as u64;
	gas_check(ctx, orders * GAS_PER_ORDER)?;
	#[cfg(feature = "profile")]
	let mut last = ctx.host.gas();
	let quote = ctx.asset(b.quote).ok_or(Status::NotFound)?;
	if !quote.is_token() {
		return Err(Status::Invalid);
	}
	let quote_permissioned = quote.flags & ASSET_PERMISSIONED != 0;
	let base = ctx.asset(b.base).ok_or(Status::NotFound)?;
	let base_permissioned = base.flags & ASSET_PERMISSIONED != 0;
	let ext = base.external().cloned();
	let m = Mkt {
		base: b.base,
		quote: b.quote,
		fee_bps: b.fee_bps,
		base_permissioned,
		quote_permissioned,
		ext: ext.as_ref(),
		btc: ext.as_ref().is_some_and(|c| matches!(c.rail, Rail::BtcSpv { .. })),
	};

	ctx.reserve(orders as usize);

	// 1. What state says about each order.
	let mut seen: Vec<Seen> = Vec::with_capacity(ctx.account_count() + orders as usize);
	let mut bid_prep: Vec<Prep> = Vec::with_capacity(b.bids.len());
	for o in &b.bids {
		gas_check(ctx, GAS_PER_STEP)?;
		bid_prep.push(prepare(ctx, &m, o, Side::Buy, &mut seen));
	}
	let mut ask_prep: Vec<Prep> = Vec::with_capacity(b.asks.len());
	for o in &b.asks {
		gas_check(ctx, GAS_PER_STEP)?;
		ask_prep.push(prepare(ctx, &m, o, Side::Sell, &mut seen));
	}
	probe!(ctx, last, "prepare");
	let mut held = vec![[0u64; 2]; ctx.account_count()];

	// 2. The price, over funded orders only. Before a price exists, funding is judged at
	// each order's own limit. Orders with nothing behind them are left out entirely, so
	// they cannot even move the midpoint.
	let funded = |orders: &[OrderRec], preps: &[Prep], side: Side, held: &mut [[u64; 2]]| -> Vec<Quote> {
		let mut v = Vec::with_capacity(orders.len());
		for (o, p) in orders.iter().zip(preps) {
			let qty = fund(&m, side, p, &o.limit, held);
			if qty > 0 {
				v.push(Quote { price: o.limit, qty });
			}
		}
		v
	};
	let bid_quotes = funded(&b.bids, &bid_prep, Side::Buy, &mut held);
	let ask_quotes = funded(&b.asks, &ask_prep, Side::Sell, &mut held);
	let price = auction::discover(&bid_quotes, &ask_quotes).ok_or(Status::NoMatch)?.price;
	probe!(ctx, last, "price");

	// 3. With the price known, what each order can fund is exact.
	held.iter_mut().for_each(|h| *h = [0, 0]);
	let capped = |orders: &[OrderRec], preps: &[Prep], side: Side, held: &mut [[u64; 2]]| -> Vec<Quote> {
		orders
			.iter()
			.zip(preps)
			.map(|(o, p)| {
				let within = match side {
					Side::Buy => o.limit.cmp_value(&price) != Ordering::Less,
					Side::Sell => o.limit.cmp_value(&price) != Ordering::Greater,
				};
				Quote { price: o.limit, qty: if within { fund(&m, side, p, &price, held) } else { 0 } }
			})
			.collect()
	};
	let bid_caps = capped(&b.bids, &bid_prep, Side::Buy, &mut held);
	let ask_caps = capped(&b.asks, &ask_prep, Side::Sell, &mut held);
	let sum = |v: &[Quote]| v.iter().map(|x| x.qty as u128).sum::<u128>();
	let volume = u64::try_from(sum(&bid_caps).min(sum(&ask_caps))).unwrap_or(u64::MAX);
	if volume == 0 {
		return Err(Status::NoMatch);
	}
	let bid_fills = auction::allocate(&bid_caps, volume);
	let ask_fills = auction::allocate(&ask_caps, volume);
	probe!(ctx, last, "allocate");

	// 4. Settle.
	let round = Round { b, price, bid_prep: &bid_prep, ask_prep: &ask_prep, bid_fills: &bid_fills, ask_fills: &ask_fills };
	let traded = match &ext {
		None => settle_swap(ctx, &round)?,
		Some(cfg) => open_deals(ctx, &round, cfg)?,
	};
	if traded == 0 {
		return Err(Status::NoMatch);
	}
	probe!(ctx, last, "settle");

	let k = key::market(b.base, b.quote);
	let mut mk: Market =
		ctx.get(&k).unwrap_or(Market { last_price: price, last_slot: now, batches: 0, volume: 0 });
	mk.last_price = price;
	mk.last_slot = now;
	mk.batches += 1;
	mk.volume = mk.volume.saturating_add(traded as u128);
	ctx.put(&k, &mk);
	Ok(())
}

/// A round whose price and fills are decided and which only remains to be settled.
struct Round<'r> {
	b: &'r BatchRec,
	price: Price,
	bid_prep: &'r [Prep],
	ask_prep: &'r [Prep],
	bid_fills: &'r [u64],
	ask_fills: &'r [u64],
}

/// Both assets live here: move them between the traders' accounts in one step.
fn settle_swap<H: Host>(ctx: &mut Ctx<H>, r: &Round) -> Res<u64> {
	let now = ctx.now;
	let b = r.b;
	let (mut quote_in, mut quote_out, mut traded) = (0u128, 0u128, 0u64);
	for ((o, p), f) in b.bids.iter().zip(r.bid_prep).zip(r.bid_fills).filter(|(_, f)| **f > 0) {
		gas_check(ctx, GAS_PER_STEP)?;
		let q = buyer_pays(&r.price, &o.limit, *f)?;
		let cost = q.checked_add(fee_of(q, b.fee_bps)).ok_or(Status::Overflow)?;
		let a = ctx.acct_mut(p.ix as usize);
		take(a, b.quote, cost, now)?;
		a.credit(b.base, *f)?;
		mark_fill(a, o, *f, now)?;
		quote_in += cost as u128;
		traded = traded.checked_add(*f).ok_or(Status::Overflow)?;
	}
	for ((o, p), g) in b.asks.iter().zip(r.ask_prep).zip(r.ask_fills).filter(|(_, g)| **g > 0) {
		gas_check(ctx, GAS_PER_STEP)?;
		let q = seller_gets(&r.price, &o.limit, *g)?;
		let net = q - fee_of(q, b.fee_bps);
		let a = ctx.acct_mut(p.ix as usize);
		take(a, b.base, *g, now)?;
		a.credit(b.quote, net)?;
		mark_fill(a, o, *g, now)?;
		quote_out += net as u128;
	}
	settle_builder(ctx, b, quote_in, quote_out)?;
	Ok(traded)
}

/// The base asset lives elsewhere: pair buyers with sellers and open one deal per pair.
/// The buyer's payment is locked in the deal, together with the seller's bond.
///
/// A pair that cannot become a deal is left out and both orders keep that quantity; it
/// never undoes the other pairs of the round.
fn open_deals<H: Host>(ctx: &mut Ctx<H>, r: &Round, cfg: &ExternalCfg) -> Res<u64> {
	let now = ctx.now;
	let b = r.b;
	let (min_height, pay_until) = match cfg.rail {
		Rail::BtcSpv { pay_secs, .. } => {
			let lc: LightClient = ctx.get(&key::chain(b.base)).ok_or(Status::NotFound)?;
			// Median time past needs eleven blocks. Until the service has seen that many
			// past its starting point, the deal's clock would not be the real one.
			if lc.tip_height < lc.base_height.saturating_add(btc::MTP_SPAN as u32 - 1) {
				return Err(Status::Stale);
			}
			// The window opens now, by JAM's clock, and is measured by Bitcoin's. It does
			// not depend on how recent the service's view of Bitcoin happens to be.
			let unix = JAM_EPOCH_UNIX + now as u64 * SLOT_SECONDS + pay_secs as u64;
			(lc.tip_height, u32::try_from(unix).unwrap_or(u32::MAX))
		},
		Rail::Oracle { .. } => (0, 0),
	};
	let is_btc = matches!(cfg.rail, Rail::BtcSpv { .. });
	let pairs = auction::pair(r.bid_fills, r.ask_fills);
	let mut bid_done = vec![0u64; b.bids.len()];
	let mut ask_done = vec![0u64; b.asks.len()];
	let (mut paid_in, mut locked, mut traded) = (0u128, 0u128, 0u64);
	for (i, j, q) in pairs.into_iter().take(MAX_DEALS_PER_BATCH) {
		gas_check(ctx, GAS_PER_STEP)?;
		let (bid, ask) = (&b.bids[i], &b.asks[j]);
		if q < cfg.min_deal || bid.account == ask.account {
			continue;
		}
		// What the buyer pays and what is locked for the seller, each rounded as in any
		// other round; the difference is the builder's to keep or to make up.
		let pays = buyer_pays(&r.price, &bid.limit, q)?;
		let amount = seller_gets(&r.price, &ask.limit, q)?;
		let bond = fee_of(amount, cfg.bond_bps);
		let cost = pays.checked_add(fee_of(pays, b.fee_bps)).ok_or(Status::Overflow)?;
		let untagged = is_btc && bid.flags & ORDER_NO_TAG != 0;
		let marker = untagged.then(|| key::untagged(b.base, &bid.delivery));
		if marker.as_ref().is_some_and(|k| ctx.exists(k)) {
			// This address already has a deal that any untagged payment settles. A second
			// one would let either seller's payment be claimed for the other's deal.
			continue;
		}
		// Each order was sized as a whole; rounding pair by pair can ask an atom more.
		let (bix, aix) = (r.bid_prep[i].ix as usize, r.ask_prep[j].ix as usize);
		if !can_take(ctx.acct(bix), b.quote, cost, now) || !can_take(ctx.acct(aix), b.quote, bond, now) {
			continue;
		}
		take(ctx.acct_mut(bix), b.quote, cost, now)?;
		take(ctx.acct_mut(aix), b.quote, bond, now)?;
		paid_in += cost as u128;
		locked += amount as u128;
		let id = ctx.config().next_deal;
		ctx.config_mut().next_deal = id.checked_add(1).ok_or(Status::Overflow)?;
		let flags = match (is_btc, untagged) {
			(true, false) => DEAL_REQUIRE_TAG,
			(true, true) => DEAL_UNTAGGED,
			(false, _) => 0,
		};
		ctx.put(
			&key::deal(id),
			&Deal {
				ext_asset: b.base,
				token: b.quote,
				payer: bid.account,
				payee: ask.account,
				ext_amount: q,
				amount,
				bond,
				delivery: bid.delivery.clone(),
				created: now,
				deadline: now.saturating_add(cfg.claim_window),
				min_height,
				pay_until,
				flags,
				votes: 0,
			},
		);
		if let Some(k) = marker {
			ctx.put_raw(&k, id.to_le_bytes().to_vec());
		}
		bid_done[i] += q;
		ask_done[j] += q;
		traded = traded.checked_add(q).ok_or(Status::Overflow)?;
	}
	let sides = [(&b.bids, r.bid_prep, &bid_done), (&b.asks, r.ask_prep, &ask_done)];
	for (orders, preps, done) in sides {
		for ((o, p), f) in orders.iter().zip(preps).zip(done.iter()).filter(|(_, f)| **f > 0) {
			mark_fill(ctx.acct_mut(p.ix as usize), o, *f, now)?;
		}
	}
	settle_builder(ctx, b, paid_in, locked)?;
	Ok(traded)
}

// ---------------------------------------------------------------------------------------
// The header chain
// ---------------------------------------------------------------------------------------

/// Extend the header chain of `asset`. `prev` must already be known; each header must
/// declare exactly the difficulty consensus prescribes after its parent. (Refine has
/// already checked that each one carries the work that difficulty demands.)
///
/// Forks are kept. The best chain is the one with the most total work, and the height
/// index always describes it.
///
/// Headers are taken for as long as the work item's gas lasts; the rest can be sent again.
pub fn add_headers<H: Host>(ctx: &mut Ctx<H>, asset: AssetId, prev: Hash32, headers: &[HeaderDigest]) -> Res {
	let mut lc: LightClient = ctx.get(&key::chain(asset)).ok_or(Status::NotFound)?;
	let mut parent: HeaderRec = ctx.get(&key::header(asset, &prev)).ok_or(Status::NotFound)?;
	let mut parent_hash = prev;
	let mut work_memo: Option<(u32, u128)> = None;
	let mut accepted = 0;
	for h in headers {
		if gas_check(ctx, GAS_PER_STEP).is_err() {
			if accepted == 0 {
				return Err(Status::OutOfGas);
			}
			break;
		}
		let expected =
			btc::next_bits(&lc.params, parent.height, parent.bits, parent.time, parent.period_start);
		if h.bits != expected {
			break;
		}
		let k = key::header(asset, &h.hash);
		let rec = match ctx.get::<HeaderRec>(&k) {
			Some(known) => known,
			None => {
				let work = match work_memo {
					Some((bits, w)) if bits == h.bits => w,
					_ => btc::work_from_compact(h.bits).ok_or(Status::Invalid)?,
				};
				work_memo = Some((h.bits, work));
				let height = parent.height.checked_add(1).ok_or(Status::Overflow)?;
				let rec = HeaderRec {
					height,
					prev: parent_hash,
					merkle_root: h.merkle_root,
					time: h.time,
					bits: h.bits,
					chainwork: parent.chainwork.checked_add(work).ok_or(Status::Overflow)?,
					period_start: if lc.params.retarget && height % btc::RETARGET_INTERVAL == 0 {
						h.time
					} else {
						parent.period_start
					},
				};
				ctx.put(&k, &rec);
				rec
			},
		};
		if rec.chainwork > lc.tip_work {
			set_tip(ctx, asset, &mut lc, &h.hash, &rec)?;
		}
		parent = rec;
		parent_hash = h.hash;
		accepted += 1;
	}
	if accepted == 0 {
		return Err(Status::Mismatch);
	}
	ctx.put(&key::chain(asset), &lc);
	Ok(())
}

/// Make `hash` the best tip and bring the height index into line with its chain.
fn set_tip<H: Host>(ctx: &mut Ctx<H>, asset: AssetId, lc: &mut LightClient, hash: &Hash32, rec: &HeaderRec) -> Res {
	let mut cur_hash = *hash;
	let mut cur = rec.clone();
	let mut steps = 0;
	loop {
		let k = key::height(asset, cur.height);
		if ctx.get::<Hash32>(&k) == Some(cur_hash) {
			break; // from here down the index already agrees
		}
		ctx.put(&k, &cur_hash);
		if cur.height <= lc.base_height {
			break;
		}
		steps += 1;
		if steps > MAX_REORG {
			return Err(Status::Invalid);
		}
		if steps > 1 {
			// A reorganisation: every further step is another read and another write.
			gas_check(ctx, GAS_PER_STEP)?;
		}
		let parent: HeaderRec = ctx.get(&key::header(asset, &cur.prev)).ok_or(Status::NotFound)?;
		cur_hash = cur.prev;
		cur = parent;
	}
	// A heavier chain can be shorter: drop index entries above the new tip.
	for h in rec.height + 1..=lc.tip_height {
		ctx.del(&key::height(asset, h));
	}
	lc.tip = *hash;
	lc.tip_height = rec.height;
	lc.tip_work = rec.chainwork;
	Ok(())
}

/// Confirmations of the block `hash` on the best chain, or `None` if it is not on it.
pub fn confirmations<H: Host>(ctx: &mut Ctx<H>, asset: AssetId, hash: &Hash32) -> Option<u32> {
	let hdr: HeaderRec = ctx.get(&key::header(asset, hash))?;
	let lc: LightClient = ctx.get(&key::chain(asset))?;
	(ctx.get::<Hash32>(&key::height(asset, hdr.height)) == Some(*hash))
		.then(|| lc.tip_height.saturating_sub(hdr.height) + 1)
}

/// Release a deal on proof that the payment happened.
///
/// Refine established that a transaction with this output exists under `merkle_root`, at
/// the depth of the block's own coinbase. Here the root is matched to a header on the best
/// chain, buried under the required number of blocks and mined inside the deal's payment
/// window, and the output is matched to the deal.
pub fn claim_payment<H: Host>(ctx: &mut Ctx<H>, p: &PaymentRec) -> Res {
	gas_check(ctx, GAS_PER_TIME_CHECK)?;
	let d: Deal = ctx.get(&key::deal(p.deal)).ok_or(Status::NotFound)?;
	let cfg = ctx.asset(d.ext_asset).and_then(|a| a.external().cloned()).ok_or(Status::NotFound)?;
	let Rail::BtcSpv { confirmations: required, .. } = cfg.rail else { return Err(Status::Invalid) };
	let hdr: HeaderRec = ctx.get(&key::header(d.ext_asset, &p.block_hash)).ok_or(Status::NotFound)?;
	if hdr.merkle_root != p.merkle_root {
		return Err(Status::Mismatch);
	}
	match confirmations(ctx, d.ext_asset, &p.block_hash) {
		Some(c) if c >= required as u32 => {},
		_ => return Err(Status::Unconfirmed),
	}
	if hdr.height <= d.min_height {
		// The block predates the deal: this payment was not made for it.
		return Err(Status::TooEarly);
	}
	if median_time_past(ctx, d.ext_asset, &hdr) > d.pay_until {
		// Paid, but after the window had closed.
		return Err(Status::Expired);
	}
	if p.script != d.delivery {
		return Err(Status::Mismatch);
	}
	if p.value < d.ext_amount {
		return Err(Status::Insufficient);
	}
	match p.tag {
		TagState::This => {},
		// A payment that names another deal settles that deal and no other. Without this, a
		// buyer could open a second, tiny deal to the same address, have the seller's
		// payment presented for it, and leave the seller's own deal unprovable.
		TagState::Other => return Err(Status::Mismatch),
		TagState::None if d.flags & DEAL_REQUIRE_TAG != 0 => return Err(Status::Mismatch),
		TagState::None => {},
	}
	// One output settles one deal, ever.
	let used = key::spent(d.ext_asset, &p.txid, p.vout);
	if ctx.exists(&used) {
		return Err(Status::Exists);
	}
	ctx.put_raw(&used, p.deal.to_le_bytes().to_vec());
	close_deal(ctx, p.deal, &d, Outcome::Paid)
}
