# Keybase Validator

## A Sketch of Operation in the Steady State

Here's a rough description of what the validator does in its steady state,
focusing just on the Merkle roots and on the users tree. The word "fetch" below
could be interpreted to mean an individual network fetch, however in the
performance section below we'll discuss how we expect to use batch fetching and
caching to hit our performance target.

- Once a minute, the main loop hits `merkle/root.json` and fetches the latest
  merkle root seqno.
- The **root loop**. For each seqno between the last one verified and the new
  one:
  - Fetch the root node at this seqno.
  - Verify that it's signed correctly, contains the correct seqno, contains
    the correct prev pointers, etc.
  - Recusviely fetch and walk the interior merkle tree nodes under this root,
    finding all the user leaves that differ from the previous verified root.
  - The **user loop**. For each user that was updated, for each sigchain seqno
    between the last verified for that user and the new one:
    - Fetch the sigchain link at that seqno.
    - If needed, fetch keys for the user. Note that "needed" is hard to
      determine prior to the introduction of the PGP `full_hash` field in the
      sigchain, but we might be able to assume that all new PGP keys from this
      point forward will include that field, and only re-fetch keys when we
      encounter a new `full_hash`.
    - Verify that it's signed correctly, contains the correct seqno, contains
      the correct prev pointers, etc.

When all of this completes, every new merkle root has been incorporated into
the local tree, and every user update has been verified. If any step fails
verification, the process flips into a global failure state, from which it
doesn't attempt to recover.

Clients query the validator by sending along the last signed merkle root they
saw from keybase.io. If the root they send has a valid signature, and the
validator hasn't yet seen it, this will kick off a run of the root loop to
catch up and wait for it to complete. The validator will also record every
signed merkle root that it encounters, and any inconsistencies will lead to
another permanent failure state, making these requests a sort of gossip
protocol about what each client is seeing.

If the validator sees any errors during the root loop, it fails the client
request that started it (if any) and all subsequent requests. The request may
also fail with a "not yet in the steady state" error if the validator hasn't
yet completed its first bootstrapping runs through root loop.

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
  validator running in the wild.
- Development. Much of development will be repeatedly re-running the validator
  against production Merkle tree data, and gradually encountering all the
  corner cases that have come up over time. Avoiding network fetches during
  this process will be helpful.

The following items will be cached in raw tables, which should never need to be
modified:

- Merkle roots, indexed by their seqno.
- Merkle interior nodes, indexed by their hash.
- User sigchain links, indexed by their UID and seqno.
- User PGP keys, indexed by their derived KID and `full_hash`.

Other derived data will be cached in separate tables, with the expectation that
these tables can be blown away during a version upgrade if need be. For
example:

- The "current" Merkle seqno. (With respect to either the current steady state
  of the validator, or the intermediate state of a root loop in progress.)
- The current seqno for each user sigchain.
- The set of active sibkeys (and their PGP full hashes) for each user.
- The set of all pinned KIDs, and the users that have pinned them.

## Bootstrapping and Performance

I've tested PGP and Keybase-flavored-NACL signature verification in Rust and
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

Loading a PGP key (the merkle root signing key) and checking a signature take
1ms combined. For comparison, `curl https://keyase.io/robots.txt` takes about
100ms.

This lets us make some important back-of-the-envelope estimates about
performance requirements. The current merkle seqno as of this writing (14 May
2019) is 5,370,352, with a rate of new roots of about 1 every 10 seconds. If
validating every one of those roots requires a single 100ms network fetch and
nothing else, bootstrapping will take 6 days. If validating every one of those
roots requires verifying a single PGP signature and nothing else, bootstrapping
will take 2 hours. (However note that with some effort, signature verification
could be parallelize across many cores, if that ends up being the bottleneck.)

Choosing a performance target is somewhat arbitrary, but my preference is for
bootstrapping to finish "overnight", or in at most 8 hours. That works out to
about 5ms per merkle seqno. It will be impossible to hit that target without
doing network batching to reduce round trips.

Some of that can be done today: Fetching a user's entire set of keys can be
done all at once. Likewise fetching a user's entire set of sigchain nodes.
Doing those operations the first time a user is encountered will substantially
reduce roundtrips.

However, I don't think there's currently an appropriate endpoint for batching
fetches of the entire merkle tree. The behavior would need to be of the form:
"Given a last seen merkle seqno S1, and a requested seqno S2, send me every
merkle root S1 < S <= S2, along with every interior node present in all of
those trees but not in S1."

