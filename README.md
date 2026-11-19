# xmip-core-archive-file

File archive target: one item is one file under a directory, its metadata
beside it. A technology of
[xmip-core-archive](https://github.com/IlleNilsson/xmip-core-archive).

One item is `<root>/<data_type>/<identifier>`, the bytes as they are, and
`<root>/.meta/<data_type>/<identifier>`, its metadata, each name made a
safe file name. An archived item is never written over (ADR-0040,
amendment 2026-11-19), by the capability's rule for every name-keyed
archive (`archive::named`): the metadata is created first, as the claim,
then the bytes, and the receipt is given only after both. Each file is
written beside its name, synced, hard-linked to the name only where the
name is free, and its directory synced (`archive::whole`), so a write that
fails leaves nothing at the name. An identifier archived again with the
same bytes answers the same receipt; with other bytes it is refused, and
so is an identifier that becomes the same file name as another's. A
restore refuses a name without metadata, and bytes whose checksum is not
the metadata's and the receipt's.

The metadata file is the capability's TOML sidecar (`archive::sidecar`):
every key and value is a basic string quoted and read back through
`xmip-core-library-codec`, every control character escaped, so a key may
hold `=` or a quote, and it records the SHA-256 of the bytes.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
