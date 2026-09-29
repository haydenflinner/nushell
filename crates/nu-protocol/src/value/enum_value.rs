use crate::{
    CompareTypes, CustomValue, EnumDef, Record, ShellError, Span, Value,
    ast::{Comparison, Operator},
    casing::Casing,
    shell_error::generic::GenericError,
};
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::cmp::Ordering;

/// A value of a user-declared `enum` type (`enum Shape { ... }`).
///
/// Enum values are nominal: their [`Value::get_type`] is
/// `Type::Custom("<enum name>")`, which only compares equal to the same enum.
///
/// For display, serialization, cell-path access, and `match` destructuring an
/// enum value lowers to its [`base_record`](Self::base_record): the variant
/// name under `kind` and the payload — when there is one — under `payload`.
/// The encoding is uniform: a record payload stays a nested record, so the
/// base record is always `{kind, payload?}` and `match` can rely on the
/// `payload` key existing for every payload-carrying variant.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnumValue {
    /// The name of the declared enum type, e.g. `Shape`.
    pub enum_name: String,
    /// The variant name, e.g. `circle`.
    pub variant: String,
    /// The variant payload, or `None` for unit variants.
    pub payload: Option<Box<Value>>,
}

impl EnumValue {
    pub fn new(
        enum_name: impl Into<String>,
        variant: impl Into<String>,
        payload: Option<Value>,
    ) -> Self {
        EnumValue {
            enum_name: enum_name.into(),
            variant: variant.into(),
            payload: payload.map(Box::new),
        }
    }

    /// The record this value lowers to: `{kind: "<variant>", payload:
    /// <value>}`, with no `payload` key for unit variants. Payloads are
    /// always nested under `payload` — never spread — so payload records
    /// may freely use `kind` or `payload` as field names.
    pub fn base_record(&self, span: Span) -> Record {
        let mut record = Record::new();
        record.push("kind", Value::string(self.variant.clone(), span));

        if let Some(payload) = &self.payload {
            record.push("payload", (**payload).clone());
        }

        record
    }

    /// Rebuild an enum value from its [`base_record`](Self::base_record)
    /// encoding: `{kind: "<variant>"}` for unit variants,
    /// `{kind: "<variant>", payload: <value>}` otherwise. The variant name
    /// and payload are validated against `enum_def` — this is the typed
    /// decode path shared by `Type.from-record` and `--as` validation.
    ///
    /// Errors when `value` is not a record, `kind` is missing or unknown,
    /// `payload` is absent for a payload variant or fails the declared
    /// payload shape, or extra fields are present.
    pub fn from_base_record(
        value: Value,
        enum_name: &str,
        enum_def: &EnumDef,
        span: Span,
    ) -> Result<Value, ShellError> {
        let Value::Record { val: record, .. } = value else {
            return Err(ShellError::CantConvert {
                to_type: enum_name.to_string(),
                from_type: value.get_type().to_string(),
                span: value.span(),
                help: Some(format!(
                    "expected a base record like {{kind: \"<variant>\", ...}}"
                )),
            });
        };
        let mut record = record.into_owned();

        let Some(kind) = record.remove("kind") else {
            return Err(GenericError::new(
                format!("missing `kind` field for `{enum_name}`"),
                "the base record needs a `kind` field naming the variant",
                span,
            )
            .into());
        };

        let variant_name = match kind {
            Value::String { val, .. } => val,
            other => {
                return Err(GenericError::new(
                    "`kind` must be a string naming the variant",
                    format!("got {}", other.get_type()),
                    other.span(),
                )
                .into());
            }
        };

        let Some(variant) = enum_def.get_variant(&variant_name) else {
            return Err(GenericError::new(
                format!("unknown variant `{variant_name}`"),
                format!("`{enum_name}` has no variant `{variant_name}`"),
                span,
            )
            .with_help(format!(
                "variants of `{enum_name}`: {}",
                enum_def.variant_names().join(", ")
            ))
            .into());
        };

        let payload = match &variant.payload {
            Some(shape) => {
                let Some(payload) = record.remove("payload") else {
                    return Err(GenericError::new(
                        format!("missing `payload` field for `{enum_name}.{variant_name}`"),
                        format!("`{variant_name}` takes a {} payload", shape.to_type()),
                        span,
                    )
                    .into());
                };
                if !record.is_empty() {
                    return Err(GenericError::new(
                        format!("extra fields in base record for `{enum_name}.{variant_name}`"),
                        "only `kind` and `payload` are allowed here",
                        span,
                    )
                    .into());
                }

                let expected = shape.to_type();
                if !payload.get_type().is_subtype_of(&expected) {
                    return Err(ShellError::CantConvert {
                        to_type: expected.to_string(),
                        from_type: payload.get_type().to_string(),
                        span: payload.span(),
                        help: Some(format!(
                            "variant `{variant_name}` of `{enum_name}` expects {expected}"
                        )),
                    });
                }
                Some(payload)
            }
            None => {
                if !record.is_empty() {
                    return Err(GenericError::new(
                        format!("`{enum_name}.{variant_name}` takes no payload"),
                        "extra fields in the base record",
                        span,
                    )
                    .into());
                }
                None
            }
        };

        Ok(Value::custom(
            Box::new(EnumValue::new(enum_name, variant_name, payload)),
            span,
        ))
    }
}

