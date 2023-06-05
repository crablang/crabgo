//! A `Source` for registry-based packages.
//!
//! # What's a Registry?
//!
//! Registries are central locations where packages can be uploaded to,
//! discovered, and searched for. The purpose of a registry is to have a
//! location that serves as permanent storage for versions of a crate over time.
//!
//! Compared to git sources, a registry provides many packages as well as many
//! versions simultaneously. Git sources can also have commits deleted through
//! rebasings where registries cannot have their versions deleted.
//!
//! # The Index of a Registry
//!
//! One of the major difficulties with a registry is that hosting so many
//! packages may quickly run into performance problems when dealing with
//! dependency graphs. It's infeasible for cargo to download the entire contents
//! of the registry just to resolve one package's dependencies, for example. As
//! a result, cargo needs some efficient method of querying what packages are
//! available on a registry, what versions are available, and what the
//! dependencies for each version is.
//!
//! One method of doing so would be having the registry expose an HTTP endpoint
//! which can be queried with a list of packages and a response of their
//! dependencies and versions is returned. This is somewhat inefficient however
//! as we may have to hit the endpoint many times and we may have already
//! queried for much of the data locally already (for other packages, for
//! example). This also involves inventing a transport format between the
//! registry and Cargo itself, so this route was not taken.
//!
//! Instead, Cargo communicates with registries through a git repository
//! referred to as the Index. The Index of a registry is essentially an easily
//! query-able version of the registry's database for a list of versions of a
//! package as well as a list of dependencies for each version.
//!
//! Using git to host this index provides a number of benefits:
//!
//! * The entire index can be stored efficiently locally on disk. This means
//!   that all queries of a registry can happen locally and don't need to touch
//!   the network.
//!
//! * Updates of the index are quite efficient. Using git buys incremental
//!   updates, compressed transmission, etc for free. The index must be updated
//!   each time we need fresh information from a registry, but this is one
//!   update of a git repository that probably hasn't changed a whole lot so
//!   it shouldn't be too expensive.
//!
//!   Additionally, each modification to the index is just appending a line at
//!   the end of a file (the exact format is described later). This means that
//!   the commits for an index are quite small and easily applied/compressible.
//!
//! ## The format of the Index
//!
//! The index is a store for the list of versions for all packages known, so its
//! format on disk is optimized slightly to ensure that `ls registry` doesn't
//! produce a list of all packages ever known. The index also wants to ensure
//! that there's not a million files which may actually end up hitting
//! filesystem limits at some point. To this end, a few decisions were made
//! about the format of the registry:
//!
//! 1. Each crate will have one file corresponding to it. Each version for a
//!    crate will just be a line in this file.
//! 2. There will be two tiers of directories for crate names, under which
//!    crates corresponding to those tiers will be located.
//!
//! As an example, this is an example hierarchy of an index:
//!
//! ```notrust
//! .
//! ├── 3
//! │   └── u
//! │       └── url
//! ├── bz
//! │   └── ip
//! │       └── bzip2
//! ├── config.json
//! ├── en
//! │   └── co
//! │       └── encoding
//! └── li
//!     ├── bg
//!     │   └── libgit2
//!     └── nk
//!         └── link-config
//! ```
//!
//! The root of the index contains a `config.json` file with a few entries
//! corresponding to the registry (see [`RegistryConfig`] below).
//!
//! Otherwise, there are three numbered directories (1, 2, 3) for crates with
//! names 1, 2, and 3 characters in length. The 1/2 directories simply have the
//! crate files underneath them, while the 3 directory is sharded by the first
//! letter of the crate name.
//!
//! Otherwise the top-level directory contains many two-letter directory names,
//! each of which has many sub-folders with two letters. At the end of all these
//! are the actual crate files themselves.
//!
//! The purpose of this layout is to hopefully cut down on `ls` sizes as well as
//! efficient lookup based on the crate name itself.
//!
//! ## Crate files
//!
//! Each file in the index is the history of one crate over time. Each line in
//! the file corresponds to one version of a crate, stored in JSON format (see
//! the `RegistryPackage` structure below).
//!
//! As new versions are published, new lines are appended to this file. The only
//! modifications to this file that should happen over time are yanks of a
//! particular version.
//!
//! # Downloading Packages
//!
//! The purpose of the Index was to provide an efficient method to resolve the
//! dependency graph for a package. So far we only required one network
//! interaction to update the registry's repository (yay!). After resolution has
//! been performed, however we need to download the contents of packages so we
//! can read the full manifest and build the source code.
//!
//! To accomplish this, this source's `download` method will make an HTTP
//! request per-package requested to download tarballs into a local cache. These
//! tarballs will then be unpacked into a destination folder.
//!
//! Note that because versions uploaded to the registry are frozen forever that
//! the HTTP download and unpacking can all be skipped if the version has
//! already been downloaded and unpacked. This caching allows us to only
//! download a package when absolutely necessary.
//!
//! # Filesystem Hierarchy
//!
//! Overall, the `$HOME/.cargo` looks like this when talking about the registry:
//!
//! ```notrust
//! # A folder under which all registry metadata is hosted (similar to
//! # $HOME/.cargo/git)
//! $HOME/.cargo/registry/
//!
//!     # For each registry that cargo knows about (keyed by hostname + hash)
//!     # there is a folder which is the checked out version of the index for
//!     # the registry in this location. Note that this is done so cargo can
//!     # support multiple registries simultaneously
//!     index/
//!         registry1-<hash>/
//!         registry2-<hash>/
//!         ...
//!
//!     # This folder is a cache for all downloaded tarballs from a registry.
//!     # Once downloaded and verified, a tarball never changes.
//!     cache/
//!         registry1-<hash>/<pkg>-<version>.crate
//!         ...
//!
//!     # Location in which all tarballs are unpacked. Each tarball is known to
//!     # be frozen after downloading, so transitively this folder is also
//!     # frozen once its unpacked (it's never unpacked again)
//!     src/
//!         registry1-<hash>/<pkg>-<version>/...
//!         ...
//! ```

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::task::{ready, Poll};

