//! `jamx`: wallet, bundler and carrier for the JAMX service.
//!
//! Every command that changes state builds a payload, hands it to JAM through `jamt` as one
//! work item, waits for the service to accumulate it and prints the receipt.

mod chain;
mod local;

use chain::{Chain, Result};
use clap::{Parser, Subcommand, ValueEnum};
use jamx_core::{
	btc::{self, HeaderRec, LightClient},
	codec::Codec,
	crypto::blake2b,
	ops::*,
	state::key,
	types::*,
};
use local::{demo_script, dev_key, parse_price, symbol, Key, Local, Regtest};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "jamx", version, about = "Tools for the JAMX non-custodial exchange service")]
struct Cli {
	/// WebSocket RPC of a JAM node.
	#[arg(long, env = "JAM_RPC", default_value = "ws://localhost:19800", global = true)]
	rpc: String,
	/// Service identifier of the JAMX instance (hex, as `jamt` prints it).
	#[arg(long, env = "JAMX_SERVICE", global = true)]
	service: Option<String>,
	/// Path of the `jamt` tool.
	#[arg(long, env = "JAMT", default_value = "jamt", global = true)]
	jamt: PathBuf,
	/// Directory for local state (names, nonces, queue, order book, regtest chain).
	#[arg(long, env = "JAMX_DIR", default_value = ".jamx", global = true)]
	dir: PathBuf,
	/// Do not send now: add to the local queue. `jamx submit` sends the queue as one payload.
	#[arg(long, short, global = true)]
	queue: bool,
	/// Sign with this key instead of the acting account's own (a second device, a new owner key).
	#[arg(long, global = true)]
	key: Option<String>,
	/// Submit on this core (default: wherever `jamt` puts it).
	#[arg(long, global = true)]
	core: Option<u16>,
	/// Gas for refine (default: all the package may use).
	#[arg(long, global = true)]
	refine_gas: Option<u64>,
	/// Gas for accumulate (default: all the package may use).
	#[arg(long, global = true)]
	accumulate_gas: Option<u64>,
	/// Slots for which a signed operation stays valid. A signature that is never submitted
	/// must not remain usable for ever.
	#[arg(long, default_value_t = 600, global = true)]
	ttl: Slot,
	#[command(subcommand)]
	cmd: Cmd,
}

#[derive(Clone, Copy, ValueEnum)]
enum SideArg {
	Buy,
	Sell,
}

#[derive(Clone, Copy, ValueEnum)]
enum IssuerArg {
	Allow,
	Disallow,
	Freeze,
	Unfreeze,
}

#[derive(Clone, Copy, ValueEnum)]
enum RoleArg {
	Owner,
	Trader,
}

#[derive(Clone, Copy, ValueEnum)]
enum VerdictArg {
	/// The payment arrived.
	Paid,
	/// It did not: the buyer is refunded and keeps the seller's bond.
	Unpaid,
	/// The destination cannot be paid at all: the deal is undone and nobody is penalised.
	Void,
}

#[derive(Subcommand)]
enum Cmd {
	/// Print the account identifier of a development key.
	Id { name: String },
	/// Read state.
	Show {
		#[command(subcommand)]
		what: Show,
	},
	/// Send what is in the local queue as one payload.
	Submit,
	/// Wait until the chain has advanced by this many slots.
	Wait { slots: u32 },

	// ---- money ----
	/// Start accepting an asset. An account can only be sent what it has accepted.
	Accept { who: String, asset: AssetId },
	/// Stop accepting an asset (its balance must be empty).
	Unaccept { who: String, asset: AssetId },
	Transfer { from: String, asset: AssetId, to: String, amount: u64 },
	/// Create a token whose balances live in the service.
	CreateToken {
		issuer: String,
		symbol: String,
		#[arg(long, default_value_t = 2)]
		decimals: u8,
		/// Only accounts the issuer allows may hold it.
		#[arg(long)]
		permissioned: bool,
		/// The issuer may freeze balances (every use is logged).
		#[arg(long)]
		freezable: bool,
		/// Independent attestors of the reserves; issuance is capped by what they attest.
		#[arg(long)]
		attestor: Vec<String>,
		#[arg(long, default_value_t = 0)]
		threshold: u8,
	},
	/// Define an external Bitcoin-style asset tracked by a header chain.
	CreateBtc {
		creator: String,
		symbol: String,
		/// Local regtest chain whose first block is the light client's starting point.
		#[arg(long)]
		chain: String,
		#[arg(long, default_value_t = 6)]
		confirmations: u8,
		/// Seconds the seller has to get the payment mined, from the moment a deal opens,
		/// measured by Bitcoin's own clock (median time past).
		#[arg(long, default_value_t = 7200)]
		pay_secs: u32,
		/// Further blocks, after the confirmations, before a deal without proof fails.
		#[arg(long, default_value_t = 3)]
		grace_blocks: u16,
		#[arg(long, default_value_t = 1000)]
		bond_bps: u16,
		/// Slots after which a deal still open is undone with nobody penalised (the way out
		/// if the header chain stalls). Default: four times what the blocks should take.
		#[arg(long)]
		window: Option<Slot>,
		#[arg(long, default_value_t = 1000)]
		min_deal: u64,
	},
	/// Define an external asset whose payments are attested by oracles (for instance Pix).
	CreateOracle {
		creator: String,
		symbol: String,
		#[arg(long, required = true)]
		oracle: Vec<String>,
		#[arg(long, default_value_t = 1)]
		threshold: u8,
		#[arg(long, default_value_t = 500)]
		bond_bps: u16,
		#[arg(long, default_value_t = 300)]
		window: Slot,
		#[arg(long, default_value_t = 1)]
		min_deal: u64,
	},
	Mint { issuer: String, asset: AssetId, to: String, amount: u64 },
	Burn { who: String, asset: AssetId, amount: u64 },
	/// State the reserves verified behind an asset (attestors only).
	Attest { attestor: String, asset: AssetId, reserves: u64 },
	/// Issuer powers over a holder of its own asset.
	Issuer {
		issuer: String,
		asset: AssetId,
		action: IssuerArg,
		target: String,
		/// Free text; its hash goes on the public log.
		#[arg(long, default_value = "")]
		reason: String,
	},

