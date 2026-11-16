//! Minimal, allocation-light binary codec.
//!
//! Fixed-width integers are little-endian. Lengths and most quantities use an unsigned
//! LEB128 varint so that the refine output (capped at ~48 KiB per work-report) stays small.

use alloc::vec::Vec;

/// Decoding error: the input was truncated or malformed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecodeError;

pub type DecodeResult<T> = Result<T, DecodeError>;

/// A bounds-checked cursor over a byte slice.
#[derive(Clone)]
pub struct Reader<'a> {
	data: &'a [u8],
}

impl<'a> Reader<'a> {
	pub fn new(data: &'a [u8]) -> Self {
		Self { data }
	}
	pub fn remaining(&self) -> usize {
		self.data.len()
	}
	pub fn is_empty(&self) -> bool {
		self.data.is_empty()
	}
	pub fn rest(&mut self) -> &'a [u8] {
		let r = self.data;
		self.data = &[];
		r
	}
	pub fn peek_rest(&self) -> &'a [u8] {
		self.data
	}
	pub fn bytes(&mut self, n: usize) -> DecodeResult<&'a [u8]> {
		if self.data.len() < n {
			return Err(DecodeError);
		}
		let (a, b) = self.data.split_at(n);
		self.data = b;
		Ok(a)
	}
	pub fn array<const N: usize>(&mut self) -> DecodeResult<[u8; N]> {
		let mut out = [0u8; N];
		out.copy_from_slice(self.bytes(N)?);
		Ok(out)
	}
	pub fn u8(&mut self) -> DecodeResult<u8> {
		let (b, rest) = self.data.split_first().ok_or(DecodeError)?;
		self.data = rest;
		Ok(*b)
	}
	pub fn u16(&mut self) -> DecodeResult<u16> {
		Ok(u16::from_le_bytes(self.array()?))
	}
	pub fn u32(&mut self) -> DecodeResult<u32> {
		Ok(u32::from_le_bytes(self.array()?))
	}
	pub fn u64(&mut self) -> DecodeResult<u64> {
		Ok(u64::from_le_bytes(self.array()?))
	}
	pub fn u128(&mut self) -> DecodeResult<u128> {
		Ok(u128::from_le_bytes(self.array()?))
	}
	/// Unsigned LEB128, at most 10 bytes, canonical (no trailing zero groups).
	pub fn var(&mut self) -> DecodeResult<u64> {
		// Most values in this service fit in one byte; take that path without the loop.
		if let Some((b, rest)) = self.data.split_first() {
			if *b < 0x80 {
				self.data = rest;
				return Ok(*b as u64);
			}
		}
		let mut v: u64 = 0;
		let mut shift = 0u32;
		loop {
			let b = self.u8()?;
			if shift == 63 && b > 1 {
				return Err(DecodeError);
			}
			v |= ((b & 0x7f) as u64) << shift;
			if b & 0x80 == 0 {
				if b == 0 && shift != 0 {
					return Err(DecodeError);
				}
				return Ok(v);
			}
			shift += 7;
			if shift > 63 {
				return Err(DecodeError);
			}
		}
	}
	pub fn var_u32(&mut self) -> DecodeResult<u32> {
		u32::try_from(self.var()?).map_err(|_| DecodeError)
	}
	pub fn var_usize(&mut self) -> DecodeResult<usize> {
		usize::try_from(self.var()?).map_err(|_| DecodeError)
	}
	/// A varint length followed by that many bytes.
	pub fn var_bytes(&mut self) -> DecodeResult<&'a [u8]> {
		let n = self.var_usize()?;
		self.bytes(n)
	}
	/// A length-prefixed sub-reader.
	pub fn sub(&mut self) -> DecodeResult<Reader<'a>> {
		Ok(Reader::new(self.var_bytes()?))
	}
	pub fn bool(&mut self) -> DecodeResult<bool> {
		match self.u8()? {
			0 => Ok(false),
			1 => Ok(true),
			_ => Err(DecodeError),
		}
	}
	/// Fails unless everything was consumed.
	pub fn finish(&self) -> DecodeResult<()> {
		if self.data.is_empty() {
			Ok(())
		} else {
			Err(DecodeError)
		}
	}
}

