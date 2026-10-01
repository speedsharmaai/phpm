//! The global package store: fetch dists, extract them once, place them into `vendor/`.
//!
//! ```text
//! lock entry ──fetch──▶ zip in memory ──extract──▶ <cache>/pkgs/v1/<vendor~pkg>/<ref>/
//!                                                        │ clone / hardlink / copy
//!                                                        ▼
//!                                              vendor/<install path>/
//! ```

mod auth;
mod error;
mod extract;
mod fetch;
mod link;
mod store;
mod sys;
#[cfg(test)]
mod testutil;
mod untar;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::task::JoinSet;

pub use auth::{Auth, BITBUCKET_TOKEN_URL, Credential, sanitize, split_inline_credentials};
pub use error::{Error, Result};
pub use fetch::{FetchOptions, Fetcher, Response, USER_AGENT};
pub use link::{LinkMode, Placement, place, place_unshared, prune};
pub use store::{Store, cache_dir, store_key};

/// A locked package's `dist`, as `composer.lock` has it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dist {
    pub kind: String,
    pub url: String,
    pub reference: Option<String>,
    pub shasum: Option<String>,
}

/// One package to install: name, dist, and where it goes relative to `vendor/`.
///
/// Metapackages have no dist and no install path; callers leave them out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    pub name: String,
    pub dist: Dist,
    /// `<name>` for libraries; `target-dir` and custom installers change it.
    pub install_path: PathBuf,
}

impl Package {
    /// A package installed at `vendor/<name>`, Composer's default.
    pub fn new(name: impl Into<String>, dist: Dist) -> Self {
        let name = name.into();
        Self {
            install_path: PathBuf::from(&name),
            name,
            dist,
        }
    }

    /// The store key: the dist reference, or a hash of the URL.
    pub fn key(&self) -> String {
        store_key(self.dist.reference.as_deref(), &self.dist.url)
    }
}

impl Store {
    /// Where `package` will be placed from.
    pub fn placement(&self, package: &Package) -> Result<Placement> {
        Ok(Placement {
            source: self.path(&package.name, &package.key())?,
            install_path: package.install_path.clone(),
        })
    }

    /// Download and extract every package not already in the store.
    ///
    /// Downloads run concurrently within the fetcher's limits; each archive is
    /// extracted on the blocking pool as soon as it arrives. Returns the names
    /// fetched, sorted.
    pub async fn fetch_missing(
        &self,
        fetcher: &Fetcher,
        packages: &[Package],
    ) -> Result<Vec<String>> {
        let fetched = self.fetch_missing_sized(fetcher, packages).await?;
        Ok(fetched.into_iter().map(|(name, _)| name).collect())
    }

    /// [`Store::fetch_missing`], with the bytes downloaded for each package.
    pub async fn fetch_missing_sized(
        &self,
        fetcher: &Fetcher,
        packages: &[Package],
    ) -> Result<Vec<(String, u64)>> {
        self.fetch_missing_then(fetcher, packages, Arc::new(|_: &Path| {}))
            .await
    }

    /// [`Store::fetch_missing_sized`], running `then` on each extracted tree
    /// on the blocking pool while the other downloads go on.
    pub async fn fetch_missing_then(
        &self,
        fetcher: &Fetcher,
        packages: &[Package],
        then: Arc<dyn Fn(&Path) + Send + Sync>,
    ) -> Result<Vec<(String, u64)>> {
        let mut seen = BTreeSet::new();
        let mut tasks = JoinSet::new();
        for package in packages {
            let key = package.key();
            if !seen.insert((package.name.clone(), key.clone()))
                || self.contains(&package.name, &key)
            {
                continue;
            }
            let insert = match package.dist.kind.as_str() {
                "zip" => Store::insert_zip,
                "tar" => Store::insert_tar,
                other => {
                    return Err(Error::InvalidPackage {
                        package: package.name.clone(),
                        reason: format!("dist type {other:?} is not supported yet"),
                    });
                }
            };
            let store = self.clone();
            let fetcher = fetcher.clone();
            let package = package.clone();
            let then = Arc::clone(&then);
            tasks.spawn(async move {
                let bytes = fetcher
                    .fetch(&package.dist.url, package.dist.shasum.as_deref())
                    .await?;
                let name = package.name.clone();
                let size = bytes.len() as u64;
                tokio::task::spawn_blocking(move || {
                    let dir = insert(&store, &name, &key, &bytes)?;
                    then(&dir);
                    Ok::<_, Error>(())
                })
                .await
                .map_err(|e| Error::Task(e.to_string()))??;
                Ok::<_, Error>((package.name, size))
            });
        }
        let mut fetched = Vec::new();
        while let Some(joined) = tasks.join_next().await {
            match joined {
                Ok(Ok(done)) => fetched.push(done),
                Ok(Err(e)) => return Err(e),
                Err(e) => return Err(Error::Task(e.to_string())),
            }
        }
        fetched.sort();
        Ok(fetched)
    }
}

#[cfg(test)]
mod tests {
    use super::{Dist, FetchOptions, Fetcher, Package, Store};
    use crate::testutil::TempDir;
    use std::path::PathBuf;

    fn dist(kind: &str, reference: Option<&str>) -> Dist {
        Dist {
            kind: kind.into(),
            url: "https://example.invalid/a.zip".into(),
            reference: reference.map(str::to_owned),
            shasum: None,
        }
    }

    #[test]
    fn installs_under_the_package_name_by_default() {
        let p = Package::new("monolog/monolog", dist("zip", Some("abc")));
        assert_eq!(p.install_path, PathBuf::from("monolog/monolog"));
        assert_eq!(p.key(), "abc");
        let store = Store::new(&PathBuf::from("/c"));
        assert_eq!(
            store.placement(&p).unwrap().source,
            PathBuf::from("/c/pkgs/v1/monolog~monolog/abc")
        );
    }

    #[tokio::test]
    async fn refuses_dist_types_it_cannot_extract() {
        let tmp = TempDir::new("lib-tar");
        let store = Store::new(tmp.path());
        let fetcher = Fetcher::new(FetchOptions::default()).unwrap();
        let err = store
            .fetch_missing(&fetcher, &[Package::new("a/b", dist("rar", Some("x")))])
            .await
            .unwrap_err();
        assert!(err.to_string().contains("dist type \"rar\""), "{err}");
    }

    #[tokio::test]
    async fn skips_what_the_store_already_has() {
        let tmp = TempDir::new("lib-skip");
        let store = Store::new(tmp.path());
        std::fs::create_dir_all(store.path("a/b", "x").unwrap()).unwrap();
        let fetcher = Fetcher::new(FetchOptions::default()).unwrap();
        let fetched = store
            .fetch_missing(&fetcher, &[Package::new("a/b", dist("zip", Some("x")))])
            .await
            .unwrap();
        assert!(fetched.is_empty());
    }
}