use anyhow::Context as _;
use cargo_util::paths::{self, exclude_from_backups_and_indexing};
use flate2::read::GzDecoder;
use log::debug;
use semver::Version;
use serde::Deserialize;
use tar::Archive;

use crate::core::dependency::{DepKind, Dependency};
use crate::core::source::MaybePackage;
use crate::core::{Package, PackageId, QueryKind, Source, SourceId, Summary};
use crate::sources::PathSource;
use crate::util::hex;
use crate::util::interning::InternedString;
use crate::util::into_url::IntoUrl;
use crate::util::network::PollExt;
use crate::util::{
    restricted_names, CargoResult, Config, Filesystem, LimitErrorReader, OptVersionReq,
};

const PACKAGE_SOURCE_LOCK: &str = ".cargo-ok";
pub const CRATES_IO_INDEX: &str = "https://github.com/rust-lang/crates.io-index";
pub const CRATES_IO_HTTP_INDEX: &str = "sparse+https://index.crates.io/";
pub const CRATES_IO_REGISTRY: &str = "crates-io";
pub const CRATES_IO_DOMAIN: &str = "crates.io";
const CRATE_TEMPLATE: &str = "{crate}";
const VERSION_TEMPLATE: &str = "{version}";
const PREFIX_TEMPLATE: &str = "{prefix}";
const LOWER_PREFIX_TEMPLATE: &str = "{lowerprefix}";
const CHECKSUM_TEMPLATE: &str = "{sha256-checksum}";
const MAX_UNPACK_SIZE: u64 = 512 * 1024 * 1024;
const MAX_COMPRESSION_RATIO: usize = 20; // 20:1

/// A "source" for a local (see `local::LocalRegistry`) or remote (see
/// `remote::RemoteRegistry`) registry.
///
/// This contains common functionality that is shared between the two registry
/// kinds, with the registry-specific logic implemented as part of the
/// [`RegistryData`] trait referenced via the `ops` field.
pub struct RegistrySource<'cfg> {
    source_id: SourceId,
    /// The path where crate files are extracted (`$CARGO_HOME/registry/src/$REG-HASH`).
    src_path: Filesystem,
    /// Local reference to [`Config`] for convenience.
    config: &'cfg Config,
    /// Abstraction for interfacing to the different registry kinds.
    ops: Box<dyn RegistryData + 'cfg>,
    /// Interface for managing the on-disk index.
    index: index::RegistryIndex<'cfg>,
    /// A set of packages that should be allowed to be used, even if they are
    /// yanked.
    ///
    /// This is populated from the entries in `Cargo.lock` to ensure that
    /// `cargo update -p somepkg` won't unlock yanked entries in `Cargo.lock`.
    /// Otherwise, the resolver would think that those entries no longer
    /// exist, and it would trigger updates to unrelated packages.
    yanked_whitelist: HashSet<PackageId>,
}

