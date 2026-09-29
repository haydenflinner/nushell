use crate::{
    CompareTypes, EnumDef, EnumValue, ShellError, Span, SyntaxShape, TypeDefKind, Value,
    engine::EngineState, shell_error::generic::GenericError,
};
use std::collections::{HashMap, HashSet};

/// A single type mismatch found while validating a value against a declared
/// type: where in the value, what was expected, what was found.
#[derive(Debug)]
pub struct ValidationIssue {
    /// JSONPath-style location of the mismatch, e.g. `$.users[3].name`.
    /// `$` is the whole value.
    pub path: String,
    /// What the type expects at `path`.
    pub expected: String,
    /// What was actually found there — a type name or `missing`.
    pub found: String,
    pub span: Span,
}

const MAX_ISSUES: usize = 5;

/// Validate `value` against the declared type named `type_name`, and
/// materialize it: enum base records (`{kind: "v", payload: P}`) become real
/// [`EnumValue`]s wherever a declared enum type appears — at the top level or
/// nested inside record fields, list elements, and instantiated type
/// arguments.
///
/// `type_name` may be a bare declared name (`User`), a module-qualified name
/// (`m.User`), or an instantiated generic (`Option<int>`); type arguments
/// are resolved the same way.
///
/// Mismatches aggregate: every one is collected with its path rather than
/// failing at the first, so all of them can be reported at once.
pub fn validate_value(
    value: Value,
    type_name: &str,
    engine_state: &EngineState,
    removed_overlays: &[Vec<u8>],
    span: Span,
) -> Result<Value, ShellError> {
    let Some((base, args)) = split_instantiation(type_name.as_bytes()) else {
        return Err(GenericError::new(
            format!("invalid type name `{type_name}`"),
            "expected `Name` or `Name<T, U>`",
            span,
        )
        .into());
    };

    let shape = SyntaxShape::Custom(
        String::from_utf8_lossy(base).into(),
        args.iter().map(|arg| name_to_shape(arg)).collect(),
    );

    // Resolve once here for a nicer error than a per-node lookup failure.
    if engine_state
        .find_type_name(base, removed_overlays)
        .is_none()
    {
        return Err(GenericError::new(
            format!("unknown type `{type_name}`"),
            "no `type` declaration with this name is in scope",
            span,
        )
        .into());
    }

    let mut issues = vec![];
    let mut path = Path::new();
    let value = check(
        value,
        &shape,
        engine_state,
        removed_overlays,
        &mut path,
        &mut issues,
    );

    if issues.is_empty() {
        Ok(value)
    } else {
        Err(issues_error(type_name, issues, span))
    }
}

/// Parse the type-argument list of an instantiated type name into shapes —
/// `Pair<Option>` gives `[Custom("Option")]` for binding `F`. `None` when
/// the name isn't a well-formed `Name<args>` spelling.
pub fn instantiation_arg_shapes(type_name: &str) -> Option<Vec<SyntaxShape>> {
    split_instantiation(type_name.as_bytes())
        .map(|(_, args)| args.iter().map(|a| name_to_shape(a)).collect())
}

/// Strictly check `value` against an already-parsed `shape`, collecting
/// every mismatch with its path. Unlike `Value::is_subtype_of`, this walk
/// is deep for declared types — a nested enum's base record is decoded and
/// its payload checked rather than passing on name alone. Used by
/// `enum-construct` for runtime payload checking.
pub fn check_against_shape(
    value: Value,
    shape: &SyntaxShape,
    engine_state: &EngineState,
    removed_overlays: &[Vec<u8>],
) -> Vec<ValidationIssue> {
    let mut issues = vec![];
    let mut path = Path::new();
    check(
        value,
        shape,
        engine_state,
        removed_overlays,
        &mut path,
        &mut issues,
    );
    issues
}

/// [`validate_value`] when the type name is optional — `None` passes the
/// value through unchanged. This is the shape `--as` flags use.
pub fn validate_maybe(
    value: Value,
    type_name: Option<&str>,
    engine_state: &EngineState,
    span: Span,
) -> Result<Value, ShellError> {
    match type_name {
        Some(name) => validate_value(value, name, engine_state, &[], span),
        None => Ok(value),
    }
}

