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

`law show` answers articles. What a rendition prints beside them, the aanhef, a bijlage, the closing formula, is `law parts`: `bron law parts BWBR0001941 --kind bijlage --number Ia`, or `--position` for a unit that prints no number.

Every answer ends with the digest of the document it was read from, where those bytes can be fetched, the byte range inside them, and the act that put that text in force. Fetch the blob, slice it at `349072..361267`, and you have the publisher's own characters; `--raw` sends that slice along instead. Ask what changed since, and bron names the amending act per article:

```
$ bron law changes BWBR0005289 --from 2019-01-01 --article 119a
BWBR0005289
2019-01-01_0 (2019-01-01 to 2019-03-31) -> 2026-07-16_0 (2026-07-16 onwards)

in the whole regulation: 29 amended, 0 commenced, 5 added, 1 repealed, 0 withdrawn, 305 unchanged

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

Those two read one law. Two more read the register as a whole, and the
difference between them is why there are two:

```
$ bron law published --from 2026-09-01 --type wet
* BWBR0020368  2026-09-01_0  2026-09-01 onwards
* BWBR0006622  2026-09-01_0  2026-09-01 onwards
  BWBR0032573  2026-09-01_0  2026-09-01 onwards

* = the service holds this text

$ bron law feed --since 40122
   40123  published   BWBR0024282  2026-07-01_0
   40124  rewritten   BWBR0005289  2002-04-12_0
next 40124  head 40124  origin 8539df1d82a7060824be7ba17f500781
```

`law published` names a window and answers what the register brought into force
inside it. Reproducible, and the wrong command to poll with: the publisher
publishes retroactively, so a rendition arriving tomorrow can carry a date
inside a window you already read. `law feed` walks a cursor in the order the
service recorded things and misses nothing. Keep `next` and `origin` together:
an origin you do not recognise means the corpus was rebuilt and the number
means something else now. Both go to stderr, so a script reads rows on stdout
and takes its cursor from `--json`. An empty walk exits 0, because nothing
happened is an answer.

`--as-of` asks what was in force. `--as-known-on` asks what the register showed, which becomes a different question the moment something is published with retroactive effect. A moment is a point in both windows, so the two combine: `--as-of 2019-06-01 --as-known-on 2020-01-01` is the text for conduct in June 2019 as the register had it on New Year's Day. `--as-known-on` alone names that day for both.

## Machine use

Every command takes `--json` and answers in a versioned envelope. The service's JSON is the contract: where a command is one call to the service, as `find`, `law search`, `law versions`, `law parts`, `law changes`, `law published` and `law feed` are, `--json` prints the envelope the service answered with, byte for byte, so a field this build has never heard of still reaches you. A failure is always in bron's own envelope, because `network` and `local` are kinds no service can state. Where the service stated the failure, `error.code` carries the service's code for it as sent, `regulation_not_found` or `rendition_withdrawn`, finer than the kind and never needed to pick an exit code. A failure bron states itself carries none, so presence says who stated it.

```
$ bron --json find 6:162 BW
{"bron":1,"grade":"authority","as_of":"2026-09-17","as_known_on":"2026-09-17","data":{"kind":"lookup","citation":"6:162","resolved":[{"reading":"book","source":"wetgeving","regulation_id":"BWBR0005289","expression":"2026-07-16_0","held":true,"number":"162","label":"Artikel 162","path":"/Boek6/Titeldeel3/Afdeling1/Artikel162","content_hash":"f8125445cab2e8d849dacd4a7d5e01fdb8c4b7dad0de796c4f64afafe1626130","caveats":[]}]}}
```

Human output is rendered from the same answer the JSON came out of, so the two can't describe different things. On failure the JSON still goes to stdout and the human sentence to stderr, so a caller never has to read stderr to tell one failure from another:

```
0  ok
1  usage
2  not found      well-formed query, nothing of that name exists
3  network        unreachable, timed out, refused
4  upstream       reachable, but no answer came of it
5  local          bron could not do its own part on this machine
```

A 404 from upstream is a 2, not a 3. It's the service answering, and reporting it as transport has a pipeline retry something that will never exist.

Exit 4 is not always worth retrying. Where the service stated why it could not answer, `error.code` says which: `rate_limited` is your own limit and `fetch_budget_spent` a budget every caller shares, both with the delay the service named, and a `corpus` code such as `rendition_too_large` or `rendition_unreadable` is a fact about that rendition that no retry changes.

`grade` says where an answer's authority comes from. Every source bron reads speaks with its own force, so it is always `authority`; it's stated rather than left implicit because a claim's grade is the one field a consumer should never infer.

## Reading through the service

`bron law` reads through `https://bron.steddle.com/api/v1`. That service holds every manifest the register publishes, so it knows which renditions exist, which windows they cover and which the publisher has withdrawn, and it does that arithmetic once for everybody. Which rendition a date resolves to is one rule, and running a second copy of it here is how a CLI comes to answer with text the register pulled months ago.

Nothing has to be taken on trust. Every answer names the blob it was read from, addressed by the hash of its own content, and the offsets of the article inside it. Fetch those bytes, hash them, slice them, and either they say what bron said or they do not. In the wetgeving lane the service froze them and names the digest; a decision or a publication is one bron fetched and hashed itself, and `provenance.frozen_by` on those answers says so and states when.

```
bron law show BWBR0005289 --article 162 --raw   # send the slice too, on the service's word
BRON_MIRROR=https://bron.example.org/api/v1     # another deployment
```

Case law and official publications are read straight from the publisher, because neither has a service equivalent that would add anything: both registers already answer by identifier, and the digest travels with the answer either way.

Nothing is kept. bron writes one thing between runs, its share of the per-host rate-limit budget, so every answer is fetched and without a network there is no answer. A question put to `bron find` travels in a POST body for the same reason nothing is written down: the material is public, the interest in it is not.

## Features

- Every published version of a law, with geldigheid and zicht kept as separate windows rather than collapsed into one date, and renditions the publisher has withdrawn named rather than quietly served.
- Content-addressed provenance. Each document is named by the SHA-256 of its own bytes, and every article and lid carries a byte range into it, so a reader can fetch those bytes and cut the quotation again.
- Amendment diffs. `bron law changes` compares a law between two dates unit by unit, its bijlagen and aanhef beside its articles, and matches them on upstream's own identity, so a renumbered article reads as one amendment rather than a removal plus an addition.
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
