//! Dutch primary legal sources, quotable to the byte.
//!
//! bron reads wetgeving, court decisions and official publications, and answers
//! with the document each claim rests on and the byte span inside it. Court
//! decisions and publications are hashed as they are read, so the digest
//! travels with the answer and nothing is kept; wetgeving is read through
//! [`service`], which resolves a date against every manifest the register
//! publishes and names the blob its answer came out of. The point is not search. The point is that a claim made from bron
//! can name the document and the characters it rests on, and that the same
//! question asked twice gives the same answer.
//!
//! Three rules run through the whole crate:
//!
//! - **Identity is the official identifier.** [`ids::BwbId`], [`ids::Ecli`],
//!   [`ids::PublicationId`]. Never a URL.
//! - **Provenance is not optional.** Every value carries the [`blob::BlobId`]
//!   of the document it came from, whether bron froze it or the service did.
//! - **Nothing in the resolution path is nondeterministic.** No clock reads
//!   inside writes, no generated identifiers in requests, no model anywhere.

#![deny(clippy::all)]

pub mod blob;
pub mod diff;
pub mod error;
pub mod ids;
pub mod model;
pub mod paths;
pub mod ratelimit;
pub mod service;
pub mod sources;
pub mod wire;
