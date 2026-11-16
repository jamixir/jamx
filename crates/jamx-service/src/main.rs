//! JAMX as a JAM service: the two entry points and the host bindings.
//!
//! All logic lives in `jamx-core`. This file only connects it to the JAM host calls.
#![no_std]
#![no_main]
extern crate alloc;

use alloc::vec::Vec;
use jam_pvm_common::{accumulate as host, declare_service, Service};
use jam_types::*;
use jamx_core::{
	accumulate::{Genesis, Session},
	state::Host,
};

/// Governance is fixed when the code is built, so the code hash commits to it:
/// `JAMX_ADMIN` is the hex account allowed to announce upgrades (absent: the code can never
/// change) and `JAMX_UPGRADE_DELAY` the number of slots an announcement must stand.
const ADMIN_HEX: Option<&str> = option_env!("JAMX_ADMIN");
const UPGRADE_DELAY: Option<&str> = option_env!("JAMX_UPGRADE_DELAY");

fn genesis() -> Genesis {
	let admin = ADMIN_HEX.and_then(|h| {
		let h = h.as_bytes();
		if h.len() != 64 {
			return None;
		}
		let nib = |c: u8| match c {
			b'0'..=b'9' => Some(c - b'0'),
			b'a'..=b'f' => Some(c - b'a' + 10),
			b'A'..=b'F' => Some(c - b'A' + 10),
			_ => None,
		};
		let mut out = [0u8; 32];
		for (i, o) in out.iter_mut().enumerate() {
			*o = nib(h[2 * i])? << 4 | nib(h[2 * i + 1])?;
		}
		Some(out)
	});
	let upgrade_delay = UPGRADE_DELAY.and_then(|s| s.parse().ok()).unwrap_or(0);
	Genesis { admin, upgrade_delay }
}

#[polkavm_derive::polkavm_import]
extern "C" {
	/// The `read` host call (index 4 in Gray Paper 0.8.0): copy up to `len` bytes of the value
	/// under `key`, from `offset`, to `out`; return the value's full length, or `u64::MAX` if
	/// there is none. Declared here to read in one call: the SDK's wrapper asks for the
	/// length first, and a host call costs about as much as a hundred memory accesses.
	#[polkavm_import(index = 4)]
	fn jamx_read(service: u64, key: *const u8, key_len: u64, out: *mut u8, offset: u64, len: u64) -> u64;
}

struct Pvm;

impl Host for Pvm {
	fn read(&mut self, key: &[u8], buf: &mut Vec<u8>) -> bool {
		buf.clear();
		buf.reserve(512);
		let mut cap = buf.capacity();
		// SAFETY: the host writes at most `cap` bytes into the buffer's spare capacity and
		// reports the value's length; only bytes it has written become part of the vector.
		let mut n = unsafe { jamx_read(u64::MAX, key.as_ptr(), key.len() as u64, buf.as_mut_ptr(), 0, cap as u64) };
		if n == u64::MAX {
			return false;
		}
		if n as usize > cap {
			buf.reserve(n as usize);
			cap = buf.capacity();
			n = unsafe { jamx_read(u64::MAX, key.as_ptr(), key.len() as u64, buf.as_mut_ptr(), 0, cap as u64) };
			if n == u64::MAX || n as usize > cap {
				return false;
			}
		}
		unsafe { buf.set_len(n as usize) };
		true
	}
	fn write(&mut self, key: &[u8], value: &[u8]) -> bool {
		host::set_storage(key, value).is_ok()
	}
	fn remove(&mut self, key: &[u8]) {
		host::remove_storage(key);
	}
	fn gas(&mut self) -> u64 {
		host::gas()
	}
	fn checkpoint(&mut self) {
		host::checkpoint();
	}
	fn upgrade(&mut self, code_hash: &[u8; 32], min_item_gas: u64, min_memo_gas: u64) {
		host::upgrade(&(*code_hash).into(), min_item_gas, min_memo_gas);
	}
	fn solicit(&mut self, hash: &[u8; 32], len: u32) -> bool {
		host::solicit(hash, len as usize).is_ok()
	}
	fn preimage_available(&mut self, hash: &[u8; 32]) -> bool {
		host::is_available(hash)
	}
	#[allow(unused_variables)]
	fn log(&mut self, msg: &str) {
		#[cfg(feature = "logging")]
		jam_pvm_common::info!("{}", msg);
	}
}

struct Jamx;
declare_service!(Jamx);

impl Service for Jamx {
	fn refine(
		_core_index: CoreIndex,
		_item_index: usize,
		service_id: ServiceId,
		payload: WorkPayload,
		_package_hash: WorkPackageHash,
	) -> WorkOutput {
		let payload = payload.take();
		#[cfg(feature = "logging")]
		let gas_before = jam_pvm_common::refine::gas();
		let out = jamx_core::refine::refine(service_id, &payload);
		#[cfg(feature = "logging")]
		jam_pvm_common::info!(
			"jamx refine: {} bytes in, {} bytes out, {} gas",
			payload.len(),
			out.len(),
			gas_before - jam_pvm_common::refine::gas()
		);
		out.into()
	}

	fn accumulate(slot: Slot, service_id: ServiceId, item_count: usize) -> Option<Hash> {
		let mut pvm = Pvm;
		#[cfg(feature = "logging")]
		let gas_before = host::gas();
		let mut session = Session::new(&mut pvm, service_id, slot, &genesis());
		#[cfg(feature = "profile")]
		jam_pvm_common::info!("profile: session start used {} gas", gas_before - host::gas());
		for i in 0..item_count {
			match host::accumulate_item(i) {
				Some(AccumulateItem::WorkItem(w)) => {
					let payload_hash: [u8; 32] = w.payload.into();
					// Each item may spend the accumulate gas it declared, and no more: the
					// invocation's gas is pooled, and one item must not eat into another's.
					match w.result {
						Ok(output) => {
							session.work(&payload_hash, &output, w.gas_limit);
						},
						Err(_) => session.failed_work(&payload_hash, w.gas_limit),
					}
				},
				Some(AccumulateItem::Transfer(t)) => session.transfer(t.source, t.amount, &t.memo.0),
				None => {},
			}
		}
		session.finish();
		#[cfg(feature = "logging")]
		jam_pvm_common::info!("jamx accumulate: {} items, {} gas", item_count, gas_before - host::gas());
		None
	}
}
