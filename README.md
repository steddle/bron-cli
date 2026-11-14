# bron

[![Version](https://img.shields.io/github/v/release/steddle/bron-cli?label=version)](https://github.com/steddle/bron-cli/releases)
[![License](https://img.shields.io/github/license/steddle/bron-cli)](LICENSE)

Dutch primary legal sources, quotable to the byte.

bron reads legislation, case law and official publications, and every answer names the document it rests on and the byte range inside it. It says what a law said on a given date, not only what it says today. The point isn't search. It's that a claim made here can name the document and the characters behind it, and that the same question asked twice gives the same answer.

Nothing is stored. Every answer is fetched, and what makes it checkable is the digest beside it rather than a copy on your disk.

## Installation

```
brew install steddle/tap/bron
```

Or `cargo install bron --locked`, or a prebuilt binary for macOS, Linux or Windows from the [releases](https://github.com/steddle/bron-cli/releases), which carry a shell and a PowerShell installer.

### Requires

- A network. bron holds nothing between runs, so every answer is fetched.
- To build from source: Rust 1.88 or newer. No system libraries: TLS goes through rustls, so there's no OpenSSL header to find on Linux.

## Quick start

Find the act, then read an article as it stood on a date:

```
$ bron law search burgerlijk wetboek boek 6
BWBR0005289  wet   history  63 of 63 held  Burgerlijk Wetboek Boek 6
BWBR0017013  AMvB  history  1 of 1 held    Vaststellingsbesluit tijdelijke AMvB ex artikel 110, Burgerlijk Wetboek Boek 6 [...]
BWBR0039501  wet   current  1 of 1 held    Wijzigingswet Burgerlijk Wetboek Boek 6 (tegengaan onredelijk lange betaaltermijnen)
BWBR0046553  wet   current  1 of 1 held    Wijzigingswet Burgerlijk Wetboek Boek 6 en Overgangswet nieuw Burgerlijk Wetboek [...]

$ bron law show BWBR0005289 --article 119a --as-of 2019-06-01
as in force on 2019-06-01, as the register showed on 2026-11-14
from rendition 2019-04-01_0, in force 2019-04-01 to 2019-07-20

Artikel 119a
/Boek6/Titeldeel1/Afdeling11/Artikel119a

1. De schadevergoeding, verschuldigd wegens vertraging in de voldoening van een geldsom, [...]
6. In afwijking van lid 5 kunnen partijen geen uiterste dag van betaling overeenkomen van meer dan zestig dagen indien de schuldenaar een rechtspersoon is die [...]

source: c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3 (349072..361267)
https://bron.steddle.com/api/v1/blobs/c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3
jci1.3:c:BWBR0005289&boek=6&titeldeel=1&afdeling=11&artikel=119a&z=2019-04-01&g=2019-04-01
published by stb-2017-170 (wijziging), dossier 34559
in force through stb-2017-171
```

`law show` answers articles. What a rendition prints beside them, the aanhef, a bijlage, the closing formula, is `law parts`: `bron law parts BWBR0001941 --kind bijlage --number Ia`, or `--position` for a unit that prints no number.

Every answer ends with the digest of the document it was read from, where those bytes can be fetched, the byte range inside them, the act that published that text and the one that put it in force. `bron blob` makes that check one command: it fetches the document, hashes the bytes as they arrive, refuses any that hash to something else, and hands over what the digest names, or the span cut from it.

```
$ bron blob c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3 --span 349072..361267
<artikel bwb-ng-variabel-deel="/Boek6/Titeldeel1/Afdeling11/Artikel119a" [...]
1338780 bytes hash to c0c5433f5c4ca92aeee2faca17d1cfdef3597d8ec7ec1036b7b03971321af8a3; cut at 349072..361267
```

The bytes go to stdout and what was checked to stderr, so `bron blob <digest> | shasum -a 256` prints the digest back. `--raw` sends the slice along with an answer instead. Ask what changed since, and bron names the amending act per article:

```
$ bron law changes BWBR0005289 --from 2019-01-01 --article 119a
BWBR0005289
from  2019-01-01_0  2019-01-01 to 2019-03-31, hash matches
      https://bron.steddle.com/api/v1/blobs/2d6cc2a355c0686602054e747abdc3aee775bdbff251ec28a9930969643f40d2
to    2026-07-16_0  2026-07-16 onwards, hash matches
      https://bron.steddle.com/api/v1/blobs/091f3702115565abb5061a2fccdc751c16debfed6614d92e0d3b2a186744bd87

in the whole regulation: 29 amended, 0 commenced, 5 added, 1 repealed, 0 withdrawn, 305 unchanged

AMENDED    Artikel 119a
           /Boek6/Titeldeel1/Afdeling11/Artikel119a
           through stb-2022-146 (wijziging); was through stb-2017-170
           spans 350651..362846 -> 348561..360752
  [...] uiterste dag van betaling overeenkomen van meer dan [-zestig-] {+30+} dagen indien de schuldenaar een rechtspersoon is die [...]
```

Case law works the same way. A ruling arrives with its court, its days and its case numbers, then the inhoudsindicatie and every paragraph with its byte range in the document the service froze:

```
$ bron case show ECLI:NL:HR:2024:1780 --limit 4
ECLI:NL:HR:2024:1780
Hoge Raad
decided 2024-11-29, published 2024-11-29
Prejudiciële beslissing
zaaknummer 24/00169, 24/00170
source: de3e3678909a1d2e51c4afc4ffe19139f514ca4d357083576e49fbb3eadbe1ff
https://bron.steddle.com/api/v1/blobs/de3e3678909a1d2e51c4afc4ffe19139f514ca4d357083576e49fbb3eadbe1ff
as the publisher served it since 2026-01-16T10:06:33+01:00

inhoudsindicatie  (4104..4575)
Prejudiciële beslissing (art. 392 Rv). Huurrecht; Europees consumentenrecht. Richtlijn oneerlijke bedingen (Richtlijn 93/13/EEG). Huurprijswijzigingsbeding; indexatiebeding en opslagbeding.

paragraph 2  (4909..4947)
HOGE RAAD DER NEDERLANDEN
[...]

$ bron case relations ECLI:NL:HR:2024:1780
eerdereAanleg  conclusie  ECLI:NL:PHR:2024:771  stated in 828d100ec9d102ad8f51c7aa9096b077c19504945de9bc07635b3ff31de801ce
lx-referentie  BWBR0001827 Artikel 392  Wetboek van Burgerlijke Rechtsvordering  (LiDO's reading)  stated in 489358aee765ad8e6ba97712d1a7522099a86103e6a8633f0fb198c97dfe4ac9
[...]
[...]
lx-referentie  cited by ECLI:NL:RBROT:2026:9667  (LiDO's reading)  stated in 4aa079cb2f87bec872d8b8f4c6820acca3183f279ba73303a9bd1bd121c3ebe8
A relation is the publisher's statement. None says whether an appeal was lodged.

note: LiDO's computer recognised the articles and rulings marked (LiDO's reading) in the ruling's text, and no publisher states them. A reference it recognised can name another regulation than the one the court meant, so read each beside the ruling.
```

`law show` and `law changes` read one law. Two more read the register as a whole, and the
difference between them is why there are two:

```
$ bron law published --from 2026-11-01 --type wet
* BWBR0032203  2026-11-06_0  2026-11-06 onwards        2026-11-06 onwards        matches    0 images    Aanbestedingswet 2012
* BWBR0050739  2026-11-01_0  2026-11-01 onwards        2026-11-01 onwards        matches    0 images    Uitvoeringswet digitaledienstenverordening
* BWBR0048028  2026-11-01_0  2026-11-01 onwards        2026-11-01 onwards        matches    0 images    Wet goed verhuurderschap
[...]
* = the service holds this text
first window = in force (geldigheid), second = shown by the register (zicht), - = the register states none
matches, differs, rewritten, absent, unchecked = what the publisher's hash says of the held bytes

$ bron law feed --since 10 --limit 2
      11  rewritten   BWBR0052799  2026-07-08_0  2026-07-08 onwards        2026-07-08 onwards
      12  rewritten   BWBR0051170  2026-07-11_0  2026-07-11 onwards        2026-07-11 onwards
next 12  head 23032  origin 30fbc1f9317d7da9f25cee30711c1e4f
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

The register publishes no rendition ahead, so what it holds of the future
stands inside the renditions in force, as provisions printed as not yet
commenced. `law upcoming` lists the regulations that print them:

```
$ bron law upcoming --from 2027-01-01 --to 2027-03-31 --limit 3
2027-01-01  BWBR0002565  2026-10-08_0  3 waiting  Overgangswet nieuw Burgerlijk Wetboek  acts stb-2025-156, stb-2026-205  decrees stb-2026-206, stb-2026-82  linked decrees stb-2026-82
2027-01-01  BWBR0005259  2026-10-29_0  1 waiting  Kadasterbesluit  acts stb-2026-338  decrees stb-2026-338  linked decrees -
2027-01-01  BWBR0031719  2026-07-01_0  6 waiting  Regeling Waadi  acts stcrt-2026-6946  decrees stb-2026-159  linked decrees -
[...]
note: this list holds what the BWB prints, and the acts the Staatsblad published that the BWB does not print, as far as bron holds the Staatsblad. An act published before that and never printed in the BWB, the kinds of act counted as left out, and EU regulations are not in it.
```

Every day on it is one the register states. Without `--from` and `--to` the
list also holds regulations whose waiting provisions state no day, after the
dated ones, the newest act first. An act the Staatsblad published from 2020
that the BWB prints nowhere is a row of its own. Treaties are left out and
counted.
`law parts ID --not-commenced` reads one regulation's waiting units of every
kind, a waiting bijlage beside the articles, and `law show ID --not-commenced`
its waiting articles alone.

An act names the kamerstukdossier it came through, and `dossier show` reads
that dossier's papers, oldest first, with what the Tweede Kamer decided on
each amendement and motie:

```
$ bron dossier show 34608 --kind amendement --limit 2
Kamerstukken 34608  Wijziging van het Burgerlijk Wetboek en het Wetboek van Burgerlijke Rechtsvordering [...] (Wet afwikkeling massaschade in collectieve actie)
  stb-2019-130  BWBR0001827 Wetboek van Burgerlijke Rechtsvordering, BWBR0002565 Overgangswet nieuw Burgerlijk Wetboek, [...]
kst-34608-12  2018-12-21  amendement  Stemmen - aangenomen
  Amendement van het lid Van Gent c.s. over aanscherping van de vereiste nauwe band met de Nederlandse rechtssfeer
  Kamerstukken II 2018/19, 34608, nr. 12
kst-34608-13  2018-12-21  amendement  Stemmen - aangenomen
  Amendement van het lid Van Gent c.s. over verbetering van het overgangsrecht
  Kamerstukken II 2018/19, 34608, nr. 13
```

A quotation can be read again in the text it cites. `quote check` says whether the unit it names holds it word for word, and where not, what differs:

```
$ bron quote check "Hij die jegens een ander een onrechtmatige daad pleegt, welke hem kan worden toegerekend, is verplicht de schade die de ander dientengevolge lijdt, te vergoeden." --regulation BWBR0005289 --article 6:162 --paragraph 1
1  found  Artikel 162 lid 1  (507288..507815)
   rendition 2026-07-16_0, as of 2026-11-14, as known on 2026-11-14
   source: 091f3702115565abb5061a2fccdc751c16debfed6614d92e0d3b2a186744bd87
   note: these fields describe article(s) 162, not the lid: amended_by, dossiers, commenced_by, in_force_from, effect, remarks. The register records provenance per article.
```

It exits 0 where every quotation is found and 6 where one is not, so a script checking a brief before it goes out can stop on it. `--file` reads up to ten at once.

`--as-of` asks what was in force. `--as-known-on` asks what the register showed, which becomes a different question the moment something is published with retroactive effect. A moment is a point in both windows, so the two combine: `--as-of 2019-06-01 --as-known-on 2020-01-01` is the text for conduct in June 2019 as the register had it on New Year's Day. `--as-known-on` alone names that day for both. Both go to the service exactly as typed; name neither and the service reads today, and every answer states the moment it resolved.

## Machine use

Every command takes `--json` and answers in a versioned envelope. The service's JSON is the contract: where a command is one call to the service, as every command but `blob` is, `--json` prints the envelope the service answered with, byte for byte, so a field this build has never heard of still reaches you. A failure is always in bron's own envelope, because `network` and `local` are kinds no service can state. Where the service stated the failure, `error.code` carries the service's code for it as sent, `regulation_not_found` or `rendition_withdrawn`, finer than the kind and never needed to pick an exit code, and the whole `error` object is the service's as it arrived, its kind included: `throttled`, `corpus` and `source` read as the API and the MCP give them, and exit 4. A failure bron states itself carries no code, so presence says who stated it. A question goes to the service as asked, so a window that runs backwards, a `--limit` of 0, a date that names no day or an identifier that is none is refused by the service, with its code, exactly as the API and the MCP refuse it; bron refuses only what it alone can judge, such as a `--span` outside the bytes it hashed.

```
$ bron --json find 6:162 BW
{"bron":1,"grade":"authority","as_of":"2026-11-14","as_known_on":"2026-11-14","data":{"kind":"lookup","citation":"6:162","resolved":[{"reading":"book","source":"wetgeving","register":"bwb","jurisdiction":"nl","regulation_id":"BWBR0005289","expression":"2026-07-16_0","held":true,"number":"162","kind":"artikel","label":"Artikel 162","path":"/Boek6/Titeldeel3/Afdeling1/Artikel162","content_hash":"f8125445cab2e8d849dacd4a7d5e01fdb8c4b7dad0de796c4f64afafe1626130","caveats":[]}]}}
```

Human output is rendered from the same answer the JSON came out of, so the two can't describe different things. On failure the JSON still goes to stdout and the human sentence to stderr, so a caller never has to read stderr to tell one failure from another:

```
0  ok
1  usage
2  not found      well-formed query, nothing of that name exists
3  network        unreachable, timed out, refused
4  upstream       reachable, but no answer came of it
5  local          bron could not do its own part on this machine
6  not quoted     quote check: a quotation is not found where it cites
```

A 404 from upstream is a 2, not a 3. It's the service answering, and reporting it as transport has a pipeline retry something that will never exist.

Exit 4 covers two things, and what is printed tells them apart. **The service stated a failure**: the sentence opens 'the service could not answer:', and `--json` carries the service's `error.code`. **Nothing that could be read came back**: the sentence names the HTTP status and whoever sent it, `504 Gateway Timeout` where the service or something in front of it did not answer in time, `502` or `503` where nothing could be passed on, and `--json` carries kind `upstream` and no code because nobody stated one. A 504 is asked again later or narrowed, not filed as a bug, and bron asks once without retrying. Where the service stated why it could not answer, `error.kind` is the service's own, `throttled`, `corpus` or `source`, and `error.code` says which: `rate_limited` is your own limit and `fetch_budget_spent` a budget every caller shares, both with the delay the service named as `retry_after`, in seconds, and a `corpus` code such as `rendition_too_large` or `rendition_unreadable` is a fact about that rendition that no retry changes. What is left, 'returned an unusable response', is an answer that came back and could not be read.

`grade` says where an answer's authority comes from. Every source bron reads speaks with its own force, so it is always `authority`; it's stated rather than left implicit because a claim's grade is the one field a consumer should never infer.

## Reading through the service

Every command reads through `https://bron.steddle.com/api/v1`. That service holds every manifest the register publishes, so it knows which renditions exist, which windows they cover and which the publisher has withdrawn, and it does that arithmetic once for everybody. Which rendition a date resolves to is one rule, and running a second copy of it here is how a CLI comes to answer with text the register pulled months ago.

Nothing has to be taken on trust. Every answer that quotes a text names the blob it was read from, addressed by the hash of its own content, and the offsets of the article or paragraph inside it. `bron blob` fetches those bytes, hashes them and cuts them, and either they say what bron said or they do not. A list of rulings names the frozen feed page or LiDO answer it stands on by digest too; the service holds those and serves none of them, since they go on naming rulings the court has withdrawn.

```
bron law show BWBR0005289 --article 162 --raw   # send the slice too, on the service's word
bron blob <digest> --span 349072..361267        # fetch it, hash it, cut it yourself
BRON_MIRROR=https://bron.example.org/api/v1     # another deployment
```

The service fetches a ruling from rechtspraak on the first read that asks for it and answers every later read from the bytes it froze. A text the publisher replaced is never served again: its digest answers 410, and the ruling names the digests it replaced.

Nothing is kept. bron writes nothing between runs, so every answer is fetched and without a network there is no answer. A question put to `bron find` travels in a POST body for the same reason nothing is written down: the material is public, the interest in it is not.

## Features

- Every published version of a law, with geldigheid and zicht kept as separate windows rather than collapsed into one date, and renditions the publisher has withdrawn named rather than quietly served.
- Acts of the EU. `bron law show celex:32005L0029 --article 7 --as-of 2024-01-01` reads the directive's article as the consolidation in force that day prints it, and `law parts` and `law versions` read the act the same way. Each answer says what the text can be cited as: a consolidation is for information only, and the authentic text is the act in the Official Journal.
- Content-addressed provenance. Each document is named by the SHA-256 of its own bytes, and every article and lid carries a byte range into it, so a reader can fetch those bytes and cut the quotation again, with `bron blob` or with any tool that hashes.
- What is coming. `bron law upcoming` lists the regulations that already print provisions they do not yet apply, with the first day to come where the register states one and the acts and besluiten behind them.
- The life of one article. `bron law history` answers every text it has had, each with the window it applied in and the act that produced it, from the renditions the corpus holds.
- What the register states around a law. `bron law about` says how much of it is held, `bron law relations` what rests on it and what cites it, and `bron law amendments` what the act that established it did elsewhere.
- The rulings under an article. `bron law decisions` lists the rulings LiDO found that cite one article, newest decided first, each with its court, its ECLI, whether bron holds it and the rendition of the law in force on the day it was decided, which is not necessarily the text the court applied. On an act of the EU, `celex:32016R0679`, it lists the cases CELLAR states interpret or cite the article, each with the places it cites, `art. 17(1)(a)`. It says how many were found and how many of them each court decided, and never that no ruling cites the article: how many LiDO or CELLAR misses is not measured.
- The structure of a law. `bron law contents` prints the table of contents of the rendition in force on a day, its boeken, titels and afdelingen with their headings as the register prints them, and under each the numbers of its articles on one line.
- Amendment diffs. `bron law changes` compares a law between two dates unit by unit, its bijlagen and aanhef beside its articles, and matches them on upstream's own identity, so a renumbered article reads as one amendment rather than a removal plus an addition.
- Rulings by ECLI. `bron case show` answers a ruling or a conclusie paragraph by paragraph, each at its byte range and under the number the court prints for it, `r.o. 3.2.10`. `--number 3.2` reads one rechtsoverweging of a ruling the service has read for its numbers, and answers `numbers_not_read` for one it has not. For a tuchtcollege's ruling it names tuchtrecht.overheid.nl, where the text is published, and a besluit of the ACM reads by `acm:{node}`. `bron case relations` answers the conclusie and the instances before and after it, in the publisher's words and never as a finding that an appeal was or was not lodged. `bron case search` ranks the rulings the service has read against a question and names each by the run of paragraphs that ranked, with that run's first words, saying how many rulings it covered: search over all case law is still to come. A hit too far from the question to answer it is held back and counted, and `--beyond-line` shows it, marked. A judgment or an opinion of the EU's courts, `ECLI:EU:C:2019:773`, reads the same way from CELLAR, in Dutch or with `--lang en` in English, each paragraph under the number the Court prints for it; `bron case relations` answers what CELLAR states it interprets and cites, each place as a reader cites it, `art. 8(2)(e)`, and `bron case search --register cellar` ranks them. `bron case recent` lists the rulings with a text rechtspraak's feed listed, newest decided first, each with its court, its ECLI, its case numbers, whether bron holds it and the summary the feed prints, which the service leaves out for a criminal case and for a ruling it withholds, and how many each court has listed. It is not every ruling decided: the feed lists no conclusie and no ruling published as metadata alone. `--subject civiel` or `straf` narrows it to an area of law, and `--since` walks the feed in the order it stamped the rulings, with the stamp to come back with.
- Official publications. `bron publication search` asks the publisher's own ranking over the kamerstukken, the Staatsblad, the Staatscourant and the Tractatenblad, live, and `bron publication show stb-2026-56` answers one with the digest of every manifestation and, where one is XML, the span of each article it prints. A publication that names private persons is refused.
- Quotations checked. `bron quote check` reads a quotation again in the article, lid or paragraph it cites and answers found, not as quoted with each difference, or found elsewhere in the same text, and exits 6 where one is not found.
- Nothing kept between runs. bron writes nothing, so there is no store to migrate, verify or grow stale.

## Changelog

See [CHANGELOG.md](CHANGELOG.md).

## Credits

- [Mischa Sigtermans](https://github.com/mischasigtermans)
- Data from [KOOP](https://www.koopoverheid.nl) (Basis Wetten Bestand, Officiële Bekendmakingen), [de Rechtspraak](https://www.rechtspraak.nl) and the Publications Office of the European Union ([CELLAR](https://op.europa.eu/en/web/cellar)). bron reads them through the bron service and keeps none of it. The repository holds answers of the service, frozen for its tests under `tests/`, and the crate ships none of them; Auteurswet art. 11 puts wetten and rechterlijke uitspraken outside copyright.

## License

MIT. See [LICENSE](LICENSE).