/// The `config.json` file stored in the index.
#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "kebab-case")]
pub struct RegistryConfig {
    /// Download endpoint for all crates.
    ///
    /// The string is a template which will generate the download URL for the
    /// tarball of a specific version of a crate. The substrings `{crate}` and
    /// `{version}` will be replaced with the crate's name and version
    /// respectively.  The substring `{prefix}` will be replaced with the
    /// crate's prefix directory name, and the substring `{lowerprefix}` will
    /// be replaced with the crate's prefix directory name converted to
    /// lowercase. The substring `{sha256-checksum}` will be replaced with the
    /// crate's sha256 checksum.
    ///
    /// For backwards compatibility, if the string does not contain any
    /// markers (`{crate}`, `{version}`, `{prefix}`, or `{lowerprefix}`), it
    /// will be extended with `/{crate}/{version}/download` to
    /// support registries like crates.io which were created before the
    /// templating setup was created.
    pub dl: String,

    /// API endpoint for the registry. This is what's actually hit to perform
    /// operations like yanks, owner modifications, publish new crates, etc.
    /// If this is None, the registry does not support API commands.
    pub api: Option<String>,

    /// Whether all operations require authentication.
    #[serde(default)]
    pub auth_required: bool,
}

/// The maximum version of the `v` field in the index this version of cargo
/// understands.
pub(crate) const INDEX_V_MAX: u32 = 2;

/// A single line in the index representing a single version of a package.
#[derive(Deserialize)]
pub struct RegistryPackage<'a> {
    name: InternedString,
    vers: Version,
    #[serde(borrow)]
    deps: Vec<RegistryDependency<'a>>,
    features: BTreeMap<InternedString, Vec<InternedString>>,
    /// This field contains features with new, extended syntax. Specifically,
    /// namespaced features (`dep:`) and weak dependencies (`pkg?/feat`).
    ///
    /// This is separated from `features` because versions older than 1.19
    /// will fail to load due to not being able to parse the new syntax, even
    /// with a `Cargo.lock` file.
    features2: Option<BTreeMap<InternedString, Vec<InternedString>>>,
    cksum: String,
    /// If `true`, Cargo will skip this version when resolving.
    ///
    /// This was added in 2014. Everything in the crates.io index has this set
    /// now, so this probably doesn't need to be an option anymore.
    yanked: Option<bool>,
    /// Native library name this package links to.
    ///
    /// Added early 2018 (see <https://github.com/rust-lang/cargo/pull/4978>),
    /// can be `None` if published before then.
    links: Option<InternedString>,
    /// Required version of rust
    ///
    /// Corresponds to `package.rust-version`.
    ///
    /// Added in 2023 (see <https://github.com/rust-lang/crates.io/pull/6267>),
    /// can be `None` if published before then or if not set in the manifest.
    rust_version: Option<InternedString>,
    /// The schema version for this entry.
    ///
    /// If this is None, it defaults to version 1. Entries with unknown
    /// versions are ignored.
    ///
    /// Version `2` format adds the `features2` field.
    ///
    /// This provides a method to safely introduce changes to index entries
    /// and allow older versions of cargo to ignore newer entries it doesn't
    /// understand. This is honored as of 1.51, so unfortunately older
    /// versions will ignore it, and potentially misinterpret version 2 and
    /// newer entries.
    ///
    /// The intent is that versions older than 1.51 will work with a
    /// pre-existing `Cargo.lock`, but they may not correctly process `cargo
    /// update` or build a lock from scratch. In that case, cargo may
    /// incorrectly select a new package that uses a new index format. A
    /// workaround is to downgrade any packages that are incompatible with the
    /// `--precise` flag of `cargo update`.
    v: Option<u32>,
}

#[test]
fn escaped_char_in_json() {
    let _: RegistryPackage<'_> = serde_json::from_str(
        r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"bae3","features":{}}"#,
    )
    .unwrap();
    let _: RegistryPackage<'_> = serde_json::from_str(
        r#"{"name":"a","vers":"0.0.1","deps":[],"cksum":"bae3","features":{"test":["k","q"]},"links":"a-sys"}"#
    ).unwrap();

    // Now we add escaped cher all the places they can go
    // these are not valid, but it should error later than json parsing
    let _: RegistryPackage<'_> = serde_json::from_str(
        r#"{
        "name":"This name has a escaped cher in it \n\t\" ",
        "vers":"0.0.1",
        "deps":[{
            "name": " \n\t\" ",
            "req": " \n\t\" ",
            "features": [" \n\t\" "],
            "optional": true,
            "default_features": true,
            "target": " \n\t\" ",
            "kind": " \n\t\" ",
            "registry": " \n\t\" "
        }],
        "cksum":"bae3",
        "features":{"test \n\t\" ":["k \n\t\" ","q \n\t\" "]},
        "links":" \n\t\" "}"#,
    )
    .unwrap();
}

