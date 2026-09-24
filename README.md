# xmip-core-archive-file

File archive target: one item is one file under a directory, its metadata
beside it. A technology of
[xmip-core-archive](https://github.com/IlleNilsson/xmip-core-archive).

The metadata file is TOML: every key and value is a basic string quoted and
read back through `xmip-core-library-codec`, every control character
escaped, so a key may hold `=` or a quote.

## Toolchain

`rust-toolchain.toml` pins the toolchain for the whole estate. Do not change it
here.

## Verification

The included workflow is manual-only and calls the versioned shared workflow at
`IlleNilsson/.github@v1`.
