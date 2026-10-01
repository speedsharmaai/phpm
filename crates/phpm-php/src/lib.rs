//! Byte-exact reproductions of the PHP functions whose output Composer writes
//! to disk: `json_encode`, `var_export` and the comparison functions its sorts
//! use.

mod export;
mod json;
mod sort;

pub use export::{
    PhpArray, PhpKey, PhpValue, array_from_json, dump_to_php_code, is_absolute_path,
    value_from_json, var_export, var_export_str,
};
pub use json::{
    detect_indent, encode_pretty, encode_pretty_escaped, encode_pretty_indented, format_float,
};
pub use sort::{smart_strcmp, strnatcasecmp, strnatcmp};
