//! Tokens, issuers, account rules and governance.

mod common;
use common::*;
use jamx_core::{accumulate::Genesis, codec::Codec, crypto::blake2b, ops::*, state::key, types::*};

const OK: u8 = Status::Ok as u8;

fn st(s: Status) -> u8 {
	s as u8
}

#[test]
fn issue_transfer_burn() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	assert_eq!(brl, 1);
	w.mint(&bank, brl, &alice, 1_000);
	assert_eq!(w.bal(&alice, brl), 1_000);
	assert_eq!(w.asset(brl).supply, 1_000);

	// An account holds only what it has accepted: nobody can push an asset into it.
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 400 }), st(Status::NotAllowed));
	w.must(&bob, Op::Accept { asset: brl, on: true });
	w.must(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 400 });
	assert_eq!((w.bal(&alice, brl), w.bal(&bob, brl)), (600, 400));

	// Overspending, zero amounts, self-transfers and unknown assets are refused.
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 601 }), st(Status::Insufficient));
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 0 }), st(Status::Invalid));
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: alice.id, amount: 1 }), st(Status::Invalid));
	assert_eq!(w.op(&alice, Op::Transfer { asset: 99, to: bob.id, amount: 1 }), st(Status::NotFound));
	assert_eq!((w.bal(&alice, brl), w.bal(&bob, brl)), (600, 400));

	// Only the issuer mints.
	assert_eq!(w.op(&alice, Op::Mint { asset: brl, to: alice.id, amount: 1 }), st(Status::Unauthorized));

	// Redemption: the holder sends tokens back and the issuer destroys them.
	w.must(&bob, Op::Transfer { asset: brl, to: bank.id, amount: 400 });
	w.must(&bank, Op::Burn { asset: brl, amount: 400 });
	assert_eq!(w.asset(brl).supply, 600);
	// Bob still accepts the asset, with nothing in it. A balance that holds something
	// cannot be dropped; an empty one can.
	assert_eq!(w.op(&alice, Op::Accept { asset: brl, on: false }), st(Status::Invalid));
	assert!(w.account(&bob.id).accepts(brl));
	w.must(&bob, Op::Accept { asset: brl, on: false });
	assert!(!w.account(&bob.id).accepts(brl));
	assert_eq!(w.op(&bob, Op::Accept { asset: brl, on: false }), st(Status::NotFound));
	assert_eq!(w.op(&bob, Op::Accept { asset: 99, on: true }), st(Status::NotFound));
}

#[test]
fn nobody_can_fill_an_account_with_assets_it_did_not_ask_for() {
	let mut w = World::new();
	let (bank, alice, mallory) = (actor(1), actor(2), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	let before = w.account(&alice.id);
	// Mallory makes throw-away tokens and tries to plant them, or a freeze mark, in Alice's
	// account. Neither a mint, nor a transfer, nor an issuer's flag creates an entry there.
	for _ in 0..3 {
		let junk = w.token(&mallory, "JUNK", ASSET_FREEZABLE);
		assert_eq!(w.op(&mallory, Op::Mint { asset: junk, to: alice.id, amount: 1 }), st(Status::NotAllowed));
		w.must(&mallory, Op::Mint { asset: junk, to: mallory.id, amount: 1 });
		assert_eq!(w.op(&mallory, Op::Transfer { asset: junk, to: alice.id, amount: 1 }), st(Status::NotAllowed));
		assert_eq!(
			w.op(&mallory, Op::IssuerSet { asset: junk, target: alice.id, action: IssuerAction::Freeze, reason: [0; 32] }),
			st(Status::NotFound)
		);
	}
	assert_eq!(w.account(&alice.id), before);
	// An account can accept a bounded number of assets, by its own choice only.
	for i in 0..MAX_BALANCES - 1 {
		let t = w.token(&actor(100 + i as u8), "T", 0);
		w.must(&alice, Op::Accept { asset: t, on: true });
	}
	let t = w.token(&actor(200), "T", 0);
	assert_eq!(w.op(&alice, Op::Accept { asset: t, on: true }), st(Status::Invalid));
	assert_eq!(w.account(&alice.id).balances.len(), MAX_BALANCES);
}

#[test]
fn a_signature_works_once() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	w.accept(&bob, brl);
	let item = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 100 });
	assert_eq!(w.submit(vec![item.clone()]), vec![OK]);
	// Replaying the very same bytes does nothing, in a later package or in the same one.
	assert_eq!(w.submit(vec![item.clone()]), vec![st(Status::Nonce)]);
	assert_eq!(w.submit(vec![item.clone(), item]), vec![st(Status::Nonce), st(Status::Nonce)]);
	assert_eq!(w.bal(&bob, brl), 100);
}

