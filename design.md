# Keybase Validator

## A Sketch of Operation in the Steady State

Here's a rough description of what the validator does in its steady state,
focusing just on the Merkle roots and on the users tree, leaving out teams for
now. The word "fetch" here could be interpreted to mean an individual network
fetch, however in the performance section below we'll discuss how we expect to
use batch fetching and caching.

- Once a minute, or when prompted by a caller, the main loop hits
  `merkle/root.json` and fetches the latest merkle root seqno.
- The **root loop**. For each seqno between the most recent one previously
  verified and this new one:
  - Fetch the root node at that seqno.
  - Verify that it's signed correctly by the root key, contains the correct
    seqno, contains the correct prev pointers, etc.
  - Recursively fetch and walk the interior merkle tree nodes under this root,
    finding all the user leaves that differ from the previous verified root.
  - The **user loop**. For each user that was updated, for each sigchain seqno
    between the most recent verified one for that user and this new one:
    - Fetch the sigchain link at that seqno.
    - If needed, fetch keys for the user. Note that "needed" is tricky to
      determine prior to the introduction of the PGP `full_hash` field in the
      sigchain, but we might be able to assume that all new PGP keys from this
      point forward will include that field, and only re-fetch keys when we
      encounter a new `full_hash`.
    - Verify that it's signed correctly by an active sibkey, contains the
      correct seqno, contains the correct prev pointers, etc.
    - Update the user's set of active sibkeys if necessary.

When all of those nested loops complete, every new merkle root has been
incorporated into the local tree, and every user update has been verified. If
any step fails verification, the process flips into a global failure state,
from which it doesn't attempt to recover.

In this design, the API of the validator is a single REST endpoint. Clients
query the validator by sending along the last signed merkle root they saw from
keybase.io. The validator checks signatures on what the client sent and
immediately records it (or confirms that it matches an existing known root with
that seqno). This constitutes a sort of gossip protocol between clients and the
validator, to try to catch any inconsistencies in what keybase.io is reporting.
If the submitted root is newer than what the validator has seen, it immediately
kicks off a new run of the root loop to catch up, and waits for that loop to
complete before responding to the client. After the root loop catches up, if
everything went smoothly, the API call returns success.

If the validator sees any errors during the root loop, or comes across any
inconsistent root nodes that claim the same seqno, it fails the client request
that started it (if any) and all subsequent requests. The request may also fail
with a "not yet bootstrapped" error if the validator hasn't yet completed its
first bootstrapping runs through root loop.

## Caching

All of the above is heavily dependent on local caching to reduce network
fetches. Every time the word "fetch" occurs above, the assumption is that the
validator will first check its cache. Past merkle roots roots and interior
nodes are cached for traversing the tree to collect differences. User keys are
cached to validate new sigchain links without refetching them.

It's possible that we could get away with caching less than everything. For
example, we don't need the contents of past merkle roots, only their hashes to
validate skip pointers from future roots. However, there are other scenarios
besides running the validator in the steady state, which do motivate us to
cache everything:

- Future version updates. The simplest way to add new features will often be to
  blow away and recompute local derived data, like the set of active sibkeys
  for a user. But there's no reason to refetch merkle roots or sigchain links
  in that case, because their contents are immutable. That sort of refetching
  could also become a thundering herd problem, if there are many copies of the
  validator running in the wild. More on the topic of blowing away derived data
  in the section on updates below.
- Development. Much of development will be repeatedly re-running the validator
  against production Merkle tree data, and gradually encountering all the
  corner cases that have come up over time. Avoiding network fetches during
  this process will be helpful.

The following items will be cached in "raw" tables, which should never need to
be dropped or modified:

- Merkle root sigs, indexed by their seqno.
- Merkle interior nodes, indexed by their hash.
- User sigchain links, indexed by their UID and seqno.
- User PGP keys, indexed by their derived KID and `full_hash`.

Other derived data will be cached in separate tables, with the expectation that
these tables can be blown away during a version upgrade if need be. For
example:

