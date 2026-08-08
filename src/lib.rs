//! Dutch primary legal sources on your disk.
//!
//! bron fetches wetgeving, court decisions and official publications, freezes
//! the exact bytes it received, and stores what it parsed alongside byte spans
//! back into those bytes. The point is not search. The point is that a claim
//! made from this corpus can name the document and the characters it rests on,
//! and that the same question asked twice gives the same answer.
//!
//! Three rules run through the whole crate:
//!
//! - **Identity is the official identifier.** [`ids::BwbId`], [`ids::Ecli`],
//!   [`ids::PublicationId`]. Never a URL.
//! - **Provenance is not optional.** Every parsed value carries the
//!   [`blob::BlobId`] of the document it came from.
//! - **Nothing in the resolution path is nondeterministic.** No clock reads
//!   inside writes, no generated identifiers in requests, no model anywhere.

#![deny(clippy::all)]

pub mod blob;
pub mod db;
pub mod diff;
pub mod error;
pub mod ids;
pub mod mirror;
pub mod model;
pub mod paths;
pub mod ratelimit;
pub mod sources;
pub mod wire;