/// A dependency as encoded in the index JSON.
#[derive(Deserialize)]
struct RegistryDependency<'a> {
    name: InternedString,
    #[serde(borrow)]
    req: Cow<'a, str>,
    features: Vec<InternedString>,
    optional: bool,
    default_features: bool,
    target: Option<Cow<'a, str>>,
    kind: Option<Cow<'a, str>>,
    registry: Option<Cow<'a, str>>,
    package: Option<InternedString>,
    public: Option<bool>,
}

impl<'a> RegistryDependency<'a> {
    /// Converts an encoded dependency in the registry to a cargo dependency
    pub fn into_dep(self, default: SourceId) -> CargoResult<Dependency> {
        let RegistryDependency {
            name,
            req,
            mut features,
            optional,
            default_features,
            target,
            kind,
            registry,
            package,
            public,
        } = self;

        let id = if let Some(registry) = &registry {
            SourceId::for_registry(&registry.into_url()?)?
        } else {
            default
        };

        let mut dep = Dependency::parse(package.unwrap_or(name), Some(&req), id)?;
        if package.is_some() {
            dep.set_explicit_name_in_toml(name);
        }
        let kind = match kind.as_deref().unwrap_or("") {
            "dev" => DepKind::Development,
            "build" => DepKind::Build,
            _ => DepKind::Normal,
        };

        let platform = match target {
            Some(target) => Some(target.parse()?),
            None => None,
        };

        // All dependencies are private by default
        let public = public.unwrap_or(false);

        // Unfortunately older versions of cargo and/or the registry ended up
        // publishing lots of entries where the features array contained the
        // empty feature, "", inside. This confuses the resolution process much
        // later on and these features aren't actually valid, so filter them all
        // out here.
        features.retain(|s| !s.is_empty());

        // In index, "registry" is null if it is from the same index.
        // In Cargo.toml, "registry" is None if it is from the default
        if !id.is_crates_io() {
            dep.set_registry_id(id);
        }

        dep.set_optional(optional)
            .set_default_features(default_features)
            .set_features(features)
            .set_platform(platform)
            .set_kind(kind)
            .set_public(public);

        Ok(dep)
    }
}

/// Result from loading data from a registry.
pub enum LoadResponse {
    /// The cache is valid. The cached data should be used.
    CacheValid,

    /// The cache is out of date. Returned data should be used.
    Data {
        raw_data: Vec<u8>,
        index_version: Option<String>,
    },

    /// The requested crate was found.
    NotFound,
}

/// An abstract interface to handle both a local (see `local::LocalRegistry`)
/// and remote (see `remote::RemoteRegistry`) registry.
///
/// This allows [`RegistrySource`] to abstractly handle both registry kinds.
pub trait RegistryData {
    /// Performs initialization for the registry.
    ///
    /// This should be safe to call multiple times, the implementation is
    /// expected to not do any work if it is already prepared.
    fn prepare(&self) -> CargoResult<()>;

    /// Returns the path to the index.
    ///
    /// Note that different registries store the index in different formats
    /// (remote=git, local=files).
    fn index_path(&self) -> &Filesystem;

    /// Loads the JSON for a specific named package from the index.
    ///
    /// * `root` is the root path to the index.
    /// * `path` is the relative path to the package to load (like `ca/rg/cargo`).
    /// * `index_version` is the version of the requested crate data currently in cache.
    fn load(
        &mut self,
        root: &Path,
        path: &Path,
        index_version: Option<&str>,
    ) -> Poll<CargoResult<LoadResponse>>;

    /// Loads the `config.json` file and returns it.
    ///
    /// Local registries don't have a config, and return `None`.
    fn config(&mut self) -> Poll<CargoResult<Option<RegistryConfig>>>;

    /// Invalidates locally cached data.
    fn invalidate_cache(&mut self);

    /// If quiet, the source should not display any progress or status messages.
    fn set_quiet(&mut self, quiet: bool);

    /// Is the local cached data up-to-date?
    fn is_updated(&self) -> bool;

