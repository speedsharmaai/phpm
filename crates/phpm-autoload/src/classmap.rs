use crate::Error;
use crate::autoloads::Autoloads;
use indexmap::IndexMap;
use phpm_php::smart_strcmp;

/// Class name to file path, first found wins, as composer/class-map-generator
/// collects it.
#[derive(Debug, Default)]
pub(crate) struct ClassMap {
    map: IndexMap<String, String>,
}

impl ClassMap {
    pub(crate) fn scan(
        autoloads: &Autoloads,
        optimize: bool,
        _base: &str,
        _vendor: &str,
    ) -> Result<Self, Error> {
        if optimize || !autoloads.classmap.is_empty() {
            return Err(Error::Unsupported("class scanning".into()));
        }
        Ok(Self::default())
    }

    pub(crate) fn add_class(&mut self, class: &str, path: &str) {
        self.map.insert(class.to_owned(), path.to_owned());
    }

    pub(crate) fn sort(&mut self) {
        self.map.sort_by(|a, _, b, _| smart_strcmp(a, b));
    }

    pub(crate) fn entries(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (k.as_str(), v.as_str()))
    }

    pub(crate) fn warnings(self) -> Vec<String> {
        drop(self.map);
        Vec::new()
    }
}
