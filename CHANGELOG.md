# Changelog

## [Unreleased]

Nothing is tagged or published yet. Everything below can still change without a migration note, and the JSON contract is not stable until there is a `v0.1.0`.

**Added**

- `bron law`: search by citeertitel, show a law or one article, list every published version, and diff a law between two dates. Articles are matched on upstream's own identity, so a renumbered article reads as one amendment rather than a removal plus an addition.
- `--as-of` and `--as-known-on` as separate flags. Geldigheid says what applied, zicht says what the register showed, and they diverge whenever something is published with retroactive effect. Both resolve once at startup so a run cannot straddle midnight.
- `bron case`: search decisions, fetch one by ECLI, and ask what the corpus can say about appeal or cassation. The appeal answer is three-valued; `no_published_successor` is a claim about what the register holds, not about whether an appeal was brought.
- `bron publication`: search and fetch kamerstukken, Staatsblad, Staatscourant and Tractatenblad.
- `bron archive`: list captures, show a page as it stood on a date, and submit a page for capture. Answers are graded `archive` and dated to the capture, never to today. Capture is the only command that writes to a third party and the only one needing credentials.
- Content-addressed document store. Every fetched document is held under the SHA-256 of its own bytes, recorded in a provenance ledger, and referenced by every row derived from it. Articles and leden carry byte ranges back into those bytes.
- `bron verify`, which re-hashes every frozen document against its own name.
- `--json` on every command, in a versioned envelope carrying a `grade` that separates a statute or ruling speaking with its own force from an archive's record of what a page said on a day.
- Exit-code contract: 0 ok, 1 usage, 2 not found, 3 network, 4 upstream, 5 corpus. On failure with `--json` the error is still JSON on stdout, with the human sentence on stderr.
- Per-host rate limiting coordinated through a file lock, so processes sharing a machine share one budget. Any language that can take an `flock(2)` can join it; see [docs/buckets.md](docs/buckets.md).
- `BRON_HOME` to relocate the corpus and blob store, for a per-project corpus in CI.