	// ---- account rules ----
	/// Change the acting account's own rules.
	Policy {
		who: String,
		#[command(subcommand)]
		change: PolicyCmd,
	},
	/// Withdraw a queued rule change.
	PolicyCancel { who: String, index: u8 },
	/// Apply a queued rule change whose delay has passed (anyone may).
	PolicyExec { account: String, index: u8 },
	Freeze { who: String, target: String },
	Unfreeze { who: String, target: String },
	/// As a recovery guardian, vote for a new owner key.
	Recover { guardian: String, target: String, new_key: String },
	RecoveryCancel { who: String },
	/// Install the recovered key once its delay has passed (anyone may).
	RecoveryExec { account: String },
	/// Cancel orders by nonce.
	Cancel { who: String, nonces: Vec<u64> },

	// ---- trading ----
	/// Sign an order and put it in the local order book (nothing goes on-chain yet).
	Order {
		who: String,
		side: SideArg,
		base: AssetId,
		quote: AssetId,
		/// Quantity of the base asset, in atoms.
		qty: u64,
		/// Limit price in quote atoms per base atom: `num/den` or a decimal.
		price: String,
		#[arg(long)]
		ioc: bool,
		/// Do not require the payment to carry the deal tag.
		#[arg(long)]
		no_tag: bool,
		/// Where the external asset must be delivered: a wallet name (demo script) or hex.
		#[arg(long)]
		deliver: Option<String>,
		#[arg(long, default_value_t = 2000)]
		expires_in: Slot,
		#[arg(long, default_value_t = 100)]
		max_fee_bps: u16,
	},
	/// List the local order book of a market.
	Orders { base: AssetId, quote: AssetId },
	/// Bundle the local order book of a market into one auction round and submit it.
	Batch {
		base: AssetId,
		quote: AssetId,
		#[arg(long)]
		builder: String,
		#[arg(long, default_value_t = 0)]
		fee_bps: u16,
		/// Keep the orders in the local book afterwards.
		#[arg(long)]
		keep: bool,
	},
	/// As the paying side, release a deal without waiting for proof.
	Release { who: String, deal: DealId },
	/// Close a deal whose deadline has passed (anyone may).
	Timeout { deal: DealId },
	/// As an oracle, attest whether a deal's off-chain payment arrived.
	OracleAttest { oracle: String, deal: DealId, verdict: VerdictArg },

	// ---- governance ----
	ProposeUpgrade {
		admin: String,
		code: PathBuf,
		#[arg(long, default_value_t = 10_000)]
		min_item_gas: u64,
		#[arg(long, default_value_t = 10_000)]
		min_memo_gas: u64,
	},
	CancelUpgrade { admin: String },
	/// Hand the administrator role to another account, or to nobody (`none`).
	SetAdmin { admin: String, new_admin: String },
	UpgradeExec,

	/// A local Bitcoin regtest chain, and carrying its data into JAM.
	Btc {
		#[command(subcommand)]
		cmd: BtcCmd,
	},
}

#[derive(Subcommand)]
enum PolicyCmd {
	AddKey { key_name: String, role: RoleArg },
	RemoveKey { key_name: String },
	/// An account that can freeze this one but not spend from it (`none` to remove).
	Guardian { account: String },
	/// Accounts that can together install a new owner key (`--threshold 0` to remove).
	Recovery {
		#[arg(long)]
		threshold: u8,
		#[arg(long, default_value_t = 14_400)]
		delay: Slot,
		guardians: Vec<String>,
	},
	/// Most of an asset that may leave per day (`none` to remove).
	Limit { asset: AssetId, per_day: String },
	/// Slots every weakening change must wait.
	Delay { slots: Slot },
	FreezeSlots { slots: Slot },
}

#[derive(Subcommand)]
enum Show {
	Account { who: String },
	Asset { id: AssetId },
	Config,
	Deals,
	Deal { id: DealId },
	Chain { asset: AssetId },
	Market { base: AssetId, quote: AssetId },
	Receipts,
	Queue,
}