    /// Prepare to start downloading a `.crate` file.
    ///
    /// Despite the name, this doesn't actually download anything. If the
    /// `.crate` is already downloaded, then it returns [`MaybeLock::Ready`].
    /// If it hasn't been downloaded, then it returns [`MaybeLock::Download`]
    /// which contains the URL to download. The [`crate::core::package::Downloads`]
    /// system handles the actual download process. After downloading, it
    /// calls [`Self::finish_download`] to save the downloaded file.
    ///
    /// `checksum` is currently only used by local registries to verify the
    /// file contents (because local registries never actually download
    /// anything). Remote registries will validate the checksum in
    /// `finish_download`. For already downloaded `.crate` files, it does not
    /// validate the checksum, assuming the filesystem does not suffer from
    /// corruption or manipulation.
    fn download(&mut self, pkg: PackageId, checksum: &str) -> CargoResult<MaybeLock>;

    /// Finish a download by saving a `.crate` file to disk.
    ///
    /// After [`crate::core::package::Downloads`] has finished a download,
    /// it will call this to save the `.crate` file. This is only relevant
    /// for remote registries. This should validate the checksum and save
    /// the given data to the on-disk cache.
    ///
    /// Returns a [`File`] handle to the `.crate` file, positioned at the start.
    fn finish_download(&mut self, pkg: PackageId, checksum: &str, data: &[u8])
        -> CargoResult<File>;

    /// Returns whether or not the `.crate` file is already downloaded.
    fn is_crate_downloaded(&self, _pkg: PackageId) -> bool {
        true
    }

    /// Validates that the global package cache lock is held.
    ///
    /// Given the [`Filesystem`], this will make sure that the package cache
    /// lock is held. If not, it will panic. See
    /// [`Config::acquire_package_cache_lock`] for acquiring the global lock.
    ///
    /// Returns the [`Path`] to the [`Filesystem`].
    fn assert_index_locked<'a>(&self, path: &'a Filesystem) -> &'a Path;

    /// Block until all outstanding Poll::Pending requests are Poll::Ready.
    fn block_until_ready(&mut self) -> CargoResult<()>;
}

/// The status of [`RegistryData::download`] which indicates if a `.crate`
/// file has already been downloaded, or if not then the URL to download.
pub enum MaybeLock {
    /// The `.crate` file is already downloaded. [`File`] is a handle to the
    /// opened `.crate` file on the filesystem.
    Ready(File),
    /// The `.crate` file is not downloaded, here's the URL to download it from.
    ///
    /// `descriptor` is just a text string to display to the user of what is
    /// being downloaded.
    Download {
        url: String,
        descriptor: String,
        authorization: Option<String>,
    },
}

mod download;
mod http_remote;
mod index;
mod local;
mod remote;

fn short_name(id: SourceId, is_shallow: bool) -> String {
    let hash = hex::short_hash(&id);
    let ident = id.url().host_str().unwrap_or("").to_string();
    let mut name = format!("{}-{}", ident, hash);
    if is_shallow {
        name.push_str("-shallow");
    }
    name
}

impl<'cfg> RegistrySource<'cfg> {
    pub fn remote(
        source_id: SourceId,
        yanked_whitelist: &HashSet<PackageId>,
        config: &'cfg Config,
    ) -> CargoResult<RegistrySource<'cfg>> {
        assert!(source_id.is_remote_registry());
        let name = short_name(
            source_id,
            config
                .cli_unstable()
                .gitoxide
                .map_or(false, |gix| gix.fetch && gix.shallow_index)
                && !source_id.is_sparse(),
        );
        let ops = if source_id.is_sparse() {
            Box::new(http_remote::HttpRegistry::new(source_id, config, &name)?) as Box<_>
        } else {
            Box::new(remote::RemoteRegistry::new(source_id, config, &name)) as Box<_>
        };