#[typetag::serde]
impl CustomValue for EnumValue {
    fn clone_value(&self, span: Span) -> Value {
        Value::custom(Box::new(self.clone()), span)
    }

    fn type_name(&self) -> String {
        self.enum_name.clone()
    }

    fn to_base_value(&self, span: Span) -> Result<Value, ShellError> {
        Ok(Value::record(self.base_record(span), span))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_mut_any(&mut self) -> &mut dyn Any {
        self
    }

    fn follow_path_string(
        &self,
        self_span: Span,
        column_name: String,
        path_span: Span,
        optional: bool,
        casing: Casing,
    ) -> Result<Value, ShellError> {
        let base = self.to_base_value(self_span)?;
        let result = base.follow_cell_path(&[crate::ast::PathMember::string(
            column_name,
            optional,
            casing,
            path_span,
        )])?;
        Ok(result.into_owned())
    }

    fn partial_cmp(&self, other: &Value) -> Option<Ordering> {
        let Value::Custom { val, .. } = other else {
            return None;
        };
        let other = val.as_any().downcast_ref::<EnumValue>()?;

        match self
            .enum_name
            .cmp(&other.enum_name)
            .then_with(|| self.variant.cmp(&other.variant))
        {
            Ordering::Equal => match (&self.payload, &other.payload) {
                (None, None) => Some(Ordering::Equal),
                (Some(lhs), Some(rhs)) => lhs.partial_cmp(rhs),
                (None, Some(_)) => Some(Ordering::Less),
                (Some(_), None) => Some(Ordering::Greater),
            },
            ord => Some(ord),
        }
    }

    fn operation(
        &self,
        lhs_span: Span,
        operator: Operator,
        op: Span,
        right: &Value,
    ) -> Result<Value, ShellError> {
        match operator {
            Operator::Comparison(Comparison::Equal) => Ok(Value::bool(
                matches!(self.partial_cmp(right), Some(Ordering::Equal)),
                op,
            )),
            Operator::Comparison(Comparison::NotEqual) => Ok(Value::bool(
                !matches!(self.partial_cmp(right), Some(Ordering::Equal)),
                op,
            )),
            _ => Err(ShellError::OperatorUnsupportedType {
                op: operator,
                unsupported: crate::Type::Custom(self.enum_name.clone().into()),
                op_span: op,
                unsupported_span: lhs_span,
                help: None,
            }),
        }
    }
}
