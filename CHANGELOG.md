# Changelog

## [0.1.0] - 2026-11-14

The first release. Bron reads Dutch and EU legislation, case law and the officiële publicaties through the Bron service at `https://bron.steddle.com/api/v1`, and every answer that quotes a text names the SHA-256 of the document it was read from and the byte range inside it. The JSON contract below holds from this version on.

**Added**

- `bron law`: `search` by any name the register gives a regulation, `about` one regulation, `show` its articles on a day, `parts` every other unit a rendition prints, `contents` its table of contents, `versions` every published rendition, `history` every text one article has had, `changes` between two days unit by unit, `amendments` and `relations` from the register, and `decisions` the rulings LiDO or the cases CELLAR name for one article. Acts of the EU read by `celex:` the same way.
- `bron law published`, `law feed` and `law upcoming`: what the register brought into force in a window, a cursor over everything the service recorded, and the regulations and Staatsblad acts that print provisions not yet applied.
- `bron case`: `show` a ruling, conclusie or besluit of the ACM paragraph by paragraph, or a case of the EU's courts from CELLAR in Dutch or English; `--number` reads one rechtsoverweging of a ruling the service has read for its numbers, and answers `numbers_not_read` for one it has not: with `retry_after` where it will, and without where it could not. Where other paragraphs print the same number, it names them, since the service can't tell the court's own number from a quotation's. Of a conclusie it says a number may be a quoted court's. `relations` names the conclusie, the instances and LiDO's references. `search` ranks the rulings the service holds against a question, and `--beyond-line` shows the hits too far from it to answer it. A search none of whose indexes answered says so under `index_unavailable`, with the 30 seconds to wait. `recent` lists what rechtspraak's feed listed, narrowed by `--subject` or walked by `--since`.
- `bron publication search` and `publication show` over the kamerstukken, the Staatsblad, the Staatscourant and the Tractatenblad, and `bron dossier show` for a kamerstukdossier's papers and what the Tweede Kamer decided on each amendement and motie.
- `bron find`: which article answers a question or a citation.
- `bron quote check`: whether the article, lid, paragraph or unit of a publication a quotation cites holds it word for word, and where not, each difference. `--publication` cites a kamerstuk or a Staatsblad, and `--ordinal` one of its units.
- `bron blob`: the bytes a digest names, hashed as they arrive, or the span cut from them.
- `--json` on every command prints the service's envelope as sent: `bron: 1`, a `grade`, `data` or `error`, any `caveats`, and `as_of` and `as_known_on` where an answer is bound to a moment. A failure the service states keeps its `kind` and `code`.
- Exit codes: 0 ok, 1 usage, 2 not found, 3 network, 4 upstream, 5 local, 6 a quotation `quote check` read is not found where it cites.
- `BRON_MIRROR` names another deployment of the service.
- Prebuilt binaries for macOS, Linux and Windows on GitHub Releases, with shell and PowerShell installers and a Homebrew formula in `steddle/homebrew-tap`.