        Ok(RegistrySource::new(
            source_id,
            config,
            &name,
            ops,
            yanked_whitelist,
        ))
    }

    pub fn local(
        source_id: SourceId,
        path: &Path,
        yanked_whitelist: &HashSet<PackageId>,
        config: &'cfg Config,
    ) -> RegistrySource<'cfg> {
        let name = short_name(source_id, false);
        let ops = local::LocalRegistry::new(path, config, &name);
        RegistrySource::new(source_id, config, &name, Box::new(ops), yanked_whitelist)
    }

    fn new(
        source_id: SourceId,
        config: &'cfg Config,
        name: &str,
        ops: Box<dyn RegistryData + 'cfg>,
        yanked_whitelist: &HashSet<PackageId>,
    ) -> RegistrySource<'cfg> {
        RegistrySource {
            src_path: config.registry_source_path().join(name),
            config,
            source_id,
            index: index::RegistryIndex::new(source_id, ops.index_path(), config),
            yanked_whitelist: yanked_whitelist.clone(),
            ops,
        }
    }

    /// Decode the configuration stored within the registry.
    ///
    /// This requires that the index has been at least checked out.
    pub fn config(&mut self) -> Poll<CargoResult<Option<RegistryConfig>>> {
        self.ops.config()
    }

    /// Unpacks a downloaded package into a location where it's ready to be
    /// compiled.
    ///
    /// No action is taken if the source looks like it's already unpacked.
    fn unpack_package(&self, pkg: PackageId, tarball: &File) -> CargoResult<PathBuf> {
        // The `.cargo-ok` file is used to track if the source is already
        // unpacked.
        let package_dir = format!("{}-{}", pkg.name(), pkg.version());
        let dst = self.src_path.join(&package_dir);
        let path = dst.join(PACKAGE_SOURCE_LOCK);
        let path = self.config.assert_package_cache_locked(&path);
        let unpack_dir = path.parent().unwrap();
        match path.metadata() {
            Ok(meta) if meta.len() > 0 => return Ok(unpack_dir.to_path_buf()),
            Ok(_meta) => {
                // The `.cargo-ok` file is not in a state we expect it to be
                // (with two bytes containing "ok").
                //
                // Cargo has always included a `.cargo-ok` file to detect if
                // extraction was interrupted, but it was originally empty.
                //
                // In 1.34, Cargo was changed to create the `.cargo-ok` file
                // before it started extraction to implement fine-grained
                // locking. After it was finished extracting, it wrote two
                // bytes to indicate it was complete. It would use the length
                // check to detect if it was possibly interrupted.
                //
                // In 1.36, Cargo changed to not use fine-grained locking, and
                // instead used a global lock. The use of `.cargo-ok` was no
                // longer needed for locking purposes, but was kept to detect
                // when extraction was interrupted.
                //
                // In 1.49, Cargo changed to not create the `.cargo-ok` file
                // before it started extraction to deal with `.crate` files
                // that inexplicably had a `.cargo-ok` file in them.
                //
                // In 1.64, Cargo changed to detect `.crate` files with
                // `.cargo-ok` files in them in response to CVE-2022-36113,
                // which dealt with malicious `.crate` files making
                // `.cargo-ok` a symlink causing cargo to write "ok" to any
                // arbitrary file on the filesystem it has permission to.
                //
                // This is all a long-winded way of explaining the
                // circumstances that might cause a directory to contain a
                // `.cargo-ok` file that is empty or otherwise corrupted.
                // Either this was extracted by a version of Rust before 1.34,
                // in which case everything should be fine. However, an empty
                // file created by versions 1.36 to 1.49 indicates that the
                // extraction was interrupted and that we need to start again.
                //
                // Another possibility is that the filesystem is simply
                // corrupted, in which case deleting the directory might be
                // the safe thing to do. That is probably unlikely, though.
                //
                // To be safe, this deletes the directory and starts over
                // again.
                log::warn!("unexpected length of {path:?}, clearing cache");
                paths::remove_dir_all(dst.as_path_unlocked())?;
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => anyhow::bail!("failed to access package completion {path:?}: {e}"),
        }
        dst.create_dir()?;
        let mut tar = {
            let size_limit = max_unpack_size(self.config, tarball.metadata()?.len());
            let gz = GzDecoder::new(tarball);
            let gz = LimitErrorReader::new(gz, size_limit);
            Archive::new(gz)
        };
        let prefix = unpack_dir.file_name().unwrap();
        let parent = unpack_dir.parent().unwrap();
        for entry in tar.entries()? {
            let mut entry = entry.with_context(|| "failed to iterate over archive")?;
            let entry_path = entry
                .path()
                .with_context(|| "failed to read entry path")?
                .into_owned();

            // We're going to unpack this tarball into the global source
            // directory, but we want to make sure that it doesn't accidentally
            // (or maliciously) overwrite source code from other crates. Cargo
            // itself should never generate a tarball that hits this error, and
            // crates.io should also block uploads with these sorts of tarballs,
            // but be extra sure by adding a check here as well.
            if !entry_path.starts_with(prefix) {
                anyhow::bail!(
                    "invalid tarball downloaded, contains \
                     a file at {:?} which isn't under {:?}",
                    entry_path,
                    prefix
                )
            }
            // Prevent unpacking the lockfile from the crate itself.
            if entry_path
                .file_name()
                .map_or(false, |p| p == PACKAGE_SOURCE_LOCK)
            {
                continue;
            }
            // Unpacking failed
            let mut result = entry.unpack_in(parent).map_err(anyhow::Error::from);
            if cfg!(windows) && restricted_names::is_windows_reserved_path(&entry_path) {
                result = result.with_context(|| {
                    format!(
                        "`{}` appears to contain a reserved Windows path, \
                        it cannot be extracted on Windows",
                        entry_path.display()
                    )
                });
            }
            result
                .with_context(|| format!("failed to unpack entry at `{}`", entry_path.display()))?;
        }

        // Now that we've finished unpacking, create and write to the lock file to indicate that
        // unpacking was successful.
        let mut ok = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("failed to open `{}`", path.display()))?;
        write!(ok, "ok")?;

        Ok(unpack_dir.to_path_buf())
    }

    fn get_pkg(&mut self, package: PackageId, path: &File) -> CargoResult<Package> {
        let path = self
            .unpack_package(package, path)
            .with_context(|| format!("failed to unpack package `{}`", package))?;
        let mut src = PathSource::new(&path, self.source_id, self.config);
        src.update()?;
        let mut pkg = match src.download(package)? {
            MaybePackage::Ready(pkg) => pkg,
            MaybePackage::Download { .. } => unreachable!(),
        };

        // After we've loaded the package configure its summary's `checksum`
        // field with the checksum we know for this `PackageId`.
        let req = OptVersionReq::exact(package.version());
        let summary_with_cksum = self
            .index
            .summaries(&package.name(), &req, &mut *self.ops)?
            .expect("a downloaded dep now pending!?")
            .map(|s| s.summary.clone())
            .next()
            .expect("summary not found");
        if let Some(cksum) = summary_with_cksum.checksum() {
            pkg.manifest_mut()
                .summary_mut()
                .set_checksum(cksum.to_string());
        }

        Ok(pkg)
    }
}