#[test]
fn a_failed_operation_still_spends_its_nonce() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.accept(&alice, brl);
	w.accept(&bob, brl);
	let item = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 100 });
	assert_eq!(w.submit(vec![item.clone()]), vec![st(Status::Insufficient)]);
	// The signature and the nonce were good, so the nonce is gone. Otherwise the same bytes
	// could be presented again by anyone, at a moment of their choosing, when they succeed.
	w.mint(&bank, brl, &alice, 100);
	assert_eq!(w.submit(vec![item]), vec![st(Status::Nonce)]);
	assert_eq!(w.bal(&bob, brl), 0);
	// What the failed operation did otherwise is undone: only the nonce moved.
	assert_eq!(w.bal(&alice, brl), 100);
}

#[test]
fn a_signature_does_not_outlive_its_expiry() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.accept(&bob, brl);
	let signed = |w: &mut World, expiry: Slot| {
		let nonce = w.nonce(&alice.id);
		let env = Envelope { account: alice.id, nonce, expiry, op: Op::Transfer { asset: brl, to: bob.id, amount: 10 } };
		Item::Signed(SignedOp::sign(&alice.seed, SVC, &env))
	};
	// Signed, handed to someone, and never submitted. Later it is worth nothing.
	let until = w.slot + 10;
	let stale = signed(&mut w, until);
	let fresh = signed(&mut w, until);
	assert_eq!(w.submit(vec![fresh]), vec![OK]);
	w.slot += 11;
	assert_eq!(w.submit(vec![stale.clone()]), vec![st(Status::Expired)]);
	assert_eq!(w.bal(&bob, brl), 10);
	// Being refused for lateness does not spend the nonce (the account did nothing).
	let now = w.slot;
	let again = signed(&mut w, now);
	assert_eq!(w.submit(vec![again]), vec![OK]);
}

#[test]
fn one_bad_record_does_not_spoil_its_neighbours() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.accept(&bob, brl);
	let a = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 60 });
	let b = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 60 });
	let c = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 40 });
	assert_eq!(w.submit(vec![a, b, c]), vec![OK, st(Status::Insufficient), OK]);
	assert_eq!((w.bal(&alice, brl), w.bal(&bob, brl)), (0, 100));
	let r = w.last_receipt();
	assert_eq!(r.statuses, vec![OK, st(Status::Insufficient), OK]);
	assert_eq!(r.slot, w.slot);
}

#[test]
fn mint_cap_follows_attested_reserves() {
	let mut w = World::new();
	let (bank, aud1, aud2, aud3, alice) = (actor(1), actor(11), actor(12), actor(13), actor(2));
	let brl = w.next_asset();
	w.must(
		&bank,
		Op::AssetCreate {
			symbol: sym("BRL"),
			decimals: 2,
			flags: 0,
			kind: AssetKindCfg::Token,
			attestors: vec![aud1.id, aud2.id, aud3.id],
			threshold: 2,
		},
	);
	w.accept(&alice, brl);
	// No attestations yet: nothing can be issued.
	assert_eq!(w.op(&bank, Op::Mint { asset: brl, to: alice.id, amount: 1 }), st(Status::Cap));
	// One auditor is not enough.
	w.must(&aud1, Op::Attest { asset: brl, reserves: 1_000 });
	assert_eq!(w.op(&bank, Op::Mint { asset: brl, to: alice.id, amount: 1 }), st(Status::Cap));
	// Two auditors: the cap is what both vouch for.
	w.must(&aud2, Op::Attest { asset: brl, reserves: 800 });
	w.must(&bank, Op::Mint { asset: brl, to: alice.id, amount: 800 });
	assert_eq!(w.op(&bank, Op::Mint { asset: brl, to: alice.id, amount: 1 }), st(Status::Cap));
	// The bank cannot attest for itself, and neither can a stranger.
	assert_eq!(w.op(&bank, Op::Attest { asset: brl, reserves: u64::MAX }), st(Status::Unauthorized));
	assert_eq!(w.op(&alice, Op::Attest { asset: brl, reserves: u64::MAX }), st(Status::Unauthorized));
	// A third auditor raises the second-highest figure.
	w.must(&aud3, Op::Attest { asset: brl, reserves: 900 });
	w.must(&bank, Op::Mint { asset: brl, to: alice.id, amount: 100 });
	assert_eq!(w.asset(brl).supply, 900);
	// An issuer cannot name itself among the auditors of its own reserves.
	let own_auditor = Op::AssetCreate {
		symbol: sym("BRL"),
		decimals: 2,
		flags: 0,
		kind: AssetKindCfg::Token,
		attestors: vec![aud1.id, bank.id],
		threshold: 1,
	};
	assert_eq!(w.op(&bank, own_auditor), st(Status::Invalid));
}

