//! The root package's scripts, and whether phpm can run an event's listeners
//! itself or only Composer can.

use std::collections::BTreeSet;

use phpm_lock::ComposerJson;
use serde_json::Value;

use crate::project::Env;

/// Script events `composer install` fires around its own steps.
pub(crate) const PRE_INSTALL: &str = "pre-install-cmd";
pub(crate) const PRE_AUTOLOAD: &str = "pre-autoload-dump";
pub(crate) const POST_AUTOLOAD: &str = "post-autoload-dump";
pub(crate) const POST_INSTALL: &str = "post-install-cmd";

/// Who can run an event's listeners.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Route {
    /// No listeners.
    Empty,
    /// Shell, `@php`, `@putenv` and references to such scripts. `composer`
    /// says some listener calls Composer itself.
    Native { listeners: usize, composer: bool },
    /// Only Composer can run them, for the reason given.
    Composer(String),
}

/// `scripts` from composer.json, normalised like Composer's `ArrayLoader`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Scripts {
    events: Vec<(String, Vec<Value>)>,
    root_bins: Vec<String>,
    skip: BTreeSet<String>,
}

pub(crate) const NO_ARGS: &str = "@no_additional_args";

/// One listener, as `EventDispatcher::doDispatch` sorts them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Listener<'a> {
    /// `@composer ...`
    ComposerCall(&'a str),
    /// `@name args...`, another script.
    Reference { name: &'a str, args: Vec<&'a str> },
    /// `Class::method`
    Callable(&'a str),
    /// `Vendor\SomeCommand`
    CommandClass(&'a str),
    /// `@php ...`, `@putenv ...` or a shell command.
    Exec(&'a str),
}

// Composer: EventDispatcher::isComposerScript, isPhpScript, isCommandClass
pub(crate) fn listener(text: &str) -> Listener<'_> {
    if text.starts_with('@') && !text.starts_with("@php ") && !text.starts_with("@putenv ") {
        if text.starts_with("@composer ") {
            return Listener::ComposerCall(text);
        }
        let mut parts = text[1..].split(' ');
        let name = parts.next().unwrap_or_default();
        return Listener::Reference {
            name,
            args: parts.collect(),
        };
    }
    if !text.contains(' ') && text.contains("::") {
        return Listener::Callable(text);
    }
    if text.contains('\\') && !text.contains(' ') && text.ends_with("Command") {
        return Listener::CommandClass(text);
    }
    Listener::Exec(text)
}

/// The listener with `@no_additional_args` taken out, and whether it was there.
pub(crate) fn strip_no_args(text: &str) -> (String, bool) {
    if !text.contains(NO_ARGS) {
        return (text.to_owned(), false);
    }
    let stripped = text
        .replace(&format!(" {NO_ARGS}"), "")
        .replace(NO_ARGS, "");
    (stripped, true)
}

// Composer: EventDispatcher::doDispatch, the `\b<script>$` match on root bins
fn is_bin_suffix(bin: &str, exec: &str) -> bool {
    bin.strip_suffix(exec).is_some_and(|head| {
        let before = head.chars().next_back();
        let first = exec.chars().next();
        let word = |c: Option<char>| c.is_some_and(|c| c.is_alphanumeric() || c == '_');
        !exec.is_empty() && word(before) != word(first)
    })
}

impl Scripts {
    pub(crate) fn new(composer: &ComposerJson, env: Env<'_>) -> Self {
        let events = composer
            .data()
            .get("scripts")
            .and_then(Value::as_object)
            .map(|map| {
                map.iter()
                    .map(|(event, value)| {
                        let list = match value {
                            Value::Null => Vec::new(),
                            Value::Array(items) => items.clone(),
                            Value::Object(_) => vec![Value::Object(serde_json::Map::new())],
                            other => vec![other.clone()],
                        };
                        (event.clone(), list)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let root_bins = match composer.data().get("bin") {
            Some(Value::String(s)) => vec![s.clone()],
            Some(Value::Array(items)) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            _ => Vec::new(),
        };
        let skip = env("COMPOSER_SKIP_SCRIPTS")
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            events,
            root_bins,
            skip,
        }
    }

    /// The listeners for `event`, empty when it has none or is skipped.
    pub(crate) fn listeners(&self, event: &str) -> &[Value] {
        if self.skip.contains(event) {
            return &[];
        }
        self.events
            .iter()
            .find(|(e, _)| e == event)
            .map_or(&[], |(_, list)| list.as_slice())
    }

    pub(crate) fn has(&self, event: &str) -> bool {
        !self.listeners(event).is_empty()
    }

    pub(crate) fn exists(&self, event: &str) -> bool {
        self.events.iter().any(|(e, _)| e == event)
    }

    pub(crate) fn route(&self, event: &str) -> Route {
        let mut seen = Vec::new();
        let mut composer = false;
        match self.walk(event, &mut seen, &mut composer) {
            Err(reason) => Route::Composer(reason),
            Ok(0) => Route::Empty,
            Ok(listeners) => Route::Native {
                listeners,
                composer,
            },
        }
    }

    fn walk(
        &self,
        event: &str,
        seen: &mut Vec<String>,
        composer: &mut bool,
    ) -> Result<usize, String> {
        if seen.iter().any(|s| s == event) {
            return Err(format!(
                "the scripts that {event} references loop back to it"
            ));
        }
        seen.push(event.to_owned());
        let mut count = 0;
        for value in self.listeners(event) {
            let Value::String(raw) = value else {
                return Err(format!("{event} has a listener only a plugin understands"));
            };
            let (text, _) = strip_no_args(raw);
            match listener(&text) {
                Listener::Callable(c) => {
                    return Err(format!("{event} calls {c}, a PHP callable"));
                }
                Listener::CommandClass(c) => {
                    return Err(format!("{event} runs {c}, a Symfony command class"));
                }
                Listener::Reference { name, .. } => {
                    count += self.walk(name, seen, composer)?;
                }
                Listener::ComposerCall(_) => {
                    *composer = true;
                    count += 1;
                }
                Listener::Exec(exec) => {
                    if self.root_bins.iter().any(|b| is_bin_suffix(b, exec)) {
                        return Err(format!(
                            "{event} runs {exec}, one of the root package's bins"
                        ));
                    }
                    *composer |= exec.starts_with("composer ");
                    count += 1;
                }
            }
        }
        seen.pop();
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use super::{Listener, Route, Scripts, listener, strip_no_args};
    use phpm_lock::ComposerJson;
    use serde_json::{Value, json};

    fn scripts(v: &Value) -> Scripts {
        scripts_env(v, None)
    }

    fn scripts_env(v: &Value, skip: Option<&'static str>) -> Scripts {
        let composer =
            ComposerJson::from_value(json!({"scripts": v, "bin": ["bin/tool"]})).unwrap();
        let env = move |k: &str| {
            (k == "COMPOSER_SKIP_SCRIPTS")
                .then(|| skip.map(str::to_owned))
                .flatten()
        };
        Scripts::new(&composer, &env)
    }

    #[test]
    fn sorts_listeners_like_the_event_dispatcher() {
        assert_eq!(
            listener("@composer dump"),
            Listener::ComposerCall("@composer dump")
        );
        assert_eq!(
            listener("@test --filter x"),
            Listener::Reference {
                name: "test",
                args: vec!["--filter", "x"]
            }
        );
        assert_eq!(listener("@php artisan"), Listener::Exec("@php artisan"));
        assert_eq!(listener("@putenv A=1"), Listener::Exec("@putenv A=1"));
        assert_eq!(listener("A\\B::c"), Listener::Callable("A\\B::c"));
        assert_eq!(
            listener("A\\BuildCommand"),
            Listener::CommandClass("A\\BuildCommand")
        );
        assert_eq!(listener("echo A::b"), Listener::Exec("echo A::b"));
        assert_eq!(listener("phpunit"), Listener::Exec("phpunit"));
        assert_eq!(
            strip_no_args("a @no_additional_args"),
            ("a".to_owned(), true)
        );
        assert_eq!(strip_no_args("@no_additional_args"), (String::new(), true));
        assert_eq!(strip_no_args("a"), ("a".to_owned(), false));
        assert!(super::is_bin_suffix("bin/tool", "tool"));
        assert!(super::is_bin_suffix("bin/tool", "bin/tool"));
        assert!(!super::is_bin_suffix("bin/tool", "ool"));
        assert!(!super::is_bin_suffix("bin/tool", ""));
    }

    #[test]
    fn routes_events() {
        let s = scripts(&json!({
            "pre-install-cmd": "echo hi",
            "post-install-cmd": ["@php -v", "@helper", "@composer --version"],
            "helper": ["@putenv X=1", "composer --version"],
            "post-autoload-dump": ["Illuminate\\Foundation\\ComposerScripts::postAutoloadDump", "@php artisan"],
            "pre-autoload-dump": ["@loop"],
            "loop": "@loop",
            "cmd": "App\\FooCommand",
            "bin": "bin/tool",
            "odd": [1],
            "auto-scripts": {"cache:clear": "symfony-cmd"},
            "uses-auto": "@auto-scripts",
            "none": null,
            "ref-missing": "@nowhere",
        }));
        assert_eq!(
            s.route("pre-install-cmd"),
            Route::Native {
                listeners: 1,
                composer: false
            }
        );
        assert_eq!(
            s.route("post-install-cmd"),
            Route::Native {
                listeners: 4,
                composer: true
            }
        );
        assert_eq!(
            s.route("post-autoload-dump"),
            Route::Composer(
                "post-autoload-dump calls Illuminate\\Foundation\\ComposerScripts::postAutoloadDump, a PHP callable"
                    .into()
            )
        );
        assert!(
            matches!(s.route("pre-autoload-dump"), Route::Composer(r) if r.contains("loop back"))
        );
        assert!(
            matches!(s.route("cmd"), Route::Composer(r) if r.contains("Symfony command class"))
        );
        assert!(matches!(s.route("bin"), Route::Composer(r) if r.contains("root package's bins")));
        assert!(matches!(s.route("odd"), Route::Composer(r) if r.contains("only a plugin")));
        assert!(
            matches!(s.route("uses-auto"), Route::Composer(r) if r.contains("auto-scripts has a listener"))
        );
        assert_eq!(s.route("none"), Route::Empty);
        assert_eq!(s.route("ref-missing"), Route::Empty);
        assert_eq!(s.route("absent"), Route::Empty);
        assert!(s.has("helper") && !s.has("none") && s.exists("none") && !s.exists("absent"));
        assert_eq!(s.listeners("pre-install-cmd"), [json!("echo hi")]);
    }

    #[test]
    fn honours_composer_skip_scripts() {
        let s = scripts_env(
            &json!({"pre-install-cmd": "echo a", "post-install-cmd": "echo b"}),
            Some(" pre-install-cmd , ,x"),
        );
        assert_eq!(s.route("pre-install-cmd"), Route::Empty);
        assert!(s.has("post-install-cmd"));
        let plain = ComposerJson::from_value(json!({})).unwrap();
        assert_eq!(
            Scripts::new(&plain, &|_| None).route("post-install-cmd"),
            Route::Empty
        );
    }
}