#[derive(Subcommand)]
enum BtcCmd {
	/// Create a local chain with a genesis block.
	Init { name: String },
	/// Mine empty blocks.
	Mine {
		name: String,
		#[arg(default_value_t = 1)]
		blocks: usize,
	},
	/// Pay a deal on the local chain (a transaction to the deal's delivery script) and mine it.
	Pay {
		name: String,
		deal: DealId,
		/// Pay this many satoshis instead of the agreed amount.
		#[arg(long)]
		sats: Option<u64>,
		/// Pay this wallet instead of the deal's delivery script.
		#[arg(long)]
		to: Option<String>,
		/// Leave out the deal tag.
		#[arg(long)]
		no_tag: bool,
	},
	/// Carry the headers JAM has not seen yet into the service.
	Relay { name: String, asset: AssetId },
	/// Carry the receipt for a deal's payment into the service.
	Claim { name: String, deal: DealId },
	/// Print the demo output script of a named wallet.
	Script { wallet: String },
}

struct App {
	chain: Chain,
	local: Local,
	queue: bool,
	sign_with: Option<String>,
	ttl: Slot,
}

fn status_name(s: u8) -> &'static str {
	match s {
		0 => "ok",
		2 => "malformed",
		3 => "unauthorized",
		4 => "nonce already used",
		5 => "frozen",
		6 => "insufficient funds",
		7 => "over the daily limit",
		8 => "not found",
		9 => "not allowed to hold the asset",
		10 => "over the mint cap",
		11 => "invalid",
		12 => "expired",
		13 => "too early",
		14 => "already exists",
		15 => "overflow",
		16 => "nothing matched",
		17 => "not confirmed deeply enough",
		18 => "does not match",
		19 => "out of gas",
		20 => "the view of the outside chain is too old",
		0xff => "refine failed",
		_ => "unknown",
	}
}

impl App {
	fn account(&self, id: &AccountId) -> Result<Account> {
		Ok(self.chain.get(&key::account(id))?.unwrap_or_default())
	}

	/// Send one item now, or queue it.
	fn send(&self, item: Item) -> Result<()> {
		if self.queue {
			let n = self.local.enqueue(&item)?;
			println!("queued ({n} item(s) waiting; send with `jamx submit`)");
			return Ok(());
		}
		self.submit(&encode_payload(&[item]), 1)
	}

	fn submit(&self, payload: &[u8], items: usize) -> Result<()> {
		let r = self.chain.submit(payload)?;
		let names: Vec<&str> = r.statuses.iter().map(|s| status_name(*s)).collect();
		println!("accumulated at slot {}: {}", r.slot, names.join(", "));
		if r.statuses.len() < items {
			println!(
				"  note: {} of {items} item(s) produced no record (rejected by refine: bad signature or malformed)",
				items - r.statuses.len()
			);
		}
		if r.statuses.iter().any(|s| *s != 0) || r.statuses.len() < items {
			return Err("not everything was applied".into());
		}
		Ok(())
	}

	/// Sign an operation for `actor`'s account and send it.
	fn op(&self, actor: &str, op: Op) -> Result<()> {
		let account = self.local.account(actor)?;
		let signer: Key = match &self.sign_with {
			Some(k) => self.local.key(k)?,
			None => self.local.key(actor)?,
		};
		let nonce = self.local.next_nonce(&account, &self.account(&account)?)?;
		let env = Envelope { account, nonce, expiry: self.chain.slot()?.saturating_add(self.ttl), op };
		self.send(Item::Signed(SignedOp::sign(&signer.seed, self.chain.service, &env)))
	}

	fn plain(&self, op: PlainOp) -> Result<()> {
		self.send(Item::Plain(op))
	}
}

fn opt_account(local: &Local, s: &str) -> Result<Option<AccountId>> {
	if s == "none" {
		Ok(None)
	} else {
		local.account(s).map(Some)
	}
}