/// Walk `value` against `shape`, returning the value with any enum base
/// records decoded into [`EnumValue`]s. Mismatches accumulate in `issues`.
fn check(
    value: Value,
    shape: &SyntaxShape,
    engine_state: &EngineState,
    removed_overlays: &[Vec<u8>],
    path: &mut Path,
    issues: &mut Vec<ValidationIssue>,
) -> Value {
    check_inner(
        value,
        shape,
        engine_state,
        removed_overlays,
        path,
        issues,
        &mut HashSet::new(),
    )
}

/// [`check`] with an `unfolding` set of alias names currently being expanded
/// at this value position. `Named`/`Custom`-alias chains recurse through
/// here (sharing the set, so `struct A = A` is caught), while descent into
/// record fields and list elements re-enters through [`check`] with a fresh
/// set — a recursive type legitimately revisited at a deeper value is not a
/// cycle.
#[expect(clippy::too_many_arguments)]
fn check_inner(
    value: Value,
    shape: &SyntaxShape,
    engine_state: &EngineState,
    removed_overlays: &[Vec<u8>],
    path: &mut Path,
    issues: &mut Vec<ValidationIssue>,
    unfolding: &mut std::collections::HashSet<Box<str>>,
) -> Value {
    if issues.len() >= MAX_ISSUES {
        return value;
    }

    match shape {
        SyntaxShape::Named(_, inner) => check_inner(
            value,
            inner,
            engine_state,
            removed_overlays,
            path,
            issues,
            unfolding,
        ),
        SyntaxShape::Custom(name, args) => {
            // An already-constructed enum value of this type passes — its
            // payload was checked when it was built. A *different* enum's
            // value is a definite mismatch.
            if let Value::Custom { val, .. } = &value {
                if let Some(ev) = val.as_any().downcast_ref::<EnumValue>() {
                    if ev.enum_name == name.rsplit('.').next().unwrap_or(name) {
                        return value;
                    }
                    issues.push(mismatch(path, shape.to_string(), &value));
                    return value;
                }
            }
            let Some(type_def) = engine_state.find_type_name(name.as_bytes(), removed_overlays)
            else {
                issues.push(mismatch(path, shape.to_string(), &value));
                return value;
            };
            if !args.is_empty() && args.len() != type_def.params.len() {
                issues.push(mismatch(
                    path,
                    format!("{name} takes {} type argument(s)", type_def.params.len()),
                    &value,
                ));
                return value;
            }
            let bindings: HashMap<&str, &SyntaxShape> = type_def
                .params
                .iter()
                .map(|p| p.as_str())
                .zip(args.iter())
                .collect();
            match &type_def.kind {
                TypeDefKind::Enum(enum_def) => {
                    // Substitute the type arguments into each variant's
                    // declared payload before decoding the base record.
                    let instantiated = EnumDef {
                        variants: enum_def
                            .variants
                            .iter()
                            .map(|v| crate::EnumVariant {
                                name: v.name.clone(),
                                payload: v.payload.as_ref().map(|p| p.substitute(&bindings)),
                            })
                            .collect(),
                    };
                    // `from_base_record` checks the payload shallowly
                    // (`is_subtype_of`), which deliberately treats any record
                    // as assignable to a `Custom` type — a nested enum's base
                    // record would pass without its payload being checked.
                    // Walk the payload strictly here instead.
                    let issues_before = issues.len();
                    if let Value::Record { val, .. } = &value {
                        if let Some(Value::String { val: kind, .. }) = val.get("kind") {
                            if let Some(variant) =
                                instantiated.variants.iter().find(|v| v.name == *kind)
                            {
                                path.push_field("payload");
                                match (val.get("payload"), &variant.payload) {
                                    (Some(payload_val), Some(payload_shape)) => {
                                        let _ = check(
                                            payload_val.clone(),
                                            payload_shape,
                                            engine_state,
                                            removed_overlays,
                                            path,
                                            issues,
                                        );
                                    }
                                    (None, Some(payload_shape)) => {
                                        issues.push(ValidationIssue {
                                            path: path.render(),
                                            expected: payload_shape.to_string(),
                                            found: "missing".into(),
                                            span: value.span(),
                                        });
                                    }
                                    _ => {}
                                }
                                path.pop();
                            }
                        }
                    }
                    let span = value.span();
                    match EnumValue::from_base_record(value.clone(), name, &instantiated, span) {
                        Ok(decoded) => decoded,
                        Err(err) => {
                            // The strict payload walk above already reported
                            // this node's problem with a better path — don't
                            // double-report it as a decode failure.
                            if issues.len() == issues_before {
                                issues.push(ValidationIssue {
                                    path: path.render(),
                                    expected: shape.to_string(),
                                    found: err.to_string(),
                                    span,
                                });
                            }
                            value
                        }
                    }
                }
                TypeDefKind::Alias(inner) => {
                    if !unfolding.insert(name.clone()) {
                        issues.push(ValidationIssue {
                            path: path.render(),
                            expected: shape.to_string(),
                            found: format!("recursive type alias `{name}`"),
                            span: value.span(),
                        });
                        return value;
                    }
                    let inner = inner.substitute(&bindings);
                    let value = check_inner(
                        value,
                        &inner,
                        engine_state,
                        removed_overlays,
                        path,
                        issues,
                        unfolding,
                    );
                    unfolding.remove(name);
                    value
                }
            }
        }
        SyntaxShape::Record(cols) => match &value {
            Value::Record { val, .. } => {
                let mut record = val.as_ref().clone();
                for (field, field_shape) in cols.iter() {
                    path.push_field(field);
                    match record.get_mut(field.as_str()) {
                        Some(field_val) => {
                            *field_val = check(
                                field_val.clone(),
                                field_shape,
                                engine_state,
                                removed_overlays,
                                path,
                                issues,
                            );
                        }
                        None => issues.push(ValidationIssue {
                            path: path.render(),
                            expected: format!("{field}: {field_shape}"),
                            found: "missing".into(),
                            span: value.span(),
                        }),
                    }
                    path.pop();
                }
                Value::record(record, value.span())
            }
            _ => {
                issues.push(mismatch(path, shape.to_string(), &value));
                value
            }
        },
        SyntaxShape::Table(cols) => match &value {
            Value::List { vals, .. } => {
                let row_shape = SyntaxShape::Record(cols.clone());
                let vals = vals
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        path.push_index(i);
                        let v = check(
                            row.clone(),
                            &row_shape,
                            engine_state,
                            removed_overlays,
                            path,
                            issues,
                        );
                        path.pop();
                        v
                    })
                    .collect();
                Value::list(vals, value.span())
            }
            _ => {
                issues.push(mismatch(path, shape.to_string(), &value));
                value
            }
        },
        SyntaxShape::List(elem) => match &value {
            Value::List { vals, .. } => {
                let vals = vals
                    .iter()
                    .enumerate()
                    .map(|(i, item)| {
                        path.push_index(i);
                        let v = check(
                            item.clone(),
                            elem,
                            engine_state,
                            removed_overlays,
                            path,
                            issues,
                        );
                        path.pop();
                        v
                    })
                    .collect();
                Value::list(vals, value.span())
            }
            _ => {
                issues.push(mismatch(path, shape.to_string(), &value));
                value
            }
        },
        SyntaxShape::OneOf(shapes) => {
            // First branch that validates cleanly wins.
            for branch in shapes {
                let mut branch_issues = vec![];
                let mut branch_path = path.clone();
                let checked = check(
                    value.clone(),
                    branch,
                    engine_state,
                    removed_overlays,
                    &mut branch_path,
                    &mut branch_issues,
                );
                if branch_issues.is_empty() {
                    return checked;
                }
            }
            issues.push(mismatch(path, shape.to_string(), &value));
            value
        }
        _ => {
            let expected = shape.to_type();
            if value.is_subtype_of(&expected) {
                value
            } else {
                issues.push(mismatch(path, expected.to_string(), &value));
                value
            }
        }
    }
}

