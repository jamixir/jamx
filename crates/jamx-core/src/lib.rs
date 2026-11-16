//! JAMX: a non-custodial exchange as a JAM service.
//!
//! This crate is the whole of the service's logic, written against a small [`state::Host`]
//! trait so that exactly the same code runs inside the PVM and in native tests and tools.
//!
//! * [`refine`] is the stateless half: signature checks, the batch auction, Bitcoin
//!   proof-of-work and Merkle receipts.
//! * [`accumulate`] is the stateful half: balances, account rules, deals, the header chain.
#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod accumulate;
pub mod auction;
pub mod btc;
pub mod codec;
pub mod crypto;
pub mod ops;
pub mod refine;
pub mod state;
pub mod types;
