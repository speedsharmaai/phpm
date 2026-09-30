//! Reading `composer.json` and `composer.lock`, and writing the installed
//! metadata Composer keeps in `vendor/composer/`.

mod error;
mod installed;
mod manifest;
mod package;
mod paths;
pub mod root;
mod time;
pub mod version;

pub use error::Error;
pub use installed::{
    COMPOSER_VERSION, INSTALLED_VERSIONS_PHP, InstallContext, InstalledFiles, installed_files,
};
pub use manifest::{ComposerJson, InstallPreferences, Lock, LockAlias};
pub use paths::{find_shortest_path, normalize_path};
pub use root::{NO_VERSION_SET, RootVersion, root_version};