#[test]
fn permissioned_token_needs_the_issuers_approval() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", ASSET_PERMISSIONED);
	let allow = |target: &Actor, action| Op::IssuerSet { asset: brl, target: target.id, action, reason: [0; 32] };
	assert_eq!(w.op(&bank, Op::Mint { asset: brl, to: alice.id, amount: 100 }), st(Status::NotAllowed));
	// The customer opens the relationship (accepts the asset); the bank then approves it.
	assert_eq!(w.op(&bank, allow(&alice, IssuerAction::Allow)), st(Status::NotFound));
	w.must(&alice, Op::Accept { asset: brl, on: true });
	assert_eq!(w.op(&bank, Op::Mint { asset: brl, to: alice.id, amount: 100 }), st(Status::NotAllowed));
	w.must(&bank, allow(&alice, IssuerAction::Allow));
	w.mint(&bank, brl, &alice, 100);
	// Bob has not passed the bank's checks: he cannot receive.
	w.must(&bob, Op::Accept { asset: brl, on: true });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 10 }), st(Status::NotAllowed));
	assert_eq!(w.bal(&alice, brl), 100);
	w.must(&bank, allow(&bob, IssuerAction::Allow));
	w.must(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 10 });
	// Losing approval stops new receipts but not sending back to the issuer.
	w.must(&bank, allow(&bob, IssuerAction::Disallow));
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 10 }), st(Status::NotAllowed));
	w.must(&bob, Op::Transfer { asset: brl, to: bank.id, amount: 10 });
	// Only the issuer manages the list, and only for assets that have one.
	assert_eq!(w.op(&alice, allow(&bob, IssuerAction::Allow)), st(Status::Unauthorized));
	assert_eq!(w.op(&bank, allow(&bob, IssuerAction::Freeze)), st(Status::Invalid));
}

#[test]
fn issuer_freeze_is_logged_and_cannot_move_funds() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", ASSET_FREEZABLE);
	let other = w.token(&alice, "OTH", 0);
	w.mint(&bank, brl, &alice, 100);
	w.mint(&alice, other, &alice, 50);
	w.accept(&bob, brl);
	w.accept(&bob, other);
	let reason = blake2b(&[b"court order 123"]);
	w.must(&bank, Op::IssuerSet { asset: brl, target: alice.id, action: IssuerAction::Freeze, reason });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 1 }), st(Status::Frozen));
	assert_eq!(w.op(&alice, Op::Burn { asset: brl, amount: 1 }), st(Status::Frozen));
	// The freeze reaches the bank's own token and nothing else.
	w.must(&alice, Op::Transfer { asset: other, to: bob.id, amount: 5 });
	// The frozen balance is still Alice's: the bank has no operation that takes it.
	assert_eq!(w.bal(&alice, brl), 100);
	// Every use of the power is on the public record.
	assert_eq!(w.asset(brl).freeze_log, 1);
	let entry = &w.host.store[&key::freeze_log(brl, 0)];
	assert_eq!(&entry[..32], &alice.id);
	assert_eq!(entry[32], IssuerAction::Freeze as u8);
	assert_eq!(&entry[33..65], &reason);
	w.must(&bank, Op::IssuerSet { asset: brl, target: alice.id, action: IssuerAction::Unfreeze, reason });
	assert_eq!(w.asset(brl).freeze_log, 2);
	w.must(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 1 });
	// The power reaches accounts that accept the asset, and a freeze cannot be shed by
	// leaving the asset and coming back.
	let carol = actor(4);
	let freeze_carol = Op::IssuerSet { asset: brl, target: carol.id, action: IssuerAction::Freeze, reason };
	assert_eq!(w.op(&bank, freeze_carol.clone()), st(Status::NotFound));
	w.accept(&carol, brl);
	w.must(&bank, freeze_carol);
	assert_eq!(w.op(&carol, Op::Accept { asset: brl, on: false }), st(Status::Frozen));
	// A token created without the power can never be frozen.
	assert_eq!(
		w.op(&alice, Op::IssuerSet { asset: other, target: bob.id, action: IssuerAction::Freeze, reason }),
		st(Status::Invalid)
	);
}

