//! Reading `composer.json` and `composer.lock`, and writing the installed
//! metadata Composer keeps in `vendor/composer/`.

pub mod constraint;
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
    CLASS_LOADER_PHP, COMPOSER_LICENSE, COMPOSER_VERSION, INSTALLED_VERSIONS_PHP, InstallContext,
    InstalledFiles, installed_files,
};
pub use manifest::{ComposerJson, InstallPreferences, Lock, LockAlias};
pub use paths::{find_shortest_path, find_shortest_path_with, normalize_path};
pub use root::{NO_VERSION_SET, RootVersion, root_version};
