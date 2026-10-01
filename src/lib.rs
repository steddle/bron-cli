//! Dutch primary legal sources, quotable to the byte.
//!
//! bron reads wetgeving, court decisions and official publications through
//! [`service`], which holds them and names the blob each answer came out of.
//! The point is not search. The point is that a claim made from bron can name
//! the document and the characters it rests on, and that the same question
//! asked twice gives the same answer.
//!
//! Three rules run through the whole crate:
//!
//! - **Identity is the official identifier.** A BWB id, an ECLI, a
//!   publication id. Never a URL.
//! - **Provenance is not optional.** Every value carries the digest of the
//!   document it came from, which the service froze and serves by that digest.
//! - **Nothing in the resolution path is nondeterministic.** No clock reads
//!   inside writes, no generated identifiers in requests, no model anywhere.

#![deny(clippy::all)]

pub mod blob;
pub mod error;
pub mod service;
pub mod transport;
pub mod wire;
