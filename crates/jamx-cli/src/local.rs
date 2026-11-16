//! What the tools keep on disk between runs: development keys' names, the next nonce per
//! account, a queue of items waiting to be sent, a bundler's order book, and a local
//! Bitcoin regtest chain.

use crate::chain::Result;
use jamx_core::{
	btc,
	codec::{Codec, Reader},
	crypto::{blake2b, ed25519_public, sha256d, Hash32},
	ops::*,
	types::*,
};
use std::{
	collections::BTreeMap,
	fs,
	path::{Path, PathBuf},
};

/// A development key: its seed is derived from a name. Convenient for demonstrations,
/// worthless for real money.
#[derive(Clone)]
pub struct Key {
	pub seed: [u8; 32],
	pub id: AccountId,
}

pub fn dev_key(name: &str) -> Key {
	let seed = blake2b(&[b"jamx/dev-key/", name.as_bytes()]);
	Key { seed, id: ed25519_public(&seed) }
}

pub struct Local {
	pub dir: PathBuf,
}

fn io<T>(r: std::io::Result<T>) -> Result<T> {
	r.map_err(|e| e.to_string())
}

fn read_lines(p: &Path) -> Vec<String> {
	fs::read_to_string(p).map(|s| s.lines().map(String::from).collect()).unwrap_or_default()
}

impl Local {
	pub fn new(dir: PathBuf) -> Result<Self> {
		io(fs::create_dir_all(&dir))?;
		Ok(Self { dir })
	}

	// ---- names ----

	/// Resolve a 64-digit hex account id, or a development key name.
	pub fn account(&self, s: &str) -> Result<AccountId> {
		if s.len() == 64 {
			if let Ok(b) = hex::decode(s) {
				return b.try_into().map_err(|_| "bad account id".to_string());
			}
		}
		Ok(self.key(s)?.id)
	}

	pub fn key(&self, name: &str) -> Result<Key> {
		if name.is_empty() || name.len() > 40 {
			return Err("key names are 1 to 40 characters".into());
		}
		let k = dev_key(name);
		let p = self.dir.join("names");
		let mut names = self.names();
		if names.insert(k.id, name.to_string()).is_none() {
			let text: String =
				names.iter().map(|(id, n)| format!("{}\t{}\n", hex::encode(id), n)).collect();
			io(fs::write(p, text))?;
		}
		Ok(k)
	}

	pub fn names(&self) -> BTreeMap<AccountId, String> {
		read_lines(&self.dir.join("names"))
			.iter()
			.filter_map(|l| {
				let (h, n) = l.split_once('\t')?;
				Some((hex::decode(h).ok()?.try_into().ok()?, n.to_string()))
			})
			.collect()
	}

	/// A name if we know one, otherwise the start of the identifier.
	pub fn label(&self, id: &AccountId) -> String {
		self.names().get(id).cloned().unwrap_or_else(|| format!("{}…", hex::encode(&id[..6])))
	}

	// ---- nonces ----

	/// Pick the next nonce for an account: never below what the chain says is free, never
	/// one this tool has already handed out.
	pub fn next_nonce(&self, id: &AccountId, on_chain: &Account) -> Result<u64> {
		let p = self.dir.join("nonces");
		let mut map: BTreeMap<String, u64> = read_lines(&p)
			.iter()
			.filter_map(|l| {
				let (k, v) = l.split_once(' ')?;
				Some((k.to_string(), v.parse().ok()?))
			})
			.collect();
		let k = hex::encode(id);
		let mut n = on_chain.nonce_floor.max(map.get(&k).copied().unwrap_or(0));
		while !on_chain.nonce_usable(n) {
			n += 1;
		}
		map.insert(k, n + 1);
		io(fs::write(p, map.iter().map(|(k, v)| format!("{k} {v}\n")).collect::<String>()))?;
		Ok(n)
	}

	// ---- queue ----

	fn frame(item: &Item) -> Vec<u8> {
		encode_payload(std::slice::from_ref(item))[1..].to_vec()
	}

	pub fn enqueue(&self, item: &Item) -> Result<usize> {
		let p = self.dir.join("queue");
		let mut lines = read_lines(&p);
		lines.push(hex::encode(Self::frame(item)));
		io(fs::write(p, lines.join("\n") + "\n"))?;
		Ok(lines.len())
	}

