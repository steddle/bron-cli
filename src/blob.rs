//! Content-addressed store of raw upstream documents.
//!
//! Everything fetched is written here before anything parses it, under the
//! SHA-256 of its own bytes. Nothing is ever rewritten: a digest names one
//! sequence of bytes forever, which is what lets a caller freeze what it saw
//! rather than refetch and hope.
//!
//! Byte offsets recorded elsewhere are offsets into these blobs.
//!
//! What is stored is the *decoded* response body, after any transfer encoding
//! the transport applied. It is byte-identical to what the parser saw, which is
//! the property that matters here, and not necessarily byte-identical to what
//! travelled over the wire.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlError, FromSqlResult, ToSqlOutput, ValueRef};
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

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_hex(self) -> String {
        let mut hex = String::with_capacity(64);
        for byte in self.0 {
            use fmt::Write;
            let _ = write!(hex, "{byte:02x}");
        }
        hex
    }
}

impl FromStr for BlobId {
    type Err = InvalidDigest;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        if text.len() != 64 {
            return Err(InvalidDigest(text.to_string()));
        }

        let mut digest = [0u8; 32];
        for (slot, pair) in digest.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
            let hex = std::str::from_utf8(pair).map_err(|_| InvalidDigest(text.to_string()))?;
            *slot = u8::from_str_radix(hex, 16).map_err(|_| InvalidDigest(text.to_string()))?;
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
        s.serialize_str(&self.to_hex())
    }
}

/// Stored as a SQLite BLOB, so the round trip costs no hex encoding.
impl ToSql for BlobId {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        Ok(ToSqlOutput::from(self.0.as_slice()))
    }
}

impl FromSql for BlobId {
    fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
        let bytes = value.as_blob()?;
        let digest: [u8; 32] = bytes
            .try_into()
            .map_err(|_| FromSqlError::Other(Box::new(InvalidDigest(format!("{bytes:x?}")))))?;
        Ok(Self(digest))
    }
}

pub struct BlobStore {
    root: PathBuf,
}