#[test]
fn second_device_can_trade_but_not_withdraw() {
	let mut w = World::new();
	let (bank, alice, phone, bob) = (actor(1), actor(2), actor(22), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.must(&alice, Op::Policy(PolicyChange::AddKey { key: phone.id, role: ROLE_TRADER }));
	// The phone key acts for Alice's account, within its role.
	assert_eq!(
		w.op_as(&phone, &alice.id, Op::Transfer { asset: brl, to: bob.id, amount: 1 }),
		st(Status::Unauthorized)
	);
	assert_eq!(w.op_as(&phone, &alice.id, Op::Cancel { nonces: vec![50] }), OK);
	// It cannot freeze the account: a frozen owner cannot veto a recovery, so that power
	// is reserved to the owner and the guardian.
	assert_eq!(w.op_as(&phone, &alice.id, Op::Freeze { target: alice.id }), st(Status::Unauthorized));
	assert_eq!(
		w.op_as(&phone, &alice.id, Op::Policy(PolicyChange::AddKey { key: bob.id, role: ROLE_OWNER })),
		st(Status::Unauthorized)
	);
	// A key that was never added is nobody.
	assert_eq!(w.op_as(&bob, &alice.id, Op::Cancel { nonces: vec![] }), st(Status::Unauthorized));
	// Removing the phone ends its access.
	w.must(&alice, Op::Policy(PolicyChange::RemoveKey { key: phone.id }));
	assert_eq!(w.op_as(&phone, &alice.id, Op::Cancel { nonces: vec![] }), st(Status::Unauthorized));
}

#[test]
fn guardian_can_freeze_but_not_spend() {
	let mut w = World::new();
	let (bank, alice, friend, thief) = (actor(1), actor(2), actor(4), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.must(&alice, Op::Policy(PolicyChange::SetFreezeSlots(1_000)));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));
	// A first protection brings a security delay with it (here capped by the freeze
	// length): without one, a thief holding the key would simply remove the guardian.
	assert_eq!(w.account(&alice.id).policy.unwrap().delay, 999);

	// The friend has no way to move Alice's money.
	assert_eq!(
		w.op_as(&friend, &alice.id, Op::Transfer { asset: brl, to: friend.id, amount: 1 }),
		st(Status::Unauthorized)
	);
	// A stranger cannot freeze her.
	assert_eq!(w.op(&thief, Op::Freeze { target: alice.id }), st(Status::Unauthorized));

	// The friend freezes: nothing leaves, not even with Alice's own key.
	w.must(&friend, Op::Freeze { target: alice.id });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: thief.id, amount: 100 }), st(Status::Frozen));
	assert_eq!(w.op(&alice, Op::Policy(PolicyChange::SetGuardian(None))), st(Status::Frozen));
	assert_eq!(w.op(&alice, Op::Burn { asset: brl, amount: 1 }), st(Status::Frozen));
	// Money can still arrive.
	w.mint(&bank, brl, &alice, 5);
	// The owner key cannot lift the freeze early; the friend can.
	assert_eq!(w.op(&alice, Op::Unfreeze { target: alice.id }), st(Status::Frozen));
	assert_eq!(w.op(&friend, Op::Freeze { target: alice.id }), st(Status::Exists));
	w.must(&friend, Op::Unfreeze { target: alice.id });
	w.must(&alice, Op::Transfer { asset: brl, to: bank.id, amount: 1 });

	// A freeze cannot be renewed back-to-back: after it ends, an equal period must pass.
	assert_eq!(w.op(&friend, Op::Freeze { target: alice.id }), st(Status::TooEarly));
	w.slot += 1_000;
	w.must(&friend, Op::Freeze { target: alice.id });
	// And it lapses by itself.
	w.slot += 1_000;
	w.must(&alice, Op::Transfer { asset: brl, to: bank.id, amount: 1 });
}

