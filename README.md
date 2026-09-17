# bron

[![License](https://img.shields.io/github/license/steddle/bron-cli)](LICENSE)

Dutch primary legal sources, quotable to the byte.

bron reads legislation, case law and official publications, and every answer names the document it rests on and the byte range inside it. It says what a law said on a given date, not only what it says today. The point isn't search. It's that a claim made here can name the document and the characters behind it, and that the same question asked twice gives the same answer.

Nothing is stored. Every answer is fetched, and what makes it checkable is the digest beside it rather than a copy on your disk.

## Installation

```
git clone https://github.com/steddle/bron-cli.git
cd bron-cli
cargo install --path .
```

### Requires

- Rust 1.85 or newer, for edition 2024.
- No system libraries. TLS goes through rustls, so there's no OpenSSL header to find on Linux.
- A network. bron holds nothing between runs, so every answer is fetched.
- `BRON_HOME` to put the rate-limit buckets somewhere other than `~/.bron/buckets`, which is what a CI run that should pace itself alone wants.

## Quick start

Find the act, then read an article as it stood on a date:

```
$ bron law search burgerlijk wetboek boek 6
BWBR0005289  Burgerlijk Wetboek Boek 6
BWBR0017013  Vaststellingsbesluit tijdelijke AMvB ex artikel 110, Burgerlijk Wetboek Boek 6 [...]
BWBR0039501  Wijzigingswet Burgerlijk Wetboek Boek 6 (tegengaan onredelijk lange betaaltermijnen)
BWBR0046553  Wijzigingswet Burgerlijk Wetboek Boek 6 en Overgangswet nieuw Burgerlijk Wetboek [...]

$ bron law show BWBR0005289 --article 119a --as-of 2019-06-01
as in force on 2019-06-01

Artikel 119a
/Boek6/Titeldeel1/Afdeling11/Artikel119a

1. De schadevergoeding, verschuldigd wegens vertraging in de voldoening van een geldsom, [...]
6. In afwijking van lid 5 kunnen partijen geen uiterste dag van betaling overeenkomen van meer dan zestig dagen indien de schuldenaar een rechtspersoon is die [...]

source: c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3 (349072..361267)
https://bron.steddle.com/api/v1/blobs/c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3
jci1.3:c:BWBR0005289&boek=6&titeldeel=1&afdeling=11&artikel=119a&z=2019-04-01&g=2019-04-01
in force through stb-2017-170 (wijziging)
```

Every answer ends with the digest of the document it was read from, where those bytes can be fetched, the byte range inside them, and the act that put that text in force. Fetch the blob, slice it at `349072..361267`, and you have the publisher's own characters; `--raw` sends that slice along instead. Ask what changed since, and bron names the amending act per article:

```
$ bron law changes BWBR0005289 --from 2019-01-01 --article 119a
BWBR0005289
2019-01-01_0 (2019-01-01 to 2019-03-31) -> 2026-07-16_0 (2026-07-16 onwards)

29 amended, 5 added, 1 repealed, 0 withdrawn, 305 unchanged

AMENDED    Artikel 119a
           through stb-2022-146
  [...] uiterste dag van betaling overeenkomen van meer dan [-zestig-] {+30+} dagen indien de schuldenaar een rechtspersoon is die [...]
```

Case law works the same way. Fetch a decision once, and the appeal question is answered from what the register holds:

```
$ bron case show ECLI:NL:HR:2019:1278
ECLI:NL:HR:2019:1278
Hoge Raad
decided 2019-07-19

Prejudiciële vraag (art. 392 Rv). Aardbevingsschade in Groningen. [...]

relations
  earlier conclusie ECLI:NL:PHR:2019:496

appeal: no published successor known, as of 2026-08-20
source: 0dba13d017e9cb0a4dc7c77a5af767559ef454c5ab9425e5185a4418a01647cf

$ bron case appeal ECLI:NL:HR:2019:1278
no published successor known, as of 2026-08-20
```

`--as-of` asks what was in force. `--as-known-on` asks what the register showed, which becomes a different question the moment something is published with retroactive effect. One flag can't choose between them, so there are two.

## Machine use

Every command takes `--json` and answers in a versioned envelope:

```
$ bron --json find 6:162 BW
{"bron":1,"grade":"authority","as_of":"2026-08-24","view":"in_force","data":{"kind":"lookup","citation":"6:162",
 "resolved":[{"reading":"book","source":"wetgeving","law_id":"BWBR0005289","version":"2026-07-16_0",
 "number":"162","held":"text","label":"Artikel 162","path":"/Boek6/Titeldeel3/Afdeling1/Artikel162",
 "content_hash":"f8125445cab2e8d849dacd4a7d5e01fdb8c4b7dad0de796c4f64afafe1626130"}]}}
```

Human and JSON output share one code path, so they can't drift apart. On failure the JSON still goes to stdout and the human sentence to stderr, so a caller never has to read stderr to tell one failure from another:

```
0  ok
1  usage
2  not found      well-formed query, nothing of that name exists
3  network        unreachable, timed out, refused
4  upstream       reachable, but the answer could not be read
5  local          bron could not do its own part on this machine
```

A 404 from upstream is a 2, not a 3. It's the service answering, and reporting it as transport has a pipeline retry something that will never exist.

`grade` says where an answer's authority comes from. Every source bron reads speaks with its own force, so it is always `authority`; it's stated rather than left implicit because a claim's grade is the one field a consumer should never infer.

## Reading through the service

`bron law` reads through `https://bron.steddle.com/api/v1`. That service holds every manifest the register publishes, so it knows which renditions exist, which windows they cover and which the publisher has withdrawn, and it does that arithmetic once for everybody. Which rendition a date resolves to is one rule, and running a second copy of it here is how a CLI comes to answer with text the register pulled months ago.

Nothing has to be taken on trust. Every answer names the blob it was read from, addressed by the hash of its own content, and the offsets of the article inside it. Fetch those bytes, hash them, slice them, and either they say what bron said or they do not. `provenance.frozen_by` says who froze them: `bron`, for a decision or a publication it fetched itself, or `service`.

```
bron law show BWBR0005289 --article 162 --raw   # send the slice too, on the service's word
BRON_MIRROR=https://bron.example.org/api/v1     # another deployment
```

Case law and official publications are read straight from the publisher, because neither has a service equivalent that would add anything: both registers already answer by identifier, and the digest travels with the answer either way.

Nothing is kept. bron writes one thing between runs, its share of the per-host rate-limit budget, so every answer is fetched and without a network there is no answer. A question put to `bron find` travels in a POST body for the same reason nothing is written down: the material is public, the interest in it is not.

## Features

- Every published version of a law, with geldigheid and zicht kept as separate windows rather than collapsed into one date, and renditions the publisher has withdrawn named rather than quietly served.
- Content-addressed provenance. Each document is named by the SHA-256 of its own bytes, and every article and lid carries a byte range into it, so a reader can fetch those bytes and cut the quotation again.
- Amendment diffs. `bron law changes` matches articles on upstream's own identity, so a renumbered article reads as one amendment rather than a removal plus an addition.
- An appeal answer that is never a boolean. `no_published_successor` is dated to the moment bron read the decision, because 'nothing published' is a claim about what the register held then and not about whether an appeal was brought.
- Per-host pacing through a file lock, so several processes on one machine share one budget against the publishers rather than each pacing itself perfectly and together going three times too fast.
- Nothing kept between runs. bron writes its share of the rate-limit budget and nothing else, so there is no store to migrate, verify or grow stale.

## Documentation

- [docs/buckets.md](docs/buckets.md): how the shared rate-limit budget works, and how a non-Rust process joins it.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## Credits

- [Mischa Sigtermans](https://github.com/mischasigtermans)
- Data from [KOOP](https://www.koopoverheid.nl) (Basis Wetten Bestand, Officiële Bekendmakingen) and [de Rechtspraak](https://www.rechtspraak.nl). bron fetches on your behalf and keeps none of it. The only material shipped here is the frozen responses under `tests/fixtures/` that the test suite parses; Auteurswet art. 11 puts wetten and rechterlijke uitspraken outside copyright.

## License

MIT. See [LICENSE](LICENSE).