/// Render the collected mismatches as one error.
fn issues_error(type_name: &str, issues: Vec<ValidationIssue>, span: Span) -> ShellError {
    let mut detail = String::new();
    for issue in &issues {
        detail.push_str(&format!(
            "\n  at {}: expected {}, found {}",
            issue.path, issue.expected, issue.found
        ));
    }
    ShellError::Generic(GenericError::new(
        format!("value does not match `{type_name}`"),
        detail,
        span,
    ))
}

fn mismatch(path: &Path, expected: impl Into<String>, value: &Value) -> ValidationIssue {
    ValidationIssue {
        path: path.render(),
        expected: expected.into(),
        found: value.get_type().to_string(),
        span: value.span(),
    }
}

/// JSONPath-style cursor into the value being validated.
#[derive(Clone)]
struct Path(Vec<String>);

impl Path {
    fn new() -> Self {
        Path(vec![])
    }

    fn push_field(&mut self, field: &str) {
        self.0.push(format!(".{field}"));
    }

    fn push_index(&mut self, index: usize) {
        self.0.push(format!("[{index}]"));
    }

    fn pop(&mut self) {
        self.0.pop();
    }

    fn render(&self) -> String {
        format!("${}", self.0.concat())
    }
}

/// Split `Name<T, U>` into `("Name", ["T", "U"])`; bare `Name` gives an
/// empty argument list. `None` when `<...>` isn't a well-formed trailing
/// argument list.
fn split_instantiation(bytes: &[u8]) -> Option<(&[u8], Vec<&[u8]>)> {
    let Some(lt) = bytes.iter().position(|&b| b == b'<') else {
        return if bytes.contains(&b'>') {
            None
        } else {
            Some((bytes, vec![]))
        };
    };
    let inner = &bytes[lt + 1..];
    if !inner.ends_with(b">") {
        return None;
    }
    let inner = &inner[..inner.len() - 1];
    let mut parts = vec![];
    let mut depth = 0i32;
    let mut start = 0;
    for (i, &b) in inner.iter().enumerate() {
        match b {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b',' if depth == 0 => {
                parts.push(trim_ascii(&inner[start..i]));
                start = i + 1;
            }
            _ => {}
        }
        if depth < 0 {
            return None;
        }
    }
    parts.push(trim_ascii(&inner[start..]));
    if depth != 0 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some((&bytes[..lt], parts))
}

