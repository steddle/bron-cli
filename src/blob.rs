//! The name a document is addressed by.
//!
//! Everything fetched is hashed before anything parses it, and the digest
//! travels with the answer. A digest names one sequence of bytes forever, so a
//! reader who wants to check a quotation fetches the address beside it, hashes
//! what arrives, and compares. The service addresses its blobs the same way,
//! which is what makes a span into one of them resolvable by anybody.
//!
//! What is hashed is the *decoded* response body, after any transfer encoding
//! the transport applied. It is byte-identical to what the parser saw, which is
//! the property that matters here, and not necessarily byte-identical to what
//! travelled over the wire.

use std::fmt;
use std::str::FromStr;

use serde::Serialize;
use sha2::{Digest, Sha256};

/// SHA-256 of a blob's contents.
///
/// Held as the raw digest rather than as hex: it is `Copy`, costs no
/// allocation, and cannot be constructed in a state that differs only by case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobId([u8; 32]);

#[derive(Debug, thiserror::Error)]
#[error("not a sha-256 digest: {0}")]
pub struct InvalidDigest(String);

impl BlobId {
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
}

impl FromStr for BlobId {
    type Err = InvalidDigest;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || InvalidDigest(text.to_string());

        if text.len() != 64 {
            return Err(invalid());
        }

        let mut digest = [0u8; 32];
        for (slot, pair) in digest.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
            let hex = std::str::from_utf8(pair).map_err(|_| invalid())?;
            *slot = u8::from_str_radix(hex, 16).map_err(|_| invalid())?;
        }
        Ok(Self(digest))
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl Serialize for BlobId {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One name for one sequence of bytes, and a different one for any other.
    #[test]
    fn a_digest_is_the_content_and_nothing_else() {
        assert_eq!(
            BlobId::of(b"onrechtmatige daad"),
            BlobId::of(b"onrechtmatige daad")
        );
        assert_ne!(
            BlobId::of(b"onrechtmatige daad"),
            BlobId::of(b"rechtmatige daad")
        );
    }

    #[test]
    fn matches_the_known_sha256_of_the_empty_input() {
        assert_eq!(
            BlobId::of(b"").to_string(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn hex_round_trips_through_from_str() {
        let id = BlobId::of(b"anything");
        assert_eq!(id.to_string().parse::<BlobId>().unwrap(), id);
        assert_eq!(id.to_string().to_uppercase().parse::<BlobId>().unwrap(), id);
    }

    #[test]
    fn rejects_text_that_is_not_a_digest() {
        assert!("../../etc/passwd".parse::<BlobId>().is_err());
        assert!("abc".parse::<BlobId>().is_err());
        assert!("z".repeat(64).parse::<BlobId>().is_err());
    }
}