impl<'cfg> Source for RegistrySource<'cfg> {
    fn query(
        &mut self,
        dep: &Dependency,
        kind: QueryKind,
        f: &mut dyn FnMut(Summary),
    ) -> Poll<CargoResult<()>> {
        // If this is a precise dependency, then it came from a lock file and in
        // theory the registry is known to contain this version. If, however, we
        // come back with no summaries, then our registry may need to be
        // updated, so we fall back to performing a lazy update.
        if kind == QueryKind::Exact && dep.source_id().precise().is_some() && !self.ops.is_updated()
        {
            debug!("attempting query without update");
            let mut called = false;
            ready!(self.index.query_inner(
                &dep.package_name(),
                dep.version_req(),
                &mut *self.ops,
                &self.yanked_whitelist,
                &mut |s| {
                    if dep.matches(&s) {
                        called = true;
                        f(s);
                    }
                },
            ))?;
            if called {
                Poll::Ready(Ok(()))
            } else {
                debug!("falling back to an update");
                self.invalidate_cache();
                Poll::Pending
            }
        } else {
            let mut called = false;
            ready!(self.index.query_inner(
                &dep.package_name(),
                dep.version_req(),
                &mut *self.ops,
                &self.yanked_whitelist,
                &mut |s| {
                    let matched = match kind {
                        QueryKind::Exact => dep.matches(&s),
                        QueryKind::Fuzzy => true,
                    };
                    if matched {
                        f(s);
                        called = true;
                    }
                }
            ))?;
            if called {
                return Poll::Ready(Ok(()));
            }
            let mut any_pending = false;
            if kind == QueryKind::Fuzzy {
                // Attempt to handle misspellings by searching for a chain of related
                // names to the original name. The resolver will later
                // reject any candidates that have the wrong name, and with this it'll
                // along the way produce helpful "did you mean?" suggestions.
                // For now we only try the canonical lysing `-` to `_` and vice versa.
                // More advanced fuzzy searching become in the future.
                for name_permutation in [
                    dep.package_name().replace('-', "_"),
                    dep.package_name().replace('_', "-"),
                ] {
                    if name_permutation.as_str() == dep.package_name().as_str() {
                        continue;
                    }
                    any_pending |= self
                        .index
                        .query_inner(
                            &name_permutation,
                            dep.version_req(),
                            &mut *self.ops,
                            &self.yanked_whitelist,
                            f,
                        )?
                        .is_pending();
                }
            }
            if any_pending {
                Poll::Pending
            } else {
                Poll::Ready(Ok(()))
            }
        }
    }

    fn supports_checksums(&self) -> bool {
        true
    }

    fn requires_precise(&self) -> bool {
        false
    }

    fn source_id(&self) -> SourceId {
        self.source_id
    }

    fn invalidate_cache(&mut self) {
        self.index.clear_summaries_cache();
        self.ops.invalidate_cache();
    }