	/// Take everything queued and return it as one payload.
	pub fn drain_queue(&self) -> Result<(Vec<u8>, usize)> {
		let p = self.dir.join("queue");
		let lines = read_lines(&p);
		let mut payload = vec![PAYLOAD_VERSION];
		for l in &lines {
			payload.extend(hex::decode(l).map_err(|e| e.to_string())?);
		}
		let _ = fs::remove_file(p);
		Ok((payload, lines.len()))
	}

	// ---- a bundler's order book ----

	fn book_path(&self, base: AssetId, quote: AssetId) -> PathBuf {
		self.dir.join(format!("orders-{base}-{quote}"))
	}

	pub fn add_order(&self, base: AssetId, quote: AssetId, o: &SignedOrder) -> Result<()> {
		let p = self.book_path(base, quote);
		let mut lines = read_lines(&p);
		lines.push(hex::encode(o.to_bytes()));
		io(fs::write(p, lines.join("\n") + "\n"))
	}

	pub fn orders(&self, base: AssetId, quote: AssetId) -> Result<Vec<SignedOrder>> {
		read_lines(&self.book_path(base, quote))
			.iter()
			.map(|l| {
				let b = hex::decode(l).map_err(|e| e.to_string())?;
				SignedOrder::from_bytes(&b).map_err(|_| "corrupt order book".to_string())
			})
			.collect()
	}

	pub fn clear_orders(&self, base: AssetId, quote: AssetId) {
		let _ = fs::remove_file(self.book_path(base, quote));
	}

	// ---- a local Bitcoin regtest chain ----

	fn chain_path(&self, name: &str) -> PathBuf {
		self.dir.join(format!("btc-{name}"))
	}

	pub fn load_chain(&self, name: &str) -> Result<Regtest> {
		let p = self.chain_path(name);
		if !p.exists() {
			return Err(format!("no local chain '{name}': run `jamx btc init {name}` first"));
		}
		let mut blocks = vec![];
		for l in read_lines(&p) {
			let (h, txs) = l.split_once(' ').ok_or("corrupt chain file")?;
			let raw: [u8; 80] =
				hex::decode(h).map_err(|e| e.to_string())?.try_into().map_err(|_| "bad header")?;
			let txs = txs
				.split(',')
				.filter(|t| !t.is_empty())
				.map(|t| hex::decode(t).map_err(|e| e.to_string()))
				.collect::<Result<Vec<_>>>()?;
			blocks.push(Block { header: btc::Header::parse(&raw), txs });
		}
		Ok(Regtest { blocks })
	}

	pub fn save_chain(&self, name: &str, c: &Regtest) -> Result<()> {
		let text: String = c
			.blocks
			.iter()
			.map(|b| {
				format!(
					"{} {}\n",
					hex::encode(b.header.serialize()),
					b.txs.iter().map(hex::encode).collect::<Vec<_>>().join(",")
				)
			})
			.collect();
		io(fs::write(self.chain_path(name), text))
	}
}

pub struct Block {
	pub header: btc::Header,
	pub txs: Vec<Vec<u8>>,
}

impl Block {
	pub fn hash(&self) -> Hash32 {
		self.header.hash()
	}
	pub fn txids(&self) -> Vec<Hash32> {
		self.txs.iter().map(|t| sha256d(t)).collect()
	}
}

/// A Bitcoin chain at regtest difficulty, mined on this machine.
pub struct Regtest {
	pub blocks: Vec<Block>,
}

impl Regtest {
	/// A new chain whose first block is dated `time`. Every later block is dated ten
	/// minutes after the one before it, however quickly it is mined here.
	pub fn genesis(seed: u32, time: u32) -> Self {
		let mut c = Self { blocks: vec![] };
		c.mine_on([0; 32], seed, time, vec![]);
		c
	}

	pub fn tip(&self) -> Hash32 {
		self.blocks.last().map(|b| b.hash()).unwrap_or([0; 32])
	}