- The verified plaintext extracted from raw signatures.
- The "current" Merkle seqno, with respect to either the current steady state
  of the validator, or the intermediate state of the root loop in progress.
  This is a singleton.
- The current seqno for each user sigchain.
- The set of active sibkeys (and their PGP full hashes) for each user.
- The set of all pinned KIDs, and the users that have pinned them.
- The global UID <-> username mapping.

## Bootstrapping and Performance

I've tested PGP and Keybase-flavored-NaCl signature verification in Rust and
put together a [small benchmark
suite](https://github.com/oconnor663/keybase_validator/blob/master/verify_example/benches/bench.rs).
Here are the results from my laptop (`cargo +nightly bench`):

```
bench_nacl_load_key               462 ns/iter (+/- 21)
bench_nacl_verify_merkle_root  73,069 ns/iter (+/- 1,291)
bench_parse_kbsig               3,164 ns/iter (+/- 22)
bench_parse_root_json           9,116 ns/iter (+/- 55)
bench_pgp_load_key            609,595 ns/iter (+/- 7,997)
bench_pgp_verify_merkle_root  277,699 ns/iter (+/- 1,827)
```

Loading a PGP key (the merkle root signing key in this measurement) and
checking a signature take 1ms combined. For comparison, `curl
https://keyase.io/robots.txt` takes about 100ms.

This lets us make some important back-of-the-envelope estimates about
performance requirements. The current merkle seqno as of this writing (14 May
2019) is 5,370,352, with a new root published about every 10 seconds. If
validating every one of those roots requires a single 100ms network fetch and
nothing else, bootstrapping will take 6 days. If validating every one of those
roots requires verifying a single PGP signature and nothing else, bootstrapping
will take 2 hours.

Choosing a performance target is somewhat arbitrary, but my preference is for
bootstrapping to finish "overnight", or in at most 8 hours. That works out to
about 5ms per merkle seqno. It will be impossible to hit that target without
doing network batching to reduce round trips.

Some of that batching can be done with the Keybase API today: Fetching a user's
entire set of keys can be done all at once. Likewise fetching a user's entire
set of sigchain nodes. However, I don't think there's currently an appropriate
endpoint for batching fetches of the entire merkle tree. The behavior would
need to be of the form: "Given a last seen merkle seqno S1, and a requested
seqno S2, send me every merkle root S1 < S <= S2, along with every interior
node present in all of those trees but not in S1." The API server will need to
add that endpoint.

That endpoint would get rid of the network round trip penalty for fetching
merkle tree nodes. However, it's likely that the keybase.io API server will
still pay a MySQL/Aurora round trip penalty for each node in assembling the
response. I haven't measured Keybase's MySQL round trip time, but I expect it
to be between 1ms and 10ms. (Max: Do you know our median response time?) Even
if a MySQL fetch takes only 1ms per merkle tree node, we would probably burn up
our entire 5ms per-root-node time budget just fetching the interior nodes.

So while the batch merkle tree fetching endpoint is an important part of
reducing overhead in the steady state, it's likely that it won't be fast enough
to hit our performance target for boostrapping. Instead, Keybase will need to
publish large dump files containing all the merkle nodes in the world, and the
validator will need to download and process these files as part of
bootstrapping.

## Dump Files

As noted immediately above, bulk dump files will probably be the most important
step in improving bootstrapping performance. At a minimum, these will need to
supply all the merkle tree interior nodes in the world. Dumping the interior
nodes en masse will allow MySQL to execute a giant table scan, avoiding the
round trip penalties that the Keybase API server pays for tree traversals.

The dump file might also include root nodes, since I'm not sure a bulk root
node fetching endpoint exists now, but alternatively such an endpoint could be
added. Likewise the dump file could include user keys and sigs, but the
existing API endpoints for fetching those in bulk are probably adequate.

The dump file could be regenerated periodically, say once a week. If generation
puts an unacceptable load on the production DB, it could instead be generated
from a replica. Only the most recent dump file needs to be kept around on the
server, so storage requirements don't need to grow quadratically over time.
Alternatively, the server could generate e.g. one dump file per day, including
only the interior nodes generated that day, and the client could do a separate
fetch for each day. (However, I don't think interior nodes record what day they
were created on, so executing daily dumps without expensive tree traversals
might require a new index on the server side.)

Ideally neither the database, nor the API server, nor the validator will need
to keep the entire world of merkle iterior nodes in memory at any point.
Formatting the whole thing as e.g. a giant JSON list of strings would make that
trickier than it needs to be, because iterating over a JSON list in a streaming
fashion is weird. A stream of concatenated JSON objects, or MessagePack
objects, or something like that would probably be easier to work with. But the
details aren't super important here.

## Properties to Validate

This is an initial list off the top of my head, and we'll need to add to it as
we think of other properties.

Tree-specific:
- Each merkle root is validly signed.
- Merkle root seqnos are a contiguous series, with no duplicates.
- Each root contains the appropriate prev pointer and skip pointers, each with
  the right hash.

User-specific:
- No merkle root ever moves a user seqno backwards.
- User seqnos are a contiguous series, with no duplicates.
- Each sigchain link contins the appropriate prev pointer, with the right
  hash.
- Each sigchain link is signed by a key that was active at the time (respecting
  the PGP `full_hash` field when present).
- No two users own the same KID ("key pinning").
- The mapping of usernames to UIDs is unique. (We could enforce the fact that
  most UIDs are the hash of a username, or that might not be necessary.)

My plan is to get an MVP working for those properties, and then to flesh out
the properties for teams afterwards, since I understand those less well. (Max:
Thoughts about that?)

## Local Storage

The validator will store everything it caches in a SQL database. Development
will use a SQLite file, and it's expected that most production deployments will
too. However, the implementation will try to make it relatively easy to add
MySQL/Postgres support later.

Note that one of the implementation details we'll need to track carefully for
SQLite is when to run [`ANALYZE`](https://sqlite.org/lang_analyze.html). SQLite
doesn't automatically update its own query planning metadata, and the caller is
responsible for triggering analytics periodically. We'll certainly need to run
that after the initial bootstrapping loop, and we might want to run it after
any bulk insert (like if the validator went offline for a few days). I'll need
to experiment with this.

## Adding New Checks in Future Versions

The database will record what version of the validator created it. When the
validator starts up, and sees that its database was created by an older
version, it may decide to drop data.

For example, it might be that version 1 validated PGP signatures loosely,
without respecting the `full_hash` field. Version 2 understands that field, and
it knows that all user sigchains need to be re-validated. One option would be
to walk through all the currently known users and specifically re-check their
PGP signatures, without dropping any data. However, this strategy could miss a
tricky interaction with teams, where we also need to check that any team
signature made with a PGP key used the right version, relative to what that
user had active at the time. (If I recall, PGP keys actually aren't allowed to
make team sigs, but I'm running with this just as an example.) Normally the
root loop manages a consistent picture of the entire world at a given time, but
here the version upgrade code would be looking back in time and trying to
recreate that picture. This sort of version-jump-specific code is complicated
to get right, duplicative of the logic in the root loop, and unlikely to be
tested thoroughly or maintained carefully.

Instead, I expect that the implementation will tend to drop much or all of the
derived data from the previous version, and to repeat the root loop in its
entirety from seqno 1. This is potentially much more expensive, but importantly
it means that there's only one codepath for validating things. As noted above,
caching can substantially reduce the cost of doing this.

Installations that can't tolerate validator downtime (for example, if we let
people put the validator in the hot path of their client requests) might need a
workflow that involves spinning up a new instance in parallel with the old one,
waiting for the new instance to bootstrap, and then swapping the two instances
at the DNS level. This would probably be the case even if we tried to retain as
much data across versions as possible, because e.g. re-verifying large parts of
the world is always going to take a long time, even if it doesn't take 8 hours.

## Dealing with PGP

A pure-Rust implementation of PGP is available at https://sequoia-pgp.org. With
any luck, the vast majority of PGP signatures in the Keybase world will be
checkable using Sequoia. The benchmarks for verifying PGP signatures above are
based on this library, and as a proof of concept it works for validating the
merkle tree root.

For keys or signatures with wacky GnuPG-specific crap in them that fail
validation in-process, we can shell out to GnuPG to validate them in a
temporary keyring (or potentially to Keybase Go code running elsewhere). That's
going to be much more expensive, but if the keys/sigs in question are a tiny
minority, it might be much simpler to shell out than to try to support every
corner case. A likely strategy here will be to attempt validation with Sequoia,
and when that fails, to automatically make a fallback attempt with GnuPG.

As noted above, we will probably want to cache signatures in two forms:

- The "raw" tables mentioned above, which include signature in the attached
  form indexed by seqno.
- Some "verified" tables which store the extracted plaintext of each signature,
  representing the fact that the signature was checked already.

The verified tables will represent derived data, but that data is unlikely to
ever be dropped, because signature verification is very stable. This will
roughly double the overall storage requirements of the validator, however,
compared to storing only the raw sigs.

One wrinkle for user PGP sigs: The state of the user's sigchain determines what
`full_hash` version of the key should be used to check a signature. If sigchain
links are always ingested in order, we could try to keep track of full hashes,
but there's a risk that different versions of the validator might disagree
about which version of the full hash applied. One workaround here could be to
store the `full_hash` that was used to validate a signature as a column in the
verified table, and to repeat the validation later if it ever looks like that
`full_hash` that was used in the past doesn't match what's expected.

## Parallelism (or lack thereof)

The initial implementation of the validator will be single-threaded, and
runtime is going to be dominated by IO in many cases. However, there are
exceptions:

- Re-validating the world after a version update / developer change might not
  need to do any network IO.
- If each merkle root seqno requires 100 KB of data (hopefully an
  overestimate), a 1 GB/s connection e.g. inside of AWS could download the
  entire world in a dump file in about 10 minutes.

In those case, the bootstrapping time will be dominated by either disk or CPU.
It might then make sense to try to parallelize across threads, to spread out
the CPU work of verifying signatures. If we do this, it'll be important to
avoid having multiple threads writing to disk at once; SQLite has a global
write lock, and having multiple threads contend for it will just slow things
down.

The central challenge in parallelizing any of this is that walking time forward
is inherently serial. For example, if merkle root number 1 modifies user A, and
root number 2 also touches A, the processing of root 2 must take into account
the result of root 1. (Did it add a sibkey? Remove one? Change the `full_hash`
of a PGP key? Etc.) Furthermore, if root 3 modifies A too, root 2 must *not*
see the result of root 3. The validator only maintains one answer for the
"current state of user A"; it's not going to bookkeep a separate snapshot of A
associated with each merkle root. So any attempt to parallelize verification
must preserve the serial nature of all this.

There are fancy strategies that can work around this. We could try to detect
when roots are independent from each other, and parallelize them only in that
case. We could even take a page out of Intel's book and try to detect
serialization violations after the fact, assuming they'll be rare and rewinding
processing to correct them when they come up. But all of these strategies are
complicated, and I don't plan to attempt any of them until I have a working
implementation of the validator running under a profiler.

## Blockchain

Publication to the blockchain happens only once every 12 hours, so it can only
be a lagging indicator of inconsistency in the tree. But the validator could
periodically check what's in the blockchain and kick off a root loop run (or
record a conflict) based on what it finds.

Rather than taking on the complexity of a local bitcoin node, it probably makes
the most sense to query a handful of blockchain explorers and just cross-check
their results. Also this doesn't necessarily need to be part of the MVP.

## Rough Work Schedule

- Week 1
  - root loop, begin walking roots from seqno 1
  - merkle tree traversals, finding differing nodes between two roots
  - user loop, begin detecting new users and new sigchain links
  - a test framework for generating fake Merkle histories
- Week 2
  - GnuPG integration for wacky signatures
  - `full_hash` handling
  - merkle skip pointers
  - UID and KID uniqueness
- Week 3
  - failure test cases
  - dump file format
  - keybase.io API endpoint for bulk root fetching
  - profiling
- Week 4
  - more profiling and optimization
  - team sigchain integration