fn trim_ascii(bytes: &[u8]) -> &[u8] {
    let start = bytes
        .iter()
        .position(|&b| !b.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    let end = bytes
        .iter()
        .rposition(|&b| !b.is_ascii_whitespace())
        .map(|e| e + 1)
        .unwrap_or(0);
    if end < start { &[] } else { &bytes[start..end] }
}

/// Map a type-argument name to a [`SyntaxShape`]: builtin type names map to
/// their shape; anything else is treated as a (possibly instantiated)
/// declared type name and resolved during checking.
fn name_to_shape(name: &[u8]) -> SyntaxShape {
    match name {
        b"int" => SyntaxShape::Int,
        b"float" => SyntaxShape::Float,
        b"number" => SyntaxShape::Number,
        b"string" => SyntaxShape::String,
        b"bool" => SyntaxShape::Boolean,
        b"binary" => SyntaxShape::Binary,
        b"datetime" | b"date" => SyntaxShape::DateTime,
        b"duration" => SyntaxShape::Duration,
        b"filesize" => SyntaxShape::Filesize,
        b"path" | b"filepath" => SyntaxShape::Filepath,
        b"glob" => SyntaxShape::GlobPattern,
        b"nothing" => SyntaxShape::Nothing,
        b"any" => SyntaxShape::Any,
        b"record" => SyntaxShape::Record(Default::default()),
        b"list" => SyntaxShape::List(Box::new(SyntaxShape::Any)),
        b"table" => SyntaxShape::Table(Default::default()),
        other => match split_instantiation(other) {
            // Composite spellings — instantiated names like
            // `Result<any, record<code: int>>` carry these verbatim.
            Some((b"record", args)) => {
                SyntaxShape::Record(args.iter().filter_map(parse_field).collect())
            }
            Some((b"table", args)) => {
                SyntaxShape::Table(args.iter().filter_map(parse_field).collect())
            }
            Some((b"list", args)) => SyntaxShape::List(Box::new(
                args.first()
                    .map(|a| name_to_shape(a))
                    .unwrap_or(SyntaxShape::Any),
            )),
            Some((b"oneof", args)) => {
                SyntaxShape::OneOf(args.iter().map(|a| name_to_shape(a)).collect())
            }
            Some((base, args)) => SyntaxShape::Custom(
                String::from_utf8_lossy(base).into(),
                args.iter().map(|a| name_to_shape(a)).collect(),
            ),
            None => SyntaxShape::Custom(String::from_utf8_lossy(other).into(), vec![]),
        },
    }
}

/// Split a `field: shape` spelling inside `record<...>`/`table<...>`
/// arguments into (name, shape).
fn parse_field(field: &&[u8]) -> Option<(String, SyntaxShape)> {
    let colon = field.iter().position(|&b| b == b':')?;
    let (name, field_shape) = field.split_at(colon);
    Some((
        String::from_utf8_lossy(trim_ascii(name)).into_owned(),
        name_to_shape(trim_ascii(&field_shape[1..])),
    ))
}
