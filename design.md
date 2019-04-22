# Keybase Validator

## Storage

The validator will store everything in a SQL database. Development will use a
SQLite file, and it's expected that most production deployments will too.
However, the design will try to make it relatively easy to add MySQL/Postgres
support later.

## Unstructured data

These items can be ingested from dump files or over a firehose to bootstrap the
local database.

- merkle tree nodes
- sigchain links (user, team)
- PGP key bodies

Everything that can be validated about an object in isolation is checked during
ingestion. For example:

- objects parse successfully and have the required fields
- PGP keys have the KID that they're supposed to
- merkle tree roots are signed by a Keybase root key
- merkle tree roots contain the skip pointers that they're supposed to
- sigchain links are signed by the key they say they're signed by

## Structured data

Unlike the properties above, which can be checked looking at a single object
(or maybe a pair of objects) in isolation, there are other properties that we
can only validate by looking at the whole world:

- the merkle root sequence is contiguous, with no duplicates
- every merkle prev pointer agrees about the hash of the root at a given seqno
- no merkle root rewinds or forks any user or team
- user and team sigchains are contiguous
- each signing key in a user or team's sigchain was valid at the time it was used

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