#[test]
fn weakening_changes_wait_out_the_delay() {
	let mut w = World::new();
	let (bank, alice, friend, thief) = (actor(1), actor(2), actor(4), actor(66));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 1_000);
	// Alice turns protections on. None of this weakens the account, so it is immediate.
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(500)));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(100) }));

	// The daily limit binds.
	w.must(&alice, Op::Transfer { asset: brl, to: bank.id, amount: 100 });
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bank.id, amount: 1 }), st(Status::Limit));

	// Now suppose the key is stolen. The thief tries to open the account up. Every such
	// change is queued, not applied.
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: None }));
	w.must(&alice, Op::Policy(PolicyChange::AddKey { key: thief.id, role: ROLE_OWNER }));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(None)));
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(0)));
	let p = w.account(&alice.id).policy.unwrap();
	assert_eq!(p.pending.len(), 4);
	assert_eq!(p.limits.len(), 1);
	assert_eq!(p.keys.len(), 1);
	assert_eq!(w.op(&alice, Op::Policy(PolicyChange::SetFreezeSlots(1))), st(Status::Invalid)); // queue full
	assert_eq!(w.plain(PlainOp::PolicyExec { account: alice.id, index: 0 }), st(Status::TooEarly));

	// The friend notices and freezes. Queued changes cannot be applied while frozen.
	w.slot += 500;
	w.must(&friend, Op::Freeze { target: alice.id });
	assert_eq!(w.plain(PlainOp::PolicyExec { account: alice.id, index: 0 }), st(Status::Frozen));

	// Frozen, the account can do one thing: drop queued changes. That only keeps it as
	// it is, and it means nobody who controls a freeze can hold the owner still until a
	// queued change comes due.
	w.must(&alice, Op::PolicyCancel { index: 1 }); // drop the thief's key
	assert_eq!(
		w.op(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(10) })),
		st(Status::Frozen)
	);

	// Strengthening is always immediate (once the freeze is over).
	w.must(&friend, Op::Unfreeze { target: alice.id });
	w.must(&alice, Op::Policy(PolicyChange::SetLimit { asset: brl, per_day: Some(10) }));
	assert_eq!(w.account(&alice.id).policy.unwrap().limits[0].per_day, 10);

	// A queued change that has waited long enough can be applied by anybody.
	assert_eq!(w.plain(PlainOp::PolicyExec { account: alice.id, index: 0 }), OK);
	assert!(w.account(&alice.id).policy.unwrap().limits.is_empty());
	assert_eq!(w.plain(PlainOp::PolicyExec { account: alice.id, index: 7 }), st(Status::NotFound));
}

#[test]
fn every_way_into_an_account_waits_out_the_delay() {
	let mut w = World::new();
	let (alice, friend, sock) = (actor(2), actor(4), actor(60));
	w.must(&alice, Op::Policy(PolicyChange::SetDelay(500)));
	w.must(&alice, Op::Policy(PolicyChange::SetGuardian(Some(friend.id))));
	let pending = |w: &World| w.account(&alice.id).policy.unwrap().pending.len();

	// Naming people who can install a new owner key is a way in, even when there was
	// no such group before. It waits like any other.
	let group = RecoveryCfg { guardians: vec![sock.id], threshold: 1, delay: 0 };
	w.must(&alice, Op::Policy(PolicyChange::SetRecovery(Some(group))));
	assert_eq!(pending(&w), 1);
	assert!(w.account(&alice.id).policy.unwrap().recovery.is_none());
	assert_eq!(w.op(&sock, Op::RecoveryApprove { target: alice.id, new_owner: sock.id }), st(Status::NotFound));

	// The freeze length is also the guardian's cool-down, so changing it in either
	// direction waits too, and it is bounded.
	w.must(&alice, Op::Policy(PolicyChange::SetFreezeSlots(MAX_FREEZE_SLOTS)));
	assert_eq!(pending(&w), 2);
	assert_eq!(
		w.op(&alice, Op::Policy(PolicyChange::SetFreezeSlots(MAX_FREEZE_SLOTS + 1))),
		st(Status::Invalid)
	);
	// The delay cannot reach the cool-down: the owner must always be able to remove a
	// guardian between two of its freezes.
	assert_eq!(
		w.op(&alice, Op::Policy(PolicyChange::SetDelay(DEFAULT_FREEZE_SLOTS))),
		st(Status::Invalid)
	);
	assert_eq!(pending(&w), 2);
}

