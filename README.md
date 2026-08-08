# bron

[![Version](https://img.shields.io/github/v/release/mischasigtermans/bron?label=version)](https://github.com/mischasigtermans/bron/releases)
[![License](https://img.shields.io/github/license/mischasigtermans/bron)](LICENSE)

Dutch primary legal sources on your own disk.

bron fetches legislation, case law and official publications, freezes the bytes that arrived, and records byte offsets from everything it stored back into those bytes. It answers what a law said on a given date, not only what it says today. The point isn't search. It's that a claim made from this corpus can name the document and the characters it rests on, and that the same question asked twice gives the same answer.

## Installation

```
git clone https://github.com/mischasigtermans/bron
cd bron
cargo install --path .
```

### Requires

- Rust 1.85 or newer, for edition 2024.
- No system libraries. SQLite is bundled and TLS goes through rustls, so there's no OpenSSL header to find on Linux.
- `BRON_HOME` to put the corpus and blob store somewhere other than `~/.bron`, which is what a per-project corpus in CI wants.
- `BRON_IA_ACCESS_KEY` and `BRON_IA_SECRET_KEY` from [archive.org](https://archive.org/account/s3.php), only for `bron archive capture`. Reading the archive needs neither.

## Quick start

```
bron law search burgerlijk wetboek boek 6
bron law show BWBR0005289 --article 162
bron law show BWBR0005289 --article 162 --as-of 2019-06-01
bron law changes BWBR0005289 --from 2019-01-01 --article 162
bron case appeal ECLI:NL:HR:2019:1278
```

The third command answers with the text that applied in June 2019, not today's. The fourth says what changed since then and names the act that changed it.

`--as-of` asks what was in force. `--as-known-on` asks what the register showed, which becomes a different question the moment something is published with retroactive effect. One flag can't choose between them, so there are two.

## Machine use

Every command takes `--json` and answers in a versioned envelope:

```
$ bron --json db stats
{"bron":1,"grade":"authority","data":{"path":"/Users/you/.bron/bron.db","laws":0,"versions":0,"articles":0,"cases":0,"publications":0,"documents":0}}
```

Human and JSON output share one code path, so they can't drift apart. On failure the JSON still goes to stdout and the human sentence to stderr, so a caller never has to read stderr to tell one failure from another:

```
0  ok
1  usage
2  not found      well-formed query, nothing of that name exists
3  network        unreachable, timed out, refused
4  upstream       reachable, but the answer could not be read
5  corpus         database or blob store unreadable, missing, or failing verification
```

A 404 from upstream is a 2, not a 3. It's the service answering, and reporting it as transport has a pipeline retry something that will never exist.

`grade` separates what a source can carry. `authority` is a statute or a ruling speaking with its own force. `archive` is the Internet Archive's record that a page said something on a day, which is a weaker claim, dated to its capture and never to today.

## Reading through a mirror

By default bron fetches bytes through `https://bron.steddle.com/api/v1`, which already holds most of them and spares both you and the publisher a request. The mirror only ever serves bytes, and only bytes that hash to the digest the publisher stated in its own manifest, which bron reads from the publisher either way. A mirror that altered a text would fail that hash.

Every answer names which of the two served it in `resolved_via`, because a dossier must not be able to hide that its text came through an intermediary.

```
bron law show BWBR0005289 --article 162 --direct   # this run, publisher only
BRON_MIRROR=                                       # opt out for good
BRON_MIRROR=https://mirror.example.org/api/v1          # somewhere else entirely
```

Dates are never resolved through a mirror. Which version applied on a day is read from validity windows that hang on nothing verifiable, so that answer always comes from the publisher's own manifest.

## Features

- Every published version of a law, with geldigheid and zicht kept as separate windows rather than collapsed into one date.
- Content-addressed storage. Each document is held under the SHA-256 of its own bytes, and every article and lid carries a byte range back into it.
- Amendment diffs. `bron law changes` matches articles on upstream's own identity, so a renumbered article reads as one amendment rather than a removal plus an addition.
- A three-valued appeal answer. `appealed`, `no_published_successor` and `unknown` stay distinct, because 'nothing published' is a claim about the register and not about whether an appeal was brought.
- Web archive access for pages no publisher keeps, graded apart from primary sources. Digest checking is three-valued too: only a match is positive evidence, because the index hashes the stored payload and the replay strips its chunk framing.
- Per-host pacing through a file lock, so several processes on one machine share one budget instead of each pacing itself perfectly and together going three times too fast.
- `bron verify` re-hashes every frozen document against its own name.

## Documentation

- [docs/buckets.md](docs/buckets.md): how the shared rate-limit budget works, and how a non-Rust process joins it.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## Credits

- [Mischa Sigtermans](https://github.com/mischasigtermans)
- Data from [KOOP](https://www.koopoverheid.nl) (Basis Wetten Bestand, Officiële Bekendmakingen), [de Rechtspraak](https://www.rechtspraak.nl) and the [Internet Archive](https://archive.org). bron fetches on your behalf and stores the result on your own disk. The only material shipped here is the frozen responses under `tests/fixtures/` that the test suite parses; Auteurswet art. 11 puts wetten and rechterlijke uitspraken outside copyright.

## License

MIT. See [LICENSE](LICENSE).