	fn mine_on(&mut self, prev: Hash32, salt: u32, time: u32, txs: Vec<Vec<u8>>) {
		let height = self.blocks.len() as u32;
		let mut all = vec![btc::build_coinbase(salt ^ height.wrapping_mul(0x9e37_79b9), &[(50_0000_0000, vec![0x51])])];
		all.extend(txs);
		let txids: Vec<Hash32> = all.iter().map(|t| sha256d(t)).collect();
		let (root, _) = btc::merkle_branch(&txids, 0);
		let header = btc::mine(prev, root, time, btc::REGTEST.pow_limit_bits);
		self.blocks.push(Block { header, txs: all });
	}

	pub fn mine(&mut self, txs: Vec<Vec<u8>>) {
		let salt = u32::from_le_bytes(self.tip()[..4].try_into().unwrap_or([0; 4]));
		let time = self.blocks.last().map_or(0, |b| b.header.time) + 600;
		self.mine_on(self.tip(), salt, time, txs);
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

	/// Find a transaction output paying at least `sats` to `script`, above block
	/// `min_height`, that can settle the deal whose tag is `tag`: it carries that tag, or, if
	/// the deal does not require one, no tag at all. Returns (block, tx index, output index).
	pub fn find_payment(
		&self,
		script: &[u8],
		sats: u64,
		tag: &Hash32,
		tag_required: bool,
		min_height: usize,
	) -> Option<(usize, usize, u32)> {
		let ours = btc::tag_script(tag);
		for (h, b) in self.blocks.iter().enumerate().skip(min_height + 1) {
			for (ti, tx) in b.txs.iter().enumerate() {
				let Ok(outs) = btc::parse_outputs(tx) else { continue };
				let tagged = outs.iter().any(|o| o.script == &ours[..]);
				let other_tag = !tagged && outs.iter().any(|o| btc::is_tag_script(o.script));
				if other_tag || (tag_required && !tagged) {
					continue;
				}
				if let Some(vo) = outs.iter().position(|o| o.script == script && o.value >= sats) {
					return Some((h, ti, vo as u32));
				}
			}
		}
		None
	}

	pub fn proof(&self, deal: DealId, block: usize, tx_index: usize, vout: u32) -> BtcProof {
		let b = &self.blocks[block];
		let (_, branch) = btc::merkle_branch(&b.txids(), tx_index);
		let (_, coinbase_branch) = btc::merkle_branch(&b.txids(), 0);
		BtcProof {
			deal,
			block_hash: b.hash(),
			tx: b.txs[tx_index].clone(),
			vout,
			index: tx_index as u32,
			branch,
			coinbase: b.txs[0].clone(),
			coinbase_branch,
		}
	}
}

/// A made-up P2WPKH output script for a named wallet (demonstrations only).
pub fn demo_script(name: &str) -> Vec<u8> {
	let mut s = vec![0x00, 0x14];
	s.extend_from_slice(&blake2b(&[b"jamx/demo-wallet/", name.as_bytes()])[..20]);
	s
}

/// Parse a price given as `num/den` or as a decimal such as `0.6`.
pub fn parse_price(s: &str) -> Result<Price> {
	let p = if let Some((n, d)) = s.split_once('/') {
		Price::new(n.trim().parse().map_err(|_| "bad price")?, d.trim().parse().map_err(|_| "bad price")?)
	} else if let Some((int, frac)) = s.split_once('.') {
		let den = 10u64.checked_pow(frac.len() as u32).ok_or("too many decimals")?;
		let num: u64 = format!("{int}{frac}").parse().map_err(|_| "bad price")?;
		Price::new(num, den)
	} else {
		Price::new(s.parse().map_err(|_| "bad price")?, 1)
	};
	if p.valid() {
		Ok(p)
	} else {
		Err("a price must be positive".into())
	}
}

pub fn symbol(s: &str) -> Result<[u8; 8]> {
	if s.is_empty() || s.len() > 8 {
		return Err("symbols are 1 to 8 characters".into());
	}
	let mut out = [0u8; 8];
	out[..s.len()].copy_from_slice(s.as_bytes());
	Ok(out)
}

pub fn decode_item(frame_hex: &str) -> Option<Item> {
	let b = hex::decode(frame_hex).ok()?;
	let mut r = Reader::new(&b);
	let tag = r.u8().ok()?;
	let body = r.var_bytes().ok()?;
	Item::dec_body(tag, &mut Reader::new(body)).ok()
}