#[test]
fn social_recovery_replaces_a_lost_key() {
	let mut w = World::new();
	let (bank, alice, new_key, bob) = (actor(1), actor(2), actor(20), actor(3));
	let (g1, g2, g3) = (actor(31), actor(32), actor(33));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.accept(&bob, brl);
	w.must(
		&alice,
		Op::Policy(PolicyChange::SetRecovery(Some(RecoveryCfg {
			guardians: vec![g1.id, g2.id, g3.id],
			threshold: 2,
			delay: 300,
		}))),
	);
	// Alice loses her key. Two of her three guardians vouch for the new one.
	let approve = |key: &Actor| Op::RecoveryApprove { target: alice.id, new_owner: key.id };
	assert_eq!(w.op(&bob, approve(&bob)), st(Status::Unauthorized));
	w.must(&g1, approve(&new_key));
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), st(Status::TooEarly));
	// A guardian voting for somebody else does not disturb the count.
	w.must(&g3, approve(&bob));
	assert_eq!(w.account(&alice.id).policy.unwrap().recovery_req.unwrap().eta, 0);
	w.must(&g2, approve(&new_key));
	let req = w.account(&alice.id).policy.unwrap().recovery_req.unwrap();
	assert_eq!((req.new_owner, req.eta), (new_key.id, w.slot + 300));
	// Once the count is reached, the choice is fixed.
	assert_eq!(w.op(&g3, approve(&bob)), st(Status::Exists));

	// The delay gives the real owner time to object.
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), st(Status::TooEarly));
	w.slot += 300;
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), OK);

	// The account identifier and the balance are unchanged; the key that controls them is new.
	assert_eq!(w.bal(&alice, brl), 100);
	assert_eq!(w.op(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 1 }), st(Status::Unauthorized));
	assert_eq!(w.op_as(&new_key, &alice.id, Op::Transfer { asset: brl, to: bob.id, amount: 1 }), OK);
	assert_eq!(w.bal(&bob, brl), 1);
}

#[test]
fn the_owner_can_veto_a_recovery() {
	let mut w = World::new();
	let (alice, g1, g2, mallory) = (actor(2), actor(31), actor(32), actor(66));
	w.must(
		&alice,
		Op::Policy(PolicyChange::SetRecovery(Some(RecoveryCfg {
			guardians: vec![g1.id, g2.id],
			threshold: 2,
			delay: 300,
		}))),
	);
	// The guardians collude.
	w.must(&g1, Op::RecoveryApprove { target: alice.id, new_owner: mallory.id });
	w.must(&g2, Op::RecoveryApprove { target: alice.id, new_owner: mallory.id });
	// Alice still has her key and says no.
	w.must(&alice, Op::RecoveryCancel);
	w.slot += 300;
	assert_eq!(w.plain(PlainOp::RecoveryExec { account: alice.id }), st(Status::NotFound));
	assert_eq!(w.op(&alice, Op::RecoveryCancel), st(Status::NotFound));
	// Nonsense configurations are refused.
	for bad in [
		RecoveryCfg { guardians: vec![], threshold: 0, delay: 1 },
		RecoveryCfg { guardians: vec![g1.id], threshold: 2, delay: 1 },
		RecoveryCfg { guardians: vec![g1.id, g1.id], threshold: 1, delay: 1 },
	] {
		assert_eq!(w.op(&alice, Op::Policy(PolicyChange::SetRecovery(Some(bad)))), st(Status::Invalid));
	}
}