fn run(cli: Cli) -> Result<()> {
	let local = Local::new(cli.dir.clone())?;
	if let Cmd::Id { name } = &cli.cmd {
		println!("{}", hex::encode(local.key(name)?.id));
		return Ok(());
	}
	if let Cmd::Btc { cmd } = &cli.cmd {
		if let Some(done) = btc_offline(&local, cmd) {
			return done;
		}
	}
	let service = cli
		.service
		.as_deref()
		.ok_or("no service: pass --service or set JAMX_SERVICE")
		.and_then(|s| u32::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| "bad service id"))?;
	let app = App {
		chain: Chain {
			rpc: cli.rpc,
			service,
			jamt: cli.jamt,
			dir: cli.dir,
			core: cli.core,
			refine_gas: cli.refine_gas,
			accumulate_gas: cli.accumulate_gas,
		},
		local,
		queue: cli.queue,
		sign_with: cli.key,
		ttl: cli.ttl,
	};
	let l = &app.local;

	match cli.cmd {
		Cmd::Id { .. } => unreachable!(),
		Cmd::Show { what } => show(&app, what),
		Cmd::Submit => {
			let (payload, n) = l.drain_queue()?;
			if n == 0 {
				return Err("the queue is empty".into());
			}
			println!("sending {n} item(s), {} bytes", payload.len());
			app.submit(&payload, n)
		},
		Cmd::Wait { slots } => {
			let now = app.chain.wait_slots(slots)?;
			println!("slot {now}");
			Ok(())
		},

		Cmd::Transfer { from, asset, to, amount } =>
			app.op(&from, Op::Transfer { asset, to: l.account(&to)?, amount }),
		Cmd::CreateToken { issuer, symbol: s, decimals, permissioned, freezable, attestor, threshold } => {
			let flags = (permissioned as u8 * ASSET_PERMISSIONED) | (freezable as u8 * ASSET_FREEZABLE);
			let attestors = attestor.iter().map(|a| l.account(a)).collect::<Result<Vec<_>>>()?;
			println!("asset id will be {}", app.chain.config()?.next_asset);
			app.op(
				&issuer,
				Op::AssetCreate { symbol: symbol(&s)?, decimals, flags, kind: AssetKindCfg::Token, attestors, threshold },
			)
		},
		Cmd::Accept { who, asset } => app.op(&who, Op::Accept { asset, on: true }),
		Cmd::Unaccept { who, asset } => app.op(&who, Op::Accept { asset, on: false }),
		Cmd::CreateBtc { creator, symbol: s, chain, confirmations, pay_secs, grace_blocks, bond_bps, window, min_deal } => {
			let c = l.load_chain(&chain)?;
			let blocks = 6 + confirmations as u32 + grace_blocks as u32;
			let window = window.unwrap_or(pay_secs / 6 + blocks * 400);
			println!("asset id will be {}", app.chain.config()?.next_asset);
			app.op(
				&creator,
				Op::AssetCreate {
					symbol: symbol(&s)?,
					decimals: 8,
					flags: 0,
					kind: AssetKindCfg::External {
						cfg: ExternalCfg {
							rail: Rail::BtcSpv { confirmations, pay_secs, grace_blocks },
							bond_bps,
							claim_window: window,
							min_deal,
						},
						btc: Some((btc::REGTEST, c.checkpoint())),
					},
					attestors: vec![],
					threshold: 0,
				},
			)
		},
		Cmd::CreateOracle { creator, symbol: s, oracle, threshold, bond_bps, window, min_deal } => {
			let oracles = oracle.iter().map(|a| l.account(a)).collect::<Result<Vec<_>>>()?;
			println!("asset id will be {}", app.chain.config()?.next_asset);
			app.op(
				&creator,
				Op::AssetCreate {
					symbol: symbol(&s)?,
					decimals: 2,
					flags: 0,
					kind: AssetKindCfg::External {
						cfg: ExternalCfg { rail: Rail::Oracle { oracles, threshold }, bond_bps, claim_window: window, min_deal },
						btc: None,
					},
					attestors: vec![],
					threshold: 0,
				},
			)
		},
		Cmd::Mint { issuer, asset, to, amount } => app.op(&issuer, Op::Mint { asset, to: l.account(&to)?, amount }),
		Cmd::Burn { who, asset, amount } => app.op(&who, Op::Burn { asset, amount }),
		Cmd::Attest { attestor, asset, reserves } => app.op(&attestor, Op::Attest { asset, reserves }),
		Cmd::Issuer { issuer, asset, action, target, reason } => {
			let action = match action {
				IssuerArg::Allow => IssuerAction::Allow,
				IssuerArg::Disallow => IssuerAction::Disallow,
				IssuerArg::Freeze => IssuerAction::Freeze,
				IssuerArg::Unfreeze => IssuerAction::Unfreeze,
			};
			app.op(
				&issuer,
				Op::IssuerSet { asset, target: l.account(&target)?, action, reason: blake2b(&[reason.as_bytes()]) },
			)
		},

		Cmd::Policy { who, change } => {
			let change = match change {
				PolicyCmd::AddKey { key_name, role } => PolicyChange::AddKey {
					key: dev_key(&key_name).id,
					role: match role {
						RoleArg::Owner => ROLE_OWNER,
						RoleArg::Trader => ROLE_TRADER,
					},
				},
				PolicyCmd::RemoveKey { key_name } => PolicyChange::RemoveKey { key: dev_key(&key_name).id },
				PolicyCmd::Guardian { account } => PolicyChange::SetGuardian(opt_account(l, &account)?),
				PolicyCmd::Recovery { threshold, delay, guardians } => PolicyChange::SetRecovery(if threshold == 0 {
					None
				} else {
					Some(RecoveryCfg {
						guardians: guardians.iter().map(|g| l.account(g)).collect::<Result<Vec<_>>>()?,
						threshold,
						delay,
					})
				}),
				PolicyCmd::Limit { asset, per_day } => PolicyChange::SetLimit {
					asset,
					per_day: if per_day == "none" { None } else { Some(per_day.parse().map_err(|_| "bad amount")?) },
				},
				PolicyCmd::Delay { slots } => PolicyChange::SetDelay(slots),
				PolicyCmd::FreezeSlots { slots } => PolicyChange::SetFreezeSlots(slots),
			};
			app.op(&who, Op::Policy(change))
		},
		Cmd::PolicyCancel { who, index } => app.op(&who, Op::PolicyCancel { index }),
		Cmd::PolicyExec { account, index } => app.plain(PlainOp::PolicyExec { account: l.account(&account)?, index }),
		Cmd::Freeze { who, target } => app.op(&who, Op::Freeze { target: l.account(&target)? }),
		Cmd::Unfreeze { who, target } => app.op(&who, Op::Unfreeze { target: l.account(&target)? }),
		Cmd::Recover { guardian, target, new_key } => app.op(
			&guardian,
			Op::RecoveryApprove { target: l.account(&target)?, new_owner: l.key(&new_key)?.id },
		),
		Cmd::RecoveryCancel { who } => app.op(&who, Op::RecoveryCancel),
		Cmd::RecoveryExec { account } => app.plain(PlainOp::RecoveryExec { account: l.account(&account)? }),
		Cmd::Cancel { who, nonces } => app.op(&who, Op::Cancel { nonces }),

		Cmd::Order { who, side, base, quote, qty, price, ioc, no_tag, deliver, expires_in, max_fee_bps } => {
			let account = l.account(&who)?;
			let signer = match &app.sign_with {
				Some(k) => l.key(k)?,
				None => l.key(&who)?,
			};
			let delivery = match deliver {
				None => vec![],
				Some(d) => hex::decode(&d).ok().filter(|b| b.len() >= 4).unwrap_or_else(|| demo_script(&d)),
			};
			let order = Order {
				account,
				nonce: l.next_nonce(&account, &app.account(&account)?)?,
				side: match side {
					SideArg::Buy => Side::Buy,
					SideArg::Sell => Side::Sell,
				},
				qty,
				limit: parse_price(&price)?,
				expiry: app.chain.slot()? + expires_in,
				max_fee_bps,
				flags: (ioc as u8 * ORDER_IOC) | (no_tag as u8 * ORDER_NO_TAG),
				delivery,
			};
			let signed = SignedOrder::sign(&signer.seed, app.chain.service, base, quote, order);
			l.add_order(base, quote, &signed)?;
			println!(
				"order signed and added to the local book of market {base}/{quote} (nonce {}); nothing is on-chain yet",
				signed.order.nonce
			);
			Ok(())
		},
		Cmd::Orders { base, quote } => {
			for o in l.orders(base, quote)? {
				let o = o.order;
				println!(
					"{:<10} {:<4} {:>12} @ {}/{}  nonce {} expires {}",
					l.label(&o.account),
					if o.side == Side::Buy { "buy" } else { "sell" },
					o.qty,
					o.limit.num,
					o.limit.den,
					o.nonce,
					o.expiry
				);
			}
			Ok(())
		},
		Cmd::Batch { base, quote, builder, fee_bps, keep } => {
			let orders = l.orders(base, quote)?;
			if orders.is_empty() {
				return Err("the local order book of this market is empty".into());
			}
			println!("bundling {} order(s) of market {base}/{quote}", orders.len());
			let item = Item::Batch(Batch { base, quote, fee_account: l.account(&builder)?, fee_bps, orders });
			let before = app.chain.get::<Market>(&key::market(base, quote))?;
			let res = app.send(item);
			if !app.queue {
				if !keep {
					l.clear_orders(base, quote);
				}
				if let Some(m) = app.chain.get::<Market>(&key::market(base, quote))? {
					if before.map_or(true, |b| b.batches != m.batches) {
						println!(
							"cleared at {}; market total: {} round(s), volume {}",
							price_str(&m.last_price),
							m.batches,
							m.volume
						);
					}
				}
			}
			res
		},
		Cmd::Release { who, deal } => app.op(&who, Op::DealRelease { deal }),
		Cmd::Timeout { deal } => app.plain(PlainOp::DealTimeout { deal }),
		Cmd::OracleAttest { oracle, deal, verdict } =>
			app.op(
				&oracle,
				Op::OracleAttest {
					deal,
					verdict: match verdict {
						VerdictArg::Paid => Verdict::Paid,
						VerdictArg::Unpaid => Verdict::NotPaid,
						VerdictArg::Void => Verdict::Void,
					},
				},
			),

		Cmd::ProposeUpgrade { admin, code, min_item_gas, min_memo_gas } => {
			let blob = std::fs::read(&code).map_err(|e| e.to_string())?;
			let code_hash = blake2b(&[&blob]);
			println!("code hash {} ({} bytes)", hex::encode(code_hash), blob.len());
			app.op(
				&admin,
				Op::GovProposeUpgrade { code_hash, code_len: blob.len() as u32, min_item_gas, min_memo_gas },
			)
		},
		Cmd::CancelUpgrade { admin } => app.op(&admin, Op::GovCancelUpgrade),
		Cmd::SetAdmin { admin, new_admin } => app.op(&admin, Op::GovSetAdmin { admin: opt_account(l, &new_admin)? }),
		Cmd::UpgradeExec => app.plain(PlainOp::GovExecUpgrade),

		Cmd::Btc { cmd } => btc_online(&app, cmd),
	}
}

