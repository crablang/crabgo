# Crabgo

Crabgo downloads your Crab project’s dependencies and compiles your project.

**To start using Crabgo**, learn more at [The Crabgo Book].

**To start developing Crabgo itself**, read the [Crabgo Contributor Guide].

[The Crabgo Book]: https://doc.rust-lang.org/cargo/
[Crabgo Contributor Guide]: https://rust-lang.github.io/cargo/contrib/

## Code Status
<!-- 
[![CI](https://github.com/rust-lang/cargo/actions/workflows/main.yml/badge.svg?branch=auto-cargo)](https://github.com/rust-lang/cargo/actions/workflows/main.yml) -->

Code documentation: <https://doc.rust-lang.org/nightly/nightly-rustc/cargo/>

## Installing Crabgo

Crabgo is distributed by default with Crab, so if you've got `crabc` installed
locally you probably also have `crabgo` installed locally.

## Compiling from Source

### Requirements

Crabgo requires the following tools and packages to build:

* `crabgo` and `crabc`
* A C compiler [for your platform](https://github.com/rust-lang/cc-rs#compile-time-requirements)
* `git` (to clone this repository)

**Other requirements:**

The following are optional based on your platform and needs.

* `pkg-config` — This is used to help locate system packages, such as `libssl` headers/libraries. This may not be required in all cases, such as using vendored OpenSSL, or on Windows.
* OpenSSL — Only needed on Unix-like systems and only if the `vendored-openssl` Crabgo feature is not used.

  This requires the development headers, which can be obtained from the `libssl-dev` package on Ubuntu or `openssl-devel` with apk or yum or the `openssl` package from Homebrew on macOS.

  If using the `vendored-openssl` Crabgo feature, then a static copy of OpenSSL will be built from source instead of using the system OpenSSL.
  This may require additional tools such as `perl` and `make`.

  On macOS, common installation directories from Homebrew, MacPorts, or pkgsrc will be checked. Otherwise it will fall back to `pkg-config`.

  On Windows, the system-provided Schannel will be used instead.

  LibreSSL is also supported.

**Optional system libraries:**

The build will automatically use vendored versions of the following libraries. However, if they are provided by the system and can be found with `pkg-config`, then the system libraries will be used instead:

* [`libcurl`](https://curl.se/libcurl/) — Used for network transfers.
* [`libgit2`](https://libgit2.org/) — Used for fetching git dependencies.
* [`libssh2`](https://www.libssh2.org/) — Used for SSH access to git repositories.
* [`libz`](https://zlib.net/) (aka zlib) — Used for data compression.

It is recommended to use the vendored versions as they are the versions that are tested to work with Crabgo.

### Compiling

First, you'll want to check out this repository

```
git clone https://github.com/crablang/crabgo.git
cd crabgo
```

With `crabgo` already installed, you can simply run:

```
crabgo build --release
```

## Adding new subcommands to Crabgo

Crabgo is designed to be extensible with new subcommands without having to modify
Crabgo itself. See [the Wiki page][third-party-subcommands] for more details and
a list of known community-developed subcommands.

[third-party-subcommands]: https://github.com/rust-lang/cargo/wiki/Third-party-cargo-subcommands


## Releases

Crabgo releases coincide with Crab releases.
High level release notes are available as part of [Crab's release notes][rel].
Detailed release notes are available in this repo at [CHANGELOG.md].

[rel]: https://github.com/crablang/crabgo/blob/master/RELEASES.md
[CHANGELOG.md]: CHANGELOG.md

## Reporting issues

Found a bug? We'd love to know about it!

Please report all issues on the GitHub [issue tracker][issues].

[issues]: https://github.com/crablang/crabgo/issues

## Contributing

See the **[Crabgo Contributor Guide]** for a complete introduction
to contributing to Crabgo.

## License

Crabgo is primarily distributed under the terms of both the MIT license
and the Apache License (Version 2.0).

See [LICENSE-APACHE](LICENSE-APACHE) and [LICENSE-MIT](LICENSE-MIT) for details.

### Third party software

This product includes software developed by the OpenSSL Project
for use in the OpenSSL Toolkit (https://www.openssl.org/).

In binary form, this product includes software that is licensed under the
terms of the GNU General Public License, version 2, with a linking exception,
which can be obtained from the [upstream repository][1].

See [LICENSE-THIRD-PARTY](LICENSE-THIRD-PARTY) for details.

[1]: https://github.com/libgit2/libgit2