#[test]
fn cancelling_and_bumping_nonces() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	w.accept(&bob, brl);
	// Alice signs two transfers and hands them to somebody (they are not submitted yet).
	// (Her nonce 0 went on accepting the asset.)
	let t1 = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 10 }); // nonce 1
	let t2 = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 20 }); // nonce 2
	let t3 = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 30 }); // nonce 3
	// She cancels the first by number, and everything below 3 with a bump.
	w.must(&alice, Op::Cancel { nonces: vec![1] });
	assert_eq!(w.submit(vec![t1]), vec![st(Status::Nonce)]);
	w.must(&alice, Op::BumpNonce { floor: 3 });
	assert_eq!(w.submit(vec![t2]), vec![st(Status::Nonce)]);
	assert_eq!(w.submit(vec![t3]), vec![OK]);
	assert_eq!(w.bal(&bob, brl), 30);
	assert_eq!(w.op(&alice, Op::BumpNonce { floor: 1 }), st(Status::Invalid));
	// Cancelling by number reaches only the window of recent nonces. Further ahead it
	// says so, rather than report a cancellation that did not happen.
	let floor = w.account(&alice.id).nonce_floor;
	assert_eq!(w.op(&alice, Op::Cancel { nonces: vec![floor + NONCE_WINDOW] }), st(Status::Nonce));
	assert_eq!(w.op(&alice, Op::Cancel { nonces: vec![floor + NONCE_WINDOW - 1] }), OK);
}

#[test]
fn upgrades_are_announced_and_delayed() {
	let admin = actor(9);
	let mut w = World::with_genesis(Genesis { admin: Some(admin.id), upgrade_delay: 1_000 });
	let stranger = actor(66);
	let code_hash = blake2b(&[b"new code"]);
	let propose = Op::GovProposeUpgrade { code_hash, code_len: 8, min_item_gas: 10, min_memo_gas: 20 };

	assert_eq!(w.op(&stranger, propose.clone()), st(Status::Unauthorized));
	assert_eq!(w.plain(PlainOp::GovExecUpgrade), st(Status::NotFound));
	w.must(&admin, propose.clone());
	// The announcement is public state, and the chain was asked to hold the new code.
	let up = w.config().upgrade.unwrap();
	assert_eq!((up.code_hash, up.eta), (code_hash, w.slot + 1_000));
	assert_eq!(w.host.solicited, vec![(code_hash, 8)]);

	// Not before the delay, not even for the administrator.
	assert_eq!(w.plain(PlainOp::GovExecUpgrade), st(Status::TooEarly));
	w.slot += 1_000;
	// Not before the code itself is on-chain for anyone to read.
	assert_eq!(w.plain(PlainOp::GovExecUpgrade), st(Status::NotFound));
	w.host.preimages.push(code_hash);
	assert_eq!(w.plain(PlainOp::GovExecUpgrade), OK);
	assert_eq!(w.host.upgraded_to, Some((code_hash, 10, 20)));
	assert!(w.config().upgrade.is_none());

	// An announced upgrade can be withdrawn.
	w.must(&admin, propose.clone());
	w.must(&admin, Op::GovCancelUpgrade);
	assert_eq!(w.plain(PlainOp::GovExecUpgrade), st(Status::NotFound));

	// The role can be handed over, or given up for good.
	w.must(&admin, Op::GovSetAdmin { admin: Some(stranger.id) });
	assert_eq!(w.op(&admin, propose.clone()), st(Status::Unauthorized));
	w.must(&stranger, Op::GovSetAdmin { admin: None });
	assert_eq!(w.op(&stranger, propose), st(Status::Unauthorized));
	assert_eq!(w.config().admin, None);
}

#[test]
fn without_an_administrator_the_code_is_final() {
	let mut w = World::new();
	let anyone = actor(5);
	let propose = Op::GovProposeUpgrade { code_hash: [1; 32], code_len: 1, min_item_gas: 0, min_memo_gas: 0 };
	assert_eq!(w.op(&anyone, propose), st(Status::Unauthorized));
	assert_eq!(w.op(&anyone, Op::GovSetAdmin { admin: Some(anyone.id) }), st(Status::Unauthorized));
	assert_eq!(w.config().admin, None);
}

