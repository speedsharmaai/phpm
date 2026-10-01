use crate::package::Package;
use indexmap::IndexMap;
use phpm_php::strnatcasecmp;
use std::collections::{HashMap, HashSet};

struct Weights<'a> {
    usage: IndexMap<&'a str, Vec<&'a str>>,
    computing: HashSet<&'a str>,
    computed: HashMap<&'a str, i64>,
}

impl<'a> Weights<'a> {
    fn importance(&mut self, name: &'a str) -> i64 {
        if let Some(&weight) = self.computed.get(name) {
            return weight;
        }
        if self.computing.contains(name) {
            return 0;
        }
        self.computing.insert(name);
        let mut weight = 0;
        let users = self.usage.get(name).cloned().unwrap_or_default();
        for user in users {
            weight -= 1 - self.importance(user);
        }
        self.computing.remove(name);
        self.computed.insert(name, weight);
        weight
    }
}

/// Indices of `packages` in `PackageSorter::sortPackages` order: most
/// depended-on first, ties by `strnatcasecmp` of the name. Circular
/// dependencies make the weights depend on the input order, as in Composer.
// Composer: Util/PackageSorter.php sortPackages
pub(crate) fn sort_packages(packages: &[&Package]) -> Vec<usize> {
    let mut usage: IndexMap<&str, Vec<&str>> = IndexMap::new();
    for package in packages {
        for link in &package.requires {
            usage
                .entry(link.target.as_str())
                .or_default()
                .push(package.name.as_str());
        }
    }
    let mut weights = Weights {
        usage,
        computing: HashSet::new(),
        computed: HashMap::new(),
    };
    let mut weighted: Vec<(usize, i64)> = packages
        .iter()
        .enumerate()
        .map(|(i, p)| (i, weights.importance(p.name.as_str())))
        .collect();
    weighted.sort_by(|(a, wa), (b, wb)| {
        wa.cmp(wb)
            .then_with(|| strnatcasecmp(&packages[*a].name, &packages[*b].name))
    });
    weighted.into_iter().map(|(i, _)| i).collect()
}

#[cfg(test)]
mod tests {
    use super::sort_packages;
    use crate::package::{Link, Package};

    fn pkg(name: &str, requires: &[&str]) -> Package {
        Package {
            name: name.into(),
            requires: requires
                .iter()
                .map(|t| Link {
                    target: (*t).into(),
                    constraint: "*".into(),
                })
                .collect(),
            ..Package::default()
        }
    }

    fn names(packages: &[Package]) -> Vec<&str> {
        let refs: Vec<&Package> = packages.iter().collect();
        sort_packages(&refs)
            .into_iter()
            .map(|i| packages[i].name.as_str())
            .collect()
    }

    #[test]
    fn dependencies_come_first() {
        let packages = [
            pkg("a/app", &["b/lib", "c/lib"]),
            pkg("b/lib", &["c/lib"]),
            pkg("c/lib", &[]),
        ];
        assert_eq!(names(&packages), ["c/lib", "b/lib", "a/app"]);
    }

    #[test]
    fn ties_use_natural_case_insensitive_order() {
        let packages = [pkg("x/item10", &[]), pkg("X/Item9", &[]), pkg("a/a", &[])];
        assert_eq!(names(&packages), ["a/a", "X/Item9", "x/item10"]);
    }

    #[test]
    fn cycles_depend_on_input_order() {
        let forward = [pkg("a/one", &["b/two"]), pkg("b/two", &["a/one"])];
        assert_eq!(names(&forward), ["a/one", "b/two"]);
        let backward = [pkg("b/two", &["a/one"]), pkg("a/one", &["b/two"])];
        assert_eq!(names(&backward), ["b/two", "a/one"]);
    }
}
