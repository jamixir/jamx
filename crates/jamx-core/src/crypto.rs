//! Hashing and signature verification.
//!
//! Everything here is plain computation with no host calls, so it behaves identically in the
//! PVM (refine) and in native tools.

use blake2::{
	digest::{consts::U32, Digest},
	Blake2b,
};
use sha2::Sha256;

pub type Hash32 = [u8; 32];
pub type PubKey = [u8; 32];
pub type Sig = [u8; 64];

/// Blake2b-256 over the concatenation of `parts` (the hash JAM itself uses).
pub fn blake2b(parts: &[&[u8]]) -> Hash32 {
	let mut h = Blake2b::<U32>::new();
	for p in parts {
		h.update(p);
	}
	h.finalize().into()
}

pub fn sha256(data: &[u8]) -> Hash32 {
	Sha256::digest(data).into()
}

/// Bitcoin's double SHA-256.
pub fn sha256d(data: &[u8]) -> Hash32 {
	sha256(&sha256(data))
}

/// Double SHA-256 of two concatenated 32-byte nodes (a Bitcoin Merkle inner node).
pub fn sha256d_pair(a: &Hash32, b: &Hash32) -> Hash32 {
	let mut buf = [0u8; 64];
	buf[..32].copy_from_slice(a);
	buf[32..].copy_from_slice(b);
	sha256d(&buf)
}

/// Strict Ed25519 verification (rejects small-order keys and non-canonical signatures).
pub fn ed25519_verify(key: &PubKey, msg: &[u8], sig: &Sig) -> bool {
	let Ok(vk) = ed25519_dalek::VerifyingKey::from_bytes(key) else { return false };
	let sig = ed25519_dalek::Signature::from_bytes(sig);
	vk.verify_strict(msg, &sig).is_ok()
}

/// Sign with a 32-byte seed. Used by wallets, tools and tests; never by the service itself.
pub fn ed25519_sign(seed: &[u8; 32], msg: &[u8]) -> Sig {
	use ed25519_dalek::Signer;
	ed25519_dalek::SigningKey::from_bytes(seed).sign(msg).to_bytes()
}

/// The public key belonging to a 32-byte seed.
pub fn ed25519_public(seed: &[u8; 32]) -> PubKey {
	ed25519_dalek::SigningKey::from_bytes(seed).verifying_key().to_bytes()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sign_verify() {
		let seed = [7u8; 32];
		let pk = ed25519_public(&seed);
		let sig = ed25519_sign(&seed, b"hello");
		assert!(ed25519_verify(&pk, b"hello", &sig));
		assert!(!ed25519_verify(&pk, b"hellp", &sig));
		let mut bad = sig;
		bad[5] ^= 1;
		assert!(!ed25519_verify(&pk, b"hello", &bad));
		assert!(!ed25519_verify(&[0u8; 32], b"hello", &sig));
	}

	#[test]
	fn known_hashes() {
		// sha256d("hello") is a well-known vector.
		assert_eq!(
			hex::encode(sha256d(b"hello")),
			"9595c9df90075148eb06860365df33584b75bff782a510c6cd4883a419833d50"
		);
		// blake2b-256("") from the BLAKE2 reference vectors.
		assert_eq!(
			hex::encode(blake2b(&[])),
			"0e5751c026e543b2e8ab2eb06099daa1d1e5df47778f7787faab45cdf12fe3a8"
		);
		assert_eq!(blake2b(&[b"ab", b"c"]), blake2b(&[b"abc"]));
	}
}