#[test]
fn running_out_of_deposit_loses_nothing() {
	let mut w = World::new();
	let (bank, alice, bob) = (actor(1), actor(2), actor(3));
	let brl = w.token(&bank, "BRL", 0);
	w.mint(&bank, brl, &alice, 100);
	let before = w.host.store.clone();
	// The service's balance no longer covers new storage.
	w.host.capacity = Some(w.host.bytes());
	let item = w.sign(&alice, Op::Transfer { asset: brl, to: bob.id, amount: 60 });
	let payload = encode_payload(&[item]);
	let output = jamx_core::refine::refine(SVC, &payload);
	let mut host = w.host.clone();
	let genesis = w.genesis.clone();
	let slot = w.slot;
	let aborted = std::panic::catch_unwind(move || {
		let mut s = jamx_core::accumulate::Session::new(&mut host, SVC, slot, &genesis);
		s.work(&[0; 32], &output, u64::MAX);
	});
	// The invocation aborts, and the host falls back to the last checkpoint, which is the
	// state before this work item: no half-applied transfer.
	assert!(aborted.is_err());
	assert_eq!(w.host.committed, before);
	assert_eq!(Account::from_bytes(&w.host.committed[&key::account(&alice.id)]).unwrap().free(brl), 100);
}

#[test]
fn asset_creation_is_validated() {
	let mut w = World::new();
	let a = actor(1);
	let token = |flags, attestors: Vec<AccountId>, threshold| Op::AssetCreate {
		symbol: sym("X"),
		decimals: 0,
		flags,
		kind: AssetKindCfg::Token,
		attestors,
		threshold,
	};
	assert_eq!(w.op(&a, token(0x80, vec![], 0)), st(Status::Invalid));
	assert_eq!(w.op(&a, token(0, vec![], 1)), st(Status::Invalid));
	assert_eq!(w.op(&a, token(0, vec![[1; 32]], 0)), st(Status::Invalid));
	assert_eq!(w.op(&a, token(0, vec![[1; 32]], 2)), st(Status::Invalid));
	assert_eq!(w.op(&a, token(0, vec![[1; 32], [1; 32]], 1)), st(Status::Invalid));
	let ext = |cfg: ExternalCfg, btc| Op::AssetCreate {
		symbol: sym("E"),
		decimals: 0,
		flags: 0,
		kind: AssetKindCfg::External { cfg, btc },
		attestors: vec![],
		threshold: 0,
	};
	let oracle = |oracles: Vec<AccountId>, threshold| ExternalCfg {
		rail: Rail::Oracle { oracles, threshold },
		bond_bps: 100,
		claim_window: 10,
		min_deal: 1,
	};
	assert_eq!(w.op(&a, ext(oracle(vec![], 0), None)), st(Status::Invalid));
	assert_eq!(w.op(&a, ext(oracle(vec![[1; 32]], 2), None)), st(Status::Invalid));
	assert_eq!(w.op(&a, ext(ExternalCfg { bond_bps: 10_001, ..oracle(vec![[1; 32]], 1) }, None)), st(Status::Invalid));
	assert_eq!(w.op(&a, ext(ExternalCfg { claim_window: 0, ..oracle(vec![[1; 32]], 1) }, None)), st(Status::Invalid));
	// A Bitcoin rail without a starting header is meaningless.
	let spv = ExternalCfg {
		rail: Rail::BtcSpv { confirmations: 6, pay_secs: 3_600, grace_blocks: 2 },
		bond_bps: 100,
		claim_window: 4_000,
		min_deal: 1,
	};
	assert_eq!(w.op(&a, ext(spv, None)), st(Status::Invalid));
	// Nothing was created by any of the above.
	assert_eq!(w.next_asset(), 1);
	assert_eq!(w.op(&a, ext(oracle(vec![[1; 32]], 1), None)), OK);
	assert_eq!(w.next_asset(), 2);
	// External assets have no balances here: they cannot be minted or transferred.
	assert_eq!(w.op(&a, Op::Mint { asset: 1, to: a.id, amount: 1 }), st(Status::Invalid));
	assert_eq!(w.op(&a, Op::Transfer { asset: 1, to: [9; 32], amount: 1 }), st(Status::Invalid));
}
