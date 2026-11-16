#![no_std]
#![no_main]
extern crate alloc;

use jam_pvm_common::{
	accumulate::{accumulate_items, set_storage},
	declare_service, info, Service,
};
use jam_types::*;

struct Hello;
declare_service!(Hello);

impl Service for Hello {
	fn refine(
		_core_index: CoreIndex,
		_item_index: usize,
		_service_id: ServiceId,
		payload: WorkPayload,
		_package_hash: WorkPackageHash,
	) -> WorkOutput {
		let p = payload.take();
		info!("hello refine: {} bytes, gas left {}", p.len(), jam_pvm_common::refine::gas());
		[&b"Hello "[..], p.as_slice()].concat().into()
	}

	fn accumulate(slot: Slot, _service_id: ServiceId, item_count: usize) -> Option<Hash> {
		info!("hello accumulate: slot {} items {}", slot, item_count);
		for item in accumulate_items().into_iter() {
			match item {
				AccumulateItem::WorkItem(w) => match w.result {
					Ok(data) => {
						info!("hello: refine ok, {} bytes", data.len());
						let r = set_storage(b"last", &data);
						info!("hello: set_storage -> {:?}", r);
					},
					Err(e) => info!("hello: refine error {:?}", e),
				},
				AccumulateItem::Transfer(t) => {
					let m = t.memo.to_vec();
					info!("hello: transfer from {} amount {} memo[..40]={:02x?}", t.source, t.amount, &m[..40]);
				},
			}
		}
		None
	}
}
