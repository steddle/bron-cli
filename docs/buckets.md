# The shared budget

Every host that throttles does so by IP. The budget therefore belongs to the
machine, not to a process, and a limiter that lives inside one process paces
itself impeccably while three of them together go three times too fast.

That is not hypothetical. Three concurrent sweeps against `web.archive.org`,
each pacing itself at 1500 ms, stalled all three. It was recorded at the time as
the archive cutting off after 25 requests. The likelier reading is that we were
the problem.

So the coordination point is **a file, not a transport**. Anything on this
machine that talks to a paced host takes the lock, waits its turn, stamps the
file and releases. This includes programs that are not `bron`: a service in
another language reading and writing the same archive cannot afford to shell out
per request for a sweep of thousands of URLs, so it paces itself against this
file directly. The two are not wrapper and wrappee. They are two consumers of
one budget, and the file is where they meet.

`bron` owns this definition. Changing it breaks the other side.

## Location

```
~/.bron/buckets/{key}.bucket
```

Fixed, and deliberately not the platform data directory, which moves between
macOS and Linux. Another language on this machine has to be able to find it
without implementing per-platform directory rules.

`BRON_HOME` overrides the parent, which is how tests get their own budget.

The key has every non-alphanumeric character replaced by `_`.

## Format

Two lines, each an integer, LF-separated:

```
1786032277123
0
```

| Line | Meaning |
|---|---|
| 1 | unix milliseconds when the most recent request **returned** |
| 2 | unix milliseconds until which every caller must hold off, or `0` |

A missing, empty or unparseable file means no history. That is not an error: the
worst case is one unpaced request, and refusing to run is worse.

## The rules

**Take an exclusive `flock(2)` on the file for the whole read-decide-write.**
Advisory, blocking, released when the descriptor closes, including on a crash.
Every language reaches the same lock through the same call. Claiming the slot
while still holding the lock is what stops two processes reading the same free
slot and both taking it.

**Stamp line 1 when a request returns, not when it is dispatched.** A call that
spent a minute inside its own retries must not hand the next caller a free pass.

**Write a refusal into line 2.** A 429 or 503 answered to one process should
slow every other process, rather than letting them walk into the same wall.

**Honour `Retry-After` from the header.** Not from the body. A well known MCP
implementation parses it out of the body, so it silently never honours it and
always falls back to a default.

**Cap any single wait at 60 seconds.** An uncapped `Retry-After` lets one URL
hold a whole run hostage.

**Never back off for less than the ordinary pacing interval.** The request that
fires because the host just refused you is the one to be more patient with.

## Which key

A key is a budget, not a hostname. Several hosts share one key when the service
throttles them together, and one host splits into several keys when it runs
separate quotas behind one name.

| Key | Interval | Used by |
|---|---|---|
| `web.archive.org` | 1500 ms | The CDX index and the replay endpoint, on both `web.archive.org` and `archive.org` |
| `web.archive.org/save` | 1500 ms | Save Page Now, on `web.archive.org/save*` |
| the hostname | 1000 ms | Everything else |

Which produces `web_archive_org.bucket`, `web_archive_org_save.bucket`, and one
file per other host.

The `bron` CLI itself dials only the publisher hosts, so every bucket it opens
is a hostname key at the default interval. The archive keys are defined here
for the machine's other consumers of that budget — redress-engine's watch
service paces against `web_archive_org.bucket` from the PHP side.

Two of those deserve their reasons stated, because both were nearly got wrong.

**`archive.org` joins `web.archive.org`'s bucket rather than getting its own.**
The Internet Archive answers on both names — `web.archive.org` for the index,
the replay and Save Page Now, `archive.org` for availability and the IAS3
endpoints — and throttles both against the same IP. Two files would mean two
processes pacing independently against one budget, which is the exact failure
this file exists to prevent. It is `web.archive.org`'s bucket and not a new
shared one, because that file already exists and PHP already opens it; minting
a fresh key for the pair would leave the two sides pacing against different
files while both believed they were coordinating.

**Save Page Now gets its own bucket even though it shares a hostname.** It is
governed by a concurrent-session quota rather than by the read rate, and it
hands back a hold-off when that quota is tripped. Writing that hold-off into the
read bucket would stall every index query on the machine — this process and the
PHP one both — over a quota neither of them spent. A refusal must only ever slow
the budget it came from.

## Honouring this from PHP

Two phases, and both are needed.

**Claim.** Open `~/.bron/buckets/web_archive_org.bucket`, `flock` it `LOCK_EX`,
read the two lines, sleep until `max(line1 + interval, line2)`, write `now` to
line 1 while still holding the lock, then release. The write is what claims the
slot: without it two processes can read the same free slot and both take it,
which is the case the lock exists for.

**Then make the request, unlocked.** Holding the lock across it would let one
30-second timeout park every other consumer on the machine for 30 seconds.

**Re-stamp on return.** Take the lock again and write the real return time to
line 1. This is the step that makes line 1 mean what it says above, and it
belongs in a `finally`: a failed request must stamp too, or a run of timeouts
leaves line 1 at the dispatch time of the first one and every other consumer
reads a stale free slot.

On a 429 or a 503, write `now + backoff` to line 2 before releasing. **Do the
same when the connection is simply dropped.** A host that has decided about you
stops answering rather than answering 429, and a client that only recognises a
status code reads that as the service being down and keeps trying: measured at
171 consecutive refusals in one run, every one reported as "cannot reach", none
of them slowing anything down.

An implementation that reads the file without locking will work almost always
and fail exactly when two processes start at once, which is the case it exists
for.
