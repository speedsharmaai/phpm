# Vendored symfony/runtime files

Copied byte for byte from `symfony/runtime`'s `Internal/autoload_runtime.template`,
unchanged across every stable tag v7.0.0-v8.1.0 (checked). The `symfony_runtime`
adapter (`crates/phpm/src/adapters/symfony_runtime.rs`) fills in its
`%runtime_class%` and `%runtime_options%` placeholders exactly as
`Internal/ComposerPlugin.php::updateAutoloadFile` does.

| File | Source |
|---|---|
| `autoload_runtime.template` | `symfony/runtime` v8.1.0, `Internal/autoload_runtime.template` |

Symfony's MIT licence covers this file; see symfony/runtime's own `LICENSE`.