    fn set_quiet(&mut self, quiet: bool) {
        self.ops.set_quiet(quiet);
    }

    fn download(&mut self, package: PackageId) -> CargoResult<MaybePackage> {
        let hash = loop {
            match self.index.hash(package, &mut *self.ops)? {
                Poll::Pending => self.block_until_ready()?,
                Poll::Ready(hash) => break hash,
            }
        };
        match self.ops.download(package, hash)? {
            MaybeLock::Ready(file) => self.get_pkg(package, &file).map(MaybePackage::Ready),
            MaybeLock::Download {
                url,
                descriptor,
                authorization,
            } => Ok(MaybePackage::Download {
                url,
                descriptor,
                authorization,
            }),
        }
    }

    fn finish_download(&mut self, package: PackageId, data: Vec<u8>) -> CargoResult<Package> {
        let hash = loop {
            match self.index.hash(package, &mut *self.ops)? {
                Poll::Pending => self.block_until_ready()?,
                Poll::Ready(hash) => break hash,
            }
        };
        let file = self.ops.finish_download(package, hash, &data)?;
        self.get_pkg(package, &file)
    }

    fn fingerprint(&self, pkg: &Package) -> CargoResult<String> {
        Ok(pkg.package_id().version().to_string())
    }

    fn describe(&self) -> String {
        self.source_id.display_index()
    }

    fn add_to_yanked_whitelist(&mut self, pkgs: &[PackageId]) {
        self.yanked_whitelist.extend(pkgs);
    }

    fn is_yanked(&mut self, pkg: PackageId) -> Poll<CargoResult<bool>> {
        self.index.is_yanked(pkg, &mut *self.ops)
    }

    fn block_until_ready(&mut self) -> CargoResult<()> {
        // Before starting to work on the registry, make sure that
        // `<cargo_home>/registry` is marked as excluded from indexing and
        // backups. Older versions of Cargo didn't do this, so we do it here
        // regardless of whether `<cargo_home>` exists.
        //
        // This does not use `create_dir_all_excluded_from_backups_atomic` for
        // the same reason: we want to exclude it even if the directory already
        // exists.
        //
        // IO errors in creating and marking it are ignored, e.g. in case we're on a
        // read-only filesystem.
        let registry_base = self.config.registry_base_path();
        let _ = registry_base.create_dir();
        exclude_from_backups_and_indexing(&registry_base.into_path_unlocked());

        self.ops.block_until_ready()
    }
}

/// Get the maximum upack size that Cargo permits
/// based on a given `size` of your compressed file.
///
/// Returns the larger one between `size * max compression ratio`
/// and a fixed max unpacked size.
///
/// In reality, the compression ratio usually falls in the range of 2:1 to 10:1.
/// We choose 20:1 to cover almost all possible cases hopefully.
/// Any ratio higher than this is considered as a zip bomb.
///
/// In the future we might want to introduce a configurable size.
///
/// Some of the real world data from common compression algorithms:
///
/// * <https://www.zlib.net/zlib_tech.html>
/// * <https://cran.r-project.org/web/packages/brotli/vignettes/brotli-2015-09-22.pdf>
/// * <https://blog.cloudflare.com/results-experimenting-brotli/>
/// * <https://tukaani.org/lzma/benchmarks.html>
fn max_unpack_size(config: &Config, size: u64) -> u64 {
    const SIZE_VAR: &str = "__CARGO_TEST_MAX_UNPACK_SIZE";
    const RATIO_VAR: &str = "__CARGO_TEST_MAX_UNPACK_RATIO";
    let max_unpack_size = if cfg!(debug_assertions) && config.get_env(SIZE_VAR).is_ok() {
        // For integration test only.
        config
            .get_env(SIZE_VAR)
            .unwrap()
            .parse()
            .expect("a max unpack size in bytes")
    } else {
        MAX_UNPACK_SIZE
    };
    let max_compression_ratio = if cfg!(debug_assertions) && config.get_env(RATIO_VAR).is_ok() {
        // For integration test only.
        config
            .get_env(RATIO_VAR)
            .unwrap()
            .parse()
            .expect("a max compression ratio in bytes")
    } else {
        MAX_COMPRESSION_RATIO
    };

    u64::max(max_unpack_size, size * max_compression_ratio as u64)
}

/// Constructs a path to a dependency in the registry index on filesystem.
/// See [`cargo_util::registry::make_dep_path`] for more.
fn make_dep_prefix(name: &str) -> String {
    cargo_util::registry::make_dep_path(name, true)
}