/// The `btc` commands that only touch the local chain file.
fn btc_offline(local: &Local, cmd: &BtcCmd) -> Option<Result<()>> {
	Some(match cmd {
		BtcCmd::Init { name } => (|| {
			let seed = u32::from_le_bytes(blake2b(&[name.as_bytes()])[..4].try_into().unwrap_or([0; 4]));
			let now = std::time::SystemTime::now()
				.duration_since(std::time::UNIX_EPOCH)
				.map_err(|e| e.to_string())?
				.as_secs() as u32;
			let c = Regtest::genesis(seed, now);
			local.save_chain(name, &c)?;
			println!("chain '{name}' created; genesis {}", hex::encode(c.tip()));
			Ok(())
		})(),
		BtcCmd::Mine { name, blocks } => (|| {
			let mut c = local.load_chain(name)?;
			for _ in 0..*blocks {
				c.mine(vec![]);
			}
			local.save_chain(name, &c)?;
			println!("chain '{name}' is now at height {}", c.blocks.len() - 1);
			Ok(())
		})(),
		BtcCmd::Script { wallet } => {
			println!("{}", hex::encode(demo_script(wallet)));
			Ok(())
		},
		_ => return None,
	})
}

fn btc_online(app: &App, cmd: BtcCmd) -> Result<()> {
	let l = &app.local;
	match cmd {
		BtcCmd::Init { .. } | BtcCmd::Mine { .. } | BtcCmd::Script { .. } => unreachable!(),
		BtcCmd::Pay { name, deal, sats, to, no_tag } => {
			let d: Deal = app.chain.get(&key::deal(deal))?.ok_or("no such deal")?;
			let mut c = l.load_chain(&name)?;
			let script = to.map(|w| demo_script(&w)).unwrap_or(d.delivery.clone());
			let mut outs = vec![(sats.unwrap_or(d.ext_amount), script)];
			if !no_tag {
				outs.push((0, btc::tag_script(&deal_tag(app.chain.service, deal))));
			}
			let tx = btc::build_tx(deal as u32, &outs);
			c.mine(vec![tx]);
			l.save_chain(&name, &c)?;
			println!(
				"paid {} sats in block {} of chain '{name}' (from the seller's own wallet; JAM is not involved)",
				outs[0].0,
				c.blocks.len() - 1
			);
			Ok(())
		},
		BtcCmd::Relay { name, asset } => {
			let c = l.load_chain(&name)?;
			// The service takes as many headers from a work item as the item's gas pays
			// for (about 75 at present), so carry them in modest runs and look again
			// after each to see how far it got.
			const RUN: usize = 70;
			loop {
				let lc: LightClient = app.chain.get(&key::chain(asset))?.ok_or("that asset has no header chain")?;
				// Find the newest block JAM already knows that is also on our chain.
				let mut known = None;
				for (h, b) in c.blocks.iter().enumerate().rev() {
					if app.chain.read(&key::header(asset, &b.hash()))?.is_some() {
						known = Some(h);
						break;
					}
				}
				let from = known.ok_or("JAM's header chain and the local chain have nothing in common")? + 1;
				if from >= c.blocks.len() {
					println!("nothing more to carry: JAM is at height {}", lc.tip_height);
					return Ok(());
				}
				let to = (from + RUN).min(c.blocks.len());
				let headers: Vec<[u8; 80]> = c.blocks[from..to].iter().map(|b| b.header.serialize()).collect();
				println!("carrying {} header(s), heights {}..={}", headers.len(), from, to - 1);
				app.send(Item::BtcHeaders { asset, headers })?;
				if app.queue {
					return Ok(());
				}
			}
		},
		BtcCmd::Claim { name, deal } => {
			let d: Deal = app.chain.get(&key::deal(deal))?.ok_or("no such deal (already settled?)")?;
			let c = l.load_chain(&name)?;
			let tag = deal_tag(app.chain.service, deal);
			let (block, tx, vout) = c
				.find_payment(&d.delivery, d.ext_amount, &tag, d.flags & DEAL_REQUIRE_TAG != 0, d.min_height as usize)
				.ok_or("no matching payment on the local chain")?;

			println!("payment found in block {block}, transaction {tx}, output {vout}; carrying the receipt");
			app.send(Item::BtcProof(c.proof(deal, block, tx, vout)))
		},
	}
}