/// Append-only encoder helpers on `Vec<u8>`.
pub trait Writer {
	fn put(&mut self, b: &[u8]);
	fn put_u8(&mut self, v: u8) {
		self.put(&[v]);
	}
	fn put_u16(&mut self, v: u16) {
		self.put(&v.to_le_bytes());
	}
	fn put_u32(&mut self, v: u32) {
		self.put(&v.to_le_bytes());
	}
	fn put_u64(&mut self, v: u64) {
		self.put(&v.to_le_bytes());
	}
	fn put_u128(&mut self, v: u128) {
		self.put(&v.to_le_bytes());
	}
	fn put_bool(&mut self, v: bool) {
		self.put_u8(v as u8);
	}
	fn put_var(&mut self, mut v: u64) {
		loop {
			let b = (v & 0x7f) as u8;
			v >>= 7;
			if v == 0 {
				self.put_u8(b);
				return;
			}
			self.put_u8(b | 0x80);
		}
	}
	fn put_var_bytes(&mut self, b: &[u8]) {
		self.put_var(b.len() as u64);
		self.put(b);
	}
}

impl Writer for Vec<u8> {
	fn put(&mut self, b: &[u8]) {
		self.extend_from_slice(b);
	}
	fn put_u8(&mut self, v: u8) {
		self.push(v);
	}
}

/// Types with a canonical binary form.
pub trait Codec: Sized {
	fn enc(&self, out: &mut Vec<u8>);
	fn dec(r: &mut Reader) -> DecodeResult<Self>;

	fn to_bytes(&self) -> Vec<u8> {
		let mut v = Vec::with_capacity(128);
		self.enc(&mut v);
		v
	}
	/// Decode from a complete buffer, rejecting trailing bytes.
	fn from_bytes(data: &[u8]) -> DecodeResult<Self> {
		let mut r = Reader::new(data);
		let v = Self::dec(&mut r)?;
		r.finish()?;
		Ok(v)
	}
}

pub fn enc_vec<T: Codec>(items: &[T], out: &mut Vec<u8>) {
	out.put_var(items.len() as u64);
	for i in items {
		i.enc(out);
	}
}

/// Decode a length-prefixed vector, refusing lengths above `max` (DoS guard).
pub fn dec_vec<T: Codec>(r: &mut Reader, max: usize) -> DecodeResult<Vec<T>> {
	let n = r.var_usize()?;
	if n > max || n > r.remaining() {
		return Err(DecodeError);
	}
	let mut v = Vec::with_capacity(n);
	for _ in 0..n {
		v.push(T::dec(r)?);
	}
	Ok(v)
}

impl Codec for [u8; 32] {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put(self);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		r.array()
	}
}

impl Codec for u64 {
	fn enc(&self, out: &mut Vec<u8>) {
		out.put_var(*self);
	}
	fn dec(r: &mut Reader) -> DecodeResult<Self> {
		r.var()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn varint_roundtrip_and_canonical() {
		for v in [0u64, 1, 127, 128, 300, 16383, 16384, u32::MAX as u64, u64::MAX - 1, u64::MAX] {
			let mut b = Vec::new();
			b.put_var(v);
			let mut r = Reader::new(&b);
			assert_eq!(r.var().unwrap(), v);
			assert!(r.is_empty());
		}
		// non-canonical: 0x80 0x00 encodes zero with a redundant group
		assert!(Reader::new(&[0x80, 0x00]).var().is_err());
		// overflow: 11 bytes
		assert!(Reader::new(&[0xff; 11]).var().is_err());
		// 10th byte may only be 0 or 1
		let mut too_big = [0xffu8; 10];
		too_big[9] = 0x02;
		assert!(Reader::new(&too_big).var().is_err());
	}

	#[test]
	fn reader_bounds() {
		let mut r = Reader::new(&[1, 2, 3]);
		assert_eq!(r.u16().unwrap(), 0x0201);
		assert!(r.u16().is_err());
		assert_eq!(r.u8().unwrap(), 3);
		assert!(r.finish().is_ok());
	}
}