impl BlobStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// Writes bytes and returns their id. Writing the same bytes twice is a
    /// no-op, so callers never need to check first.
    pub fn put(&self, bytes: &[u8]) -> Result<BlobId> {
        let id = BlobId::of(bytes);
        let path = self.path(&id);

        if path.exists() {
            return Ok(id);
        }

        let parent = path.parent().expect("blob paths always have a parent");
        std::fs::create_dir_all(parent)
            .with_context(|| format!("cannot create {}", parent.display()))?;

        // Staged under a name unique to this process so two writers racing on
        // the same content cannot truncate each other's file, then renamed into
        // place, which is atomic within a filesystem on both macOS and Linux.
        let staging = parent.join(format!(".{}.{}.partial", id, std::process::id()));
        std::fs::write(&staging, bytes)
            .with_context(|| format!("cannot write {}", staging.display()))?;

        if let Err(e) = std::fs::rename(&staging, &path) {
            let _ = std::fs::remove_file(&staging);
            return Err(e).with_context(|| format!("cannot commit {}", path.display()));
        }

        Ok(id)
    }

    pub fn get(&self, id: &BlobId) -> Result<Vec<u8>> {
        let path = self.path(id);
        std::fs::read(&path)
            .with_context(|| format!("blob {id} is missing from {}", path.display()))
    }

    /// Reads a blob and confirms it still hashes to its own name.
    ///
    /// This is the read for anything that will be quoted or cited. [`get`] is
    /// the read for anything that will not, and the difference matters: a
    /// corrupted blob returned unchecked yields a confident wrong quotation,
    /// which is worse than an error in a tool whose premise is provenance.
    ///
    /// [`get`]: BlobStore::get
    pub fn get_verified(&self, id: &BlobId) -> Result<Vec<u8>> {
        let bytes = self.get(id)?;
        let found = BlobId::of(&bytes);
        anyhow::ensure!(
            found == *id,
            "blob {id} does not match its digest: the stored bytes now hash to {found}",
        );
        Ok(bytes)
    }

    pub fn contains(&self, id: &BlobId) -> bool {
        self.path(id).exists()
    }

    /// Streams a blob back and confirms it still hashes to its own name.
    ///
    /// This is the integrity check behind every citation: if it fails, an offset
    /// recorded against this document no longer points where it did.
    pub fn verify(&self, id: &BlobId) -> Result<bool> {
        let path = self.path(id);
        let mut file = std::fs::File::open(&path)
            .with_context(|| format!("blob {id} is missing from {}", path.display()))?;
        let mut hasher = Sha256::new();
        io::copy(&mut file, &mut hasher).with_context(|| format!("cannot read blob {id}"))?;
        Ok(BlobId(hasher.finalize().into()) == *id)
    }

    /// Sharded two levels deep: a flat directory of tens of thousands of
    /// documents is slow to stat on every filesystem worth supporting.
    fn path(&self, id: &BlobId) -> PathBuf {
        let hex = id.to_hex();
        self.root.join(&hex[0..2]).join(&hex[2..4]).join(hex)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, BlobStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = BlobStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn digest_is_content_addressed() {
        let (_dir, store) = store();
        let a = store.put(b"onrechtmatige daad").unwrap();
        let b = store.put(b"onrechtmatige daad").unwrap();
        assert_eq!(a, b);
        assert_ne!(a, store.put(b"rechtmatige daad").unwrap());
    }

    #[test]
    fn matches_the_known_sha256_of_the_empty_input() {
        assert_eq!(
            BlobId::of(b"").to_hex(),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn round_trips_and_verifies() {
        let (_dir, store) = store();
        let id = store.put(b"<toestand/>").unwrap();
        assert_eq!(store.get(&id).unwrap(), b"<toestand/>");
        assert!(store.contains(&id));
        assert!(store.verify(&id).unwrap());
    }

    #[test]
    fn detects_a_blob_that_was_tampered_with() {
        let (_dir, store) = store();
        let id = store.put(b"original text").unwrap();
        std::fs::write(store.path(&id), b"substituted!!").unwrap();
        assert!(!store.verify(&id).unwrap());
    }

    #[test]
    fn leaves_no_partial_files_behind() {
        let (dir, store) = store();
        store.put(b"x").unwrap();
        let partials = files_under(dir.path())
            .filter(|p| p.contains(".partial"))
            .count();
        assert_eq!(partials, 0);
    }

    #[test]
    fn hex_round_trips_through_from_str() {
        let id = BlobId::of(b"anything");
        assert_eq!(id.to_hex().parse::<BlobId>().unwrap(), id);
        assert_eq!(id.to_string().to_uppercase().parse::<BlobId>().unwrap(), id);
    }

    #[test]
    fn rejects_text_that_is_not_a_digest() {
        assert!("../../etc/passwd".parse::<BlobId>().is_err());
        assert!("abc".parse::<BlobId>().is_err());
        assert!("z".repeat(64).parse::<BlobId>().is_err());
    }

    #[test]
    fn round_trips_through_sqlite_as_a_blob() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        conn.execute("CREATE TABLE t (blob BLOB PRIMARY KEY)", [])
            .unwrap();

        let id = BlobId::of(b"<uitspraak/>");
        conn.execute("INSERT INTO t VALUES (?1)", rusqlite::params![id])
            .unwrap();

        let back: BlobId = conn
            .query_row("SELECT blob FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(back, id);

        let len: usize = conn
            .query_row("SELECT length(blob) FROM t", [], |r| r.get(0))
            .unwrap();
        assert_eq!(len, 32, "stored as raw bytes, not hex");
    }

    fn files_under(root: &Path) -> impl Iterator<Item = String> {
        let mut found = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    found.push(path.to_string_lossy().into_owned());
                }
            }
        }
        found.into_iter()
    }
}