/// A price as a reduced fraction and as a decimal, for people.
fn price_str(p: &Price) -> String {
	let (mut a, mut b) = (p.num, p.den);
	while b != 0 {
		(a, b) = (b, a % b);
	}
	let g = a.max(1);
	format!("{}/{} ({:.6} quote per base)", p.num / g, p.den / g, p.num as f64 / p.den as f64)
}

fn sym_str(s: &[u8; 8]) -> String {
	String::from_utf8_lossy(s).trim_end_matches('\0').to_string()
}

fn show(app: &App, what: Show) -> Result<()> {
	let l = &app.local;
	let c = &app.chain;
	match what {
		Show::Account { who } => {
			let id = l.account(&who)?;
			let a = app.account(&id)?;
			let now = c.slot()?;
			println!("account {} ({})", l.label(&id), hex::encode(id));
			if a.is_empty() {
				println!("  (no state)");
				return Ok(());
			}
			for b in &a.balances {
				let sym = c.get::<Asset>(&key::asset(b.asset))?.map(|x| sym_str(&x.symbol)).unwrap_or_default();
				let mut tags = vec![];
				if b.flags & BAL_ALLOWED != 0 {
					tags.push("allowed");
				}
				if b.flags & BAL_FROZEN != 0 {
					tags.push("FROZEN BY ISSUER");
				}
				println!("  asset {:<3} {:<8} {:>16}  {}", b.asset, sym, b.free, tags.join(", "));
			}
			println!("  nonce floor {}", a.nonce_floor);
			for o in &a.open {
				println!("  resting order nonce {}: {} filled, expires {}", o.nonce, o.filled, o.expiry);
			}
			if let Some(p) = &a.policy {
				for k in &p.keys {
					println!(
						"  key {} ({})",
						l.label(&k.key),
						if k.role == ROLE_OWNER { "owner" } else { "trader" }
					);
				}
				if let Some(g) = &p.guardian {
					println!("  guardian (can freeze, cannot spend): {}", l.label(g));
				}
				if let Some(r) = &p.recovery {
					let names: Vec<String> = r.guardians.iter().map(|g| l.label(g)).collect();
					println!("  recovery: {} of [{}], delay {} slots", r.threshold, names.join(", "), r.delay);
				}
				if let Some(r) = &p.recovery_req {
					let votes: Vec<String> = r.votes.iter().map(|v| format!("#{}→{}", v.guardian, l.label(&v.key))).collect();
					println!("  recovery in progress: votes [{}], takes effect at {}", votes.join(", "), r.eta);
				}
				println!("  security delay {} slots; freeze lasts {} slots", p.delay, p.freeze_slots);
				if p.is_frozen(now) {
					println!("  FROZEN until slot {} (now {})", p.frozen_until, now);
				}
				for lim in &p.limits {
					println!("  daily limit on asset {}: {} (spent {})", lim.asset, lim.per_day, lim.spent);
				}
				for (i, q) in p.pending.iter().enumerate() {
					println!("  queued change #{i}, applicable at slot {}: {:?}", q.eta, q.change);
				}
			}
			Ok(())
		},
		Show::Asset { id } => {
			let a: Asset = c.get(&key::asset(id))?.ok_or("no such asset")?;
			println!("asset {id}: {} (issuer {})", sym_str(&a.symbol), l.label(&a.issuer));
			match &a.kind {
				AssetKind::Token => {
					let cap = a.mint_cap();
					println!(
						"  token, {} decimals, supply {}, mint cap {}",
						a.decimals,
						a.supply,
						if cap == u64::MAX { "none".to_string() } else { cap.to_string() }
					);
					println!(
						"  permissioned: {}, freezable: {} (freeze log: {} entries)",
						a.flags & ASSET_PERMISSIONED != 0,
						a.flags & ASSET_FREEZABLE != 0,
						a.freeze_log
					);
					for (who, v) in a.attestors.iter().zip(&a.attested) {
						println!("  attestor {} vouches for {}", l.label(who), v);
					}
					if !a.attestors.is_empty() {
						println!("  cap = the largest figure at least {} attestor(s) vouch for", a.threshold);
					}
				},
				AssetKind::External(cfg) => {
					match &cfg.rail {
						Rail::BtcSpv { confirmations, pay_secs, grace_blocks } => {
							println!("  external, proven by block headers and Merkle receipts, {confirmations} confirmations");
							println!(
								"  payment due within {pay_secs} s of the deal, by Bitcoin's clock; a deal without proof fails {} blocks after that",
								*confirmations as u32 + *grace_blocks as u32
							);
						},
						Rail::Oracle { oracles, threshold } => {
							let names: Vec<String> = oracles.iter().map(|o| l.label(o)).collect();
							println!("  external, attested by {threshold} of [{}]", names.join(", "));
						},
					}
					let window = match &cfg.rail {
						Rail::BtcSpv { .. } => "slots until an open deal is undone without penalty",
						Rail::Oracle { .. } => "slots to deliver",
					};
					println!(
						"  seller's bond {}%, {} {window}, smallest deal {}",
						cfg.bond_bps as f64 / 100.0,
						cfg.claim_window,
						cfg.min_deal
					);
				},
			}
			Ok(())
		},
		Show::Config => {
			println!("service {:08x}", c.service);
			if c.read(key::CONFIG)?.is_none() {
				println!("  no state yet: nothing has been accumulated. The administrator and upgrade delay");
				println!("  compiled into the code are recorded when the first work item is.");
				return Ok(());
			}
			let cfg = c.config()?;
			match cfg.admin {
				Some(a) => println!("  administrator: {} (upgrades wait {} slots)", l.label(&a), cfg.upgrade_delay),
				None => println!("  no administrator: the code can never be changed"),
			}
			if let Some(u) = cfg.upgrade {
				println!("  UPGRADE ANNOUNCED: code {} applicable at slot {}", hex::encode(u.code_hash), u.eta);
			}
			println!("  next asset {}, next deal {}, receipts {}", cfg.next_asset, cfg.next_deal, cfg.receipt_seq);
			Ok(())
		},
		Show::Deals => {
			let cfg = c.config()?;
			let mut any = false;
			for id in 1..cfg.next_deal {
				if let Some(d) = c.get::<Deal>(&key::deal(id))? {
					print_deal(app, id, &d)?;
					any = true;
				}
			}
			if !any {
				println!("no open deals");
			}
			Ok(())
		},
		Show::Deal { id } => match c.get::<Deal>(&key::deal(id))? {
			Some(d) => print_deal(app, id, &d),
			None => {
				println!("deal {id} is not open (settled, timed out, or never existed)");
				Ok(())
			},
		},
		Show::Chain { asset } => {
			let lc: LightClient = c.get(&key::chain(asset))?.ok_or("that asset has no header chain")?;
			println!("header chain of asset {asset}");
			println!("  tip {} at height {}", hex::encode(lc.tip), lc.tip_height);
			println!("  total work since the checkpoint (height {}): {}", lc.base_height, lc.tip_work);
			if let Some(h) = c.get::<HeaderRec>(&key::header(asset, &lc.tip))? {
				println!("  tip time {}, difficulty bits {:#010x}", h.time, h.bits);
			}
			Ok(())
		},
		Show::Market { base, quote } => {
			match c.get::<Market>(&key::market(base, quote))? {
				None => println!("market {base}/{quote} has not traded yet"),
				Some(m) => println!(
					"market {base}/{quote}: last price {} at slot {}, {} round(s), volume {}",
					price_str(&m.last_price),
					m.last_slot,
					m.batches,
					m.volume
				),
			}
			Ok(())
		},
		Show::Receipts => {
			let seq = c.config()?.receipt_seq;
			for back in (1..=8u32.min(seq)).rev() {
				let idx = ((seq - back) % jamx_core::accumulate::RECEIPT_RING) as u8;
				if let Some(r) = c.get::<jamx_core::accumulate::Receipt>(&key::receipt(idx))? {
					let names: Vec<&str> = r.statuses.iter().map(|s| status_name(*s)).collect();
					println!("slot {} payload {}…: {}", r.slot, hex::encode(&r.payload_hash[..6]), names.join(", "));
				}
			}
			Ok(())
		},
		Show::Queue => {
			let lines = std::fs::read_to_string(l.dir.join("queue")).unwrap_or_default();
			let mut n = 0;
			for line in lines.lines() {
				match local::decode_item(line) {
					Some(Item::Signed(s)) => match Envelope::from_bytes(&s.body) {
						Ok(e) => println!("{n}: {} nonce {}: {:?}", l.label(&e.account), e.nonce, e.op),
						Err(_) => println!("{n}: (malformed)"),
					},
					Some(Item::Batch(b)) => println!("{n}: batch {}/{} with {} orders", b.base, b.quote, b.orders.len()),
					Some(Item::BtcHeaders { asset, headers }) => println!("{n}: {} header(s) for asset {asset}", headers.len()),
					Some(Item::BtcProof(p)) => println!("{n}: receipt for deal {}", p.deal),
					Some(Item::Plain(p)) => println!("{n}: {p:?}"),
					None => println!("{n}: (unreadable)"),
				}
				n += 1;
			}
			if n == 0 {
				println!("the queue is empty");
			}
			Ok(())
		},
	}
}

