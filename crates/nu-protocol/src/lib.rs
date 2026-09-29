#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "os"), allow(unused))]
#![cfg_attr(
    not(target_arch = "wasm32"),
    allow(
        clippy::disallowed_types,
        reason = "This file may be compiled as host build-script code while building the wasm target"
    )
)]

mod alias;
pub mod ast;
pub mod casing;
mod collection_columns;
mod completion;
pub mod config;
pub mod debugger;
mod deprecation;
mod did_you_mean;
pub mod engine;
mod errors;
pub mod eval_base;
pub mod eval_const;
mod example;
mod id;
pub mod ir;
mod last_result;
mod lev_distance;
mod module;
mod one_of;
pub mod parser_path;
mod pipeline;
#[cfg(feature = "plugin")]
mod plugin;
#[cfg(feature = "os")]
pub mod process;
mod signature;
pub mod span;
mod syntax_shape;
mod ty;
mod ty_relation;
mod value;

pub use alias::*;
pub use ast::unit::*;
pub use collection_columns::*;
pub use completion::*;
pub use config::*;
pub use deprecation::*;
pub use did_you_mean::did_you_mean;
pub use engine::{
    ENV_VARIABLE_ID, IN_VARIABLE_ID, LAST_RESULT_VAR_NAME, LAST_VARIABLE_ID, NU_VARIABLE_ID,
};
pub use errors::*;
pub use example::*;
pub use id::*;
pub use last_result::{
    block_is_bare_last_result, block_is_bare_last_result_with, truncate_value_to_budget,
    value_is_error_only,
};
pub use lev_distance::levenshtein_distance;
pub use module::*;
pub use one_of::*;
pub use pipeline::*;
#[cfg(feature = "plugin")]
pub use plugin::*;
pub use signature::*;
pub use span::*;
pub use syntax_shape::*;
pub use ty::*;
pub use ty_relation::*;
pub use value::*;

pub use nu_derive_value::*;

#[cfg(test)]
#[macro_use]
extern crate nu_test_support;

#[cfg(test)]
use nu_test_support::harness::main;

/// The tag prefix used to encode a [`Value::Decimal`] as a string in data
/// formats that have no native decimal type (JSON, MessagePack, TOML, NUON,
/// SQLite).
///
/// A decimal is encoded as `"!decimal:<decimal>"`, e.g. `!decimal:1.5`.
/// See [`encode_decimal_string`] and [`decode_decimal_string`].
pub const DECIMAL_STRING_TAG: &str = "!decimal:";

/// Encodes a [`rust_decimal::Decimal`] as a tagged string (e.g. `"!decimal:1.5"`)
/// for data formats that cannot represent decimals natively.
///
/// The result can be converted back with [`decode_decimal_string`].
pub fn encode_decimal_string(val: rust_decimal::Decimal) -> String {
    format!("{DECIMAL_STRING_TAG}{val}")
}

/// Decodes a tagged decimal string produced by [`encode_decimal_string`].
///
/// Returns `None` if `s` does not start with [`DECIMAL_STRING_TAG`] or the
/// payload is not a valid decimal, in which case `s` should be treated as an
/// ordinary string.
pub fn decode_decimal_string(s: &str) -> Option<rust_decimal::Decimal> {
    s.strip_prefix(DECIMAL_STRING_TAG)?.parse().ok()
}

/// Creates a ShellError for decimal to float conversion failures
pub fn decimal_to_float_error(span: Span) -> ShellError {
    ShellError::CantConvert {
        to_type: "float".into(),
        from_type: "decimal".into(),
        span,
        help: Some(
            "Decimal value is too large or has too much precision to convert to float".into(),
        ),
    }
}
