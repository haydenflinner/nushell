use nu_test_support::fs::Stub::FileWithContentToBeTrimmed;
use nu_test_support::prelude::*;

// Named type aliases (`type Name = <shape>`)

#[test]
fn type_alias_in_signature() -> Result {
    test()
        .run("type UserId = int; def bump [id: UserId] { $id + 1 }; bump 41")
        .expect_value_eq(42)
}

#[test]
fn type_alias_record_in_signature() -> Result {
    test()
        .run("type Pt = record<x: int, y: int>; def f [p: Pt] { $p.x }; f {x: 3, y: 4}")
        .expect_value_eq(3)
}

#[test]
fn type_alias_rejects_wrong_input() -> Result {
    test()
        .run("type UserId = int; def bump [id: UserId] { $id }; bump 'abc'")
        .expect_error_code_eq("nu::parser::parse_mismatch")
}

#[test]
fn type_alias_referencing_other_type() -> Result {
    test()
        .run("type Pt = record<x: int, y: int>; type Pair = record<a: Pt, b: Pt>; def f [p: Pair] { $p.b.y }; f {a: {x: 1, y: 2}, b: {x: 3, y: 4}}")
        .expect_value_eq(4)
}

#[test]
fn type_cannot_redefine_builtin() -> Result {
    test()
        .run("type int = string")
        .expect_parse_error()
        .map(drop)
}

// Enum types (`type Name = enum<...>`)

#[test]
fn enum_unit_variant() -> Result {
    test()
        .run("type Shape = enum<circle: float, point>; Shape.point | describe")
        .expect_value_eq("Shape")
}

#[test]
fn enum_payload_variant() -> Result {
    test()
        .run("type Shape = enum<circle: float, point>; Shape.circle 2.5 | describe")
        .expect_value_eq("Shape")
}

#[test]
fn enum_base_record() -> Result {
    test()
        .run("type Shape = enum<circle: record<radius: float>, point>; (Shape.circle {radius: 2.0}).radius")
        .expect_value_eq(2.0)
}

#[test]
fn enum_serializes_as_base_record() -> Result {
    test()
        .run("type Shape = enum<circle: record<radius: float>, point>; Shape.circle {radius: 2.0} | to json --raw")
        .expect_value_eq(r#"{"kind":"circle","radius":2.0}"#)
}

#[test]
fn enum_unknown_variant_is_parse_error() -> Result {
    let err = test()
        .run("type Shape = enum<circle, point>; Shape.cirle")
        .expect_parse_error()?;
    assert!(format!("{err:?}").contains("unknown variant `cirle`"));
    Ok(())
}

#[test]
fn enum_duplicate_variant_is_error() -> Result {
    test()
        .run("type Shape = enum<a, a>")
        .expect_parse_error()
        .map(drop)
}

#[test]
fn enum_unit_variant_rejects_payload() -> Result {
    test()
        .run("type Shape = enum<point>; Shape.point 5")
        .expect_parse_error()
        .map(drop)
}

#[test]
fn enum_wrong_payload_type_is_error() -> Result {
    test()
        .run(r#"type Shape = enum<circle: float>; Shape.circle "nope""#)
        .expect_parse_error()
        .map(drop)
}

#[test]
fn enum_payload_checked_at_runtime() -> Result {
    let err = test()
        .run("type Shape = enum<circle: float>; let x: any = 'hi'; Shape.circle $x")
        .expect_shell_error()?;
    assert!(format!("{err:?}").contains("float"));
    Ok(())
}

#[test]
fn enum_in_signature_accepts_own_values() -> Result {
    test()
        .run("type Shape = enum<point>; def f [s: Shape] { $s | describe }; f Shape.point")
        .expect_value_eq("Shape")
}

// Match destructuring + exhaustiveness

#[test]
fn enum_match_destructures_payload() -> Result {
    test()
        .run("type Shape = enum<circle: record<radius: float>, point>; match (Shape.circle {radius: 2.0}) { {kind: 'circle', radius: $r} => { $r * $r }, {kind: 'point'} => 0.0 }")
        .expect_value_eq(4.0)
}

#[test]
fn enum_match_missing_variant_is_error() -> Result {
    let err = test()
        .run("type Shape = enum<a, b, c>; let s = Shape.a; match $s { {kind: 'a'} => 'a', {kind: 'b'} => 'b' }")
        .expect_parse_error()?;
    assert!(format!("{err:?}").contains("missing variants: c"));
    Ok(())
}

#[test]
fn enum_match_wildcard_covers_all() -> Result {
    test()
        .run("type Shape = enum<a, b>; let s = Shape.b; match $s { {kind: 'a'} => 'a', _ => 'other' }")
        .expect_value_eq("other")
}

#[test]
fn enum_match_or_pattern_covers() -> Result {
    test()
        .run("type Shape = enum<a, b>; let s = Shape.b; match $s { {kind: 'a'} | {kind: 'b'} => 'ab' }")
        .expect_value_eq("ab")
}

#[test]
fn match_on_plain_record_unchanged() -> Result {
    test()
        .run("match {kind: 'x'} { {kind: 'x'} => 'yes' }")
        .expect_value_eq("yes")
}

// Modules

#[test]
fn module_export_type() -> Result {
    Playground::setup("module_export_type", |dirs, sandbox| -> Result {
        sandbox.with_files(&[FileWithContentToBeTrimmed(
            "shapes.nu",
            "
                export type Shape = enum<circle: record<radius: float>, point>
                export def area [s: Shape] {
                    match $s { {kind: 'circle', radius: $r} => { $r * $r }, {kind: 'point'} => 0.0 }
                }
            ",
        )]);

        test()
            .cwd(dirs.test())
            .run("use shapes.nu *; let c = Shape.circle {radius: 2.0}; area $c")
            .expect_value_eq(4.0)
    })
}

#[test]
fn module_import_named_type() -> Result {
    Playground::setup("module_import_named_type", |dirs, sandbox| -> Result {
        sandbox.with_files(&[FileWithContentToBeTrimmed(
            "shapes.nu",
            "
                export type Shape = enum<circle: float, point>
            ",
        )]);

        test()
            .cwd(dirs.test())
            .run("use shapes.nu Shape; Shape.point | describe")
            .expect_value_eq("Shape")
    })
}