fn print_deal(app: &App, id: DealId, d: &Deal) -> Result<()> {
	let l = &app.local;
	let now = app.chain.slot()?;
	println!("deal {id}");
	println!(
		"  {} locked {} of asset {}; {} must deliver {} of external asset {} and posted a bond of {}",
		l.label(&d.payer),
		d.amount,
		d.token,
		l.label(&d.payee),
		d.ext_amount,
		d.ext_asset,
		d.bond
	);
	println!("  deliver to: {}", hex::encode(&d.delivery));
	if d.flags & DEAL_REQUIRE_TAG != 0 {
		println!("  payment must carry tag {}", hex::encode(deal_tag(app.chain.service, id)));
	}
	let left = if now > d.deadline { "PASSED".to_string() } else { format!("{} slots left", d.deadline - now) };
	if d.pay_until != 0 {
		println!(
			"  payment must be mined above height {} and before Unix time {} by Bitcoin's clock",
			d.min_height, d.pay_until
		);
		if let Some(lc) = app.chain.get::<LightClient>(&key::chain(d.ext_asset))? {
			println!("  the service has seen the outside chain up to height {}", lc.tip_height);
		}
		println!("  if the header chain stalls, the deal is undone without penalty after slot {} ({left})", d.deadline);
	} else {
		println!("  deadline slot {} ({left})", d.deadline);
	}
	Ok(())
}

fn main() {
	// Die quietly when the reader of our output goes away (`jamx ... | head`).
	unsafe {
		libc::signal(libc::SIGPIPE, libc::SIG_DFL);
	}
	if let Err(e) = run(Cli::parse()) {
		eprintln!("error: {e}");
		std::process::exit(1);
	}
}
