//! Talking to a JAM node: reading service storage over the JIP-2 RPC and submitting work
//! items through `jamt`.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use jamx_core::{
	accumulate::{Receipt, RECEIPT_RING},
	codec::Codec,
	crypto::blake2b,
	state::key,
	types::Config,
};
use serde_json::{json, Value};
use std::{
	path::PathBuf,
	process::Command,
	thread::sleep,
	time::{Duration, Instant},
};

pub type Result<T> = std::result::Result<T, String>;

pub struct Chain {
	pub rpc: String,
	pub service: u32,
	pub jamt: PathBuf,
	pub dir: PathBuf,
	pub core: Option<u16>,
	pub refine_gas: Option<u64>,
	pub accumulate_gas: Option<u64>,
}

impl Chain {
	fn call(&self, method: &str, params: Value) -> Result<Value> {
		let (mut ws, _) =
			tungstenite::connect(&self.rpc).map_err(|e| format!("cannot reach {}: {e}", self.rpc))?;
		let req = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
		ws.send(tungstenite::Message::Text(req.to_string())).map_err(|e| e.to_string())?;
		loop {
			match ws.read().map_err(|e| e.to_string())? {
				tungstenite::Message::Text(t) => {
					let v: Value = serde_json::from_str(&t).map_err(|e| e.to_string())?;
					let _ = ws.close(None);
					if let Some(err) = v.get("error") {
						return Err(format!("rpc {method}: {err}"));
					}
					return Ok(v.get("result").cloned().unwrap_or(Value::Null));
				},
				tungstenite::Message::Close(_) => return Err("rpc closed".into()),
				_ => {},
			}
		}
	}

	/// Hash (base64) and slot of the best block.
	pub fn best(&self) -> Result<(String, u32)> {
		let v = self.call("bestBlock", json!([]))?;
		Ok((
			v["header_hash"].as_str().ok_or("no header_hash")?.to_string(),
			v["slot"].as_u64().ok_or("no slot")? as u32,
		))
	}

	pub fn slot(&self) -> Result<u32> {
		Ok(self.best()?.1)
	}

	/// Read one storage item of the service at the best block.
	pub fn read(&self, k: &[u8]) -> Result<Option<Vec<u8>>> {
		let (head, _) = self.best()?;
		match self.call("serviceValue", json!([head, self.service, B64.encode(k)]))? {
			Value::Null => Ok(None),
			Value::String(s) => Ok(Some(B64.decode(s).map_err(|e| e.to_string())?)),
			other => Err(format!("unexpected serviceValue result: {other}")),
		}
	}

	pub fn get<T: Codec>(&self, k: &[u8]) -> Result<Option<T>> {
		match self.read(k)? {
			None => Ok(None),
			Some(b) => T::from_bytes(&b).map(Some).map_err(|_| "stored record does not decode".into()),
		}
	}

	pub fn config(&self) -> Result<Config> {
		Ok(self.get(key::CONFIG)?.unwrap_or_default())
	}

	fn find_receipt(&self, payload_hash: &[u8; 32], not_before: u32) -> Result<Option<Receipt>> {
		let seq = self.config()?.receipt_seq;
		for back in 1..=8u32.min(seq) {
			let idx = ((seq - back) % RECEIPT_RING) as u8;
			if let Some(r) = self.get::<Receipt>(&key::receipt(idx))? {
				if &r.payload_hash == payload_hash && r.slot >= not_before {
					return Ok(Some(r));
				}
			}
		}
		Ok(None)
	}

	/// Hand a payload to JAM as a work item and wait until the service has accumulated it.
	pub fn submit(&self, payload: &[u8]) -> Result<Receipt> {
		let hash = blake2b(&[payload]);
		std::fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;
		let path = self.dir.join(format!("payload-{}.bin", hex::encode(&hash[..6])));
		std::fs::write(&path, payload).map_err(|e| e.to_string())?;
		let start_slot = self.slot()?;

		let mut cmd = Command::new(&self.jamt);
		cmd.arg("--rpc").arg(&self.rpc);
		if let Some(c) = self.core {
			cmd.arg("--force-core").arg(c.to_string());
		}
		cmd.arg("item");
		if let Some(g) = self.refine_gas {
			cmd.arg("-G").arg(g.to_string());
		}
		if let Some(g) = self.accumulate_gas {
			cmd.arg("-g").arg(g.to_string());
		}
		cmd.arg(format!("{:08x}", self.service)).arg(&path);
		let out = cmd.output().map_err(|e| format!("cannot run {}: {e}", self.jamt.display()))?;
		if !out.status.success() {
			return Err(format!(
				"jamt failed: {}{}",
				String::from_utf8_lossy(&out.stdout),
				String::from_utf8_lossy(&out.stderr)
			));
		}
		let _ = std::fs::remove_file(&path);

		let deadline = Instant::now() + Duration::from_secs(150);
		loop {
			if let Some(r) = self.find_receipt(&hash, start_slot)? {
				return Ok(r);
			}
			if Instant::now() > deadline {
				return Err("timed out waiting for the work item to be accumulated".into());
			}
			sleep(Duration::from_millis(1500));
		}
	}

	/// Block until the chain has advanced by `slots`.
	pub fn wait_slots(&self, slots: u32) -> Result<u32> {
		let target = self.slot()? + slots;
		loop {
			let now = self.slot()?;
			if now >= target {
				return Ok(now);
			}
			sleep(Duration::from_secs(2));
		}
	}
}