An endpoint like that would get rid of the network round trip penalty for
fetching merkle tree nodes, if the validator fetched them in batches of 10 or
100. However, it's likely that the keybase.io API server will still pay a
MySQL/Aurora round trip penalty for each node in assembling the response. I
haven't measured Keybase's MySQL round trip time, but I expect it to be between
1ms and 10ms. (Max: Do you know our median response time?) Even if a MySQL
fetch takes only 1ms per merkle tree node, we would probably burn up our entire
5ms per-root-node time budget just fetching the interior nodes.

So while that batch endpoint is probably an important step to reduce overhead
in the steady state, it's likely that it won't be fast enough to hit our
performance target for boostrapping. Instead, Keybase will need to publish
large dump files containing all the merkle nodes in the world, and the
validator will need to download and process these files as part of
bootstrapping. More discussion of this design below.

### Bootstrapping

Bootstrapping a validator up from an empty state will be dominated by network
overhead, but there are several ways that overhead could be reduced:

- The keybase.io API could provide a streaming endpoint for reads. That would
  reduce the round trips associated with each fetch. Alternatively, we could
  support HTTP2 and let that reduce the overhead for us.
- Keybase could export periodic dump files containing all the raw objects that
  the validator needs. Those could be hosted in e.g. a large Amazon S3 bucket.

The former would probably end up putting a lot of strain on the API server
itself, so my preference would be for the latter. A new validator could
automatically download one of these dump files during startup. The dump files
could also be designed in a streaming or chunked fashion, so that processing
can happen in parallel with fetching, and so that an interrupted bootstrapping
could resume where it left off.

The exact layout of dump files will be easier to design when we have a working
validator and we can profile the hotspots in the bootstrapping process. We'll also have a better estimate of whether 

- naive loop
- list of exact things to be verified
- bootstrapping optimizations
- updates (throwing away some caches, feature flags)
- blockchain integration
- team tree verification
- parallelism optimizations
- in-the-hot-path queries

## Storage

The validator will store everything in a SQL database. Development will use a
SQLite file, and it's expected that most production deployments will too.
However, the implementation will try to make it relatively easy to add
MySQL/Postgres support later.

## Isolated objects and their properties

These items can be ingested from dump files or over a firehose connection to
bootstrap the local database. In general they're all identified by their hash.

- merkle tree roots, nodes, and leaves
- sigchain links (user, team)
- PGP key bodies

Everything that can be validated about an object in isolation is checked during
ingestion. For example:

- objects parse successfully and have the required fields
- PGP keys have the KID that they're supposed to
- merkle tree roots are signed by a Keybase root key
- merkle tree roots contain the skip pointers that they're supposed to
- sigchain links are signed by the key they claim

Once an object is stored in the local DB, these properties are assumed to be
verified. This is potentially important for perfomance, for example if
signature verification occasionally requires shelling out to GPG or doing some
kind of RPC. Most operations in the steady state should be incremental,
operating on only a small number of objects, such that object loading doesn't
necessarily need to be optimized. However, it will occasionally be necessary to
re-verify large parts of the world, for example when an new version of the
validator adds a new rule. Avoiding shelling out thousands of times in that
case could be important.

## Global consistency

Unlike the properties above, which can be checked looking at a single object
(or maybe a pair of objects) in isolation, there are other properties that we
can only validate by looking at the whole world:

- the merkle root sequence numbers are contiguous
- every merkle prev pointer has the correct hash
- no merkle root rewinds or forks any user or team
- user and team sigchains sequence numbers are contiguous
- each signing key in a user or team's sigchain was valid at the time it was
  used

## Adding new checks over time

A "finshed checks" table will store a list of properties that have been
validated about either individual objects or about the entire world. During
boot, the validator loads all the entries in that table, and performs any
checks that it knows about that are missing from the list. The goal is that
future versions of the validator can add a new property (e.g. "prev sig ID and
prev sig payload hash hash must match"), and after updating a given
installation, the new version of the validator will perform the new check on
all its existing cached data, and record success in the DB (or switch into the
"world is on fire" state). Checking a property across the entire world is
expensive, and it's important that the validator doesn't pay those costs during
every boot.

## Dealing with PGP

A pure-Rust implementation of PGP is available at https://sequoia-pgp.org. With
any luck, the vast majority of PGP signatures in the Keybase world will be
checkable using that library, with decent performance.

For keys or signatures with wacky GnuPG-specific crap in them that fail
validation in-process, we can shell out to GnuPG itself to validate them in a
temporary keyring. That's going to be much more expensive, but if the keys/sigs
in question are a tiny minority, it will be much simpler to shell out than
trying to implement our own support.
