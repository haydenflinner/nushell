# Named types and sum types for Nushell

The problem, in user words. htngr: "Nushell allows incorrect programs and throws errors only at runtime." danrasmuson: "as my scripts expand to a few hundred lines I start to find the current day limitations in the type system to be a hurdle."

This code parses today. It fails at runtime:

```nu
def foo [x: record<bar: int>] { $x.baz }
foo { bar: 2 }
```

Users tag records by hand to fake sum types. nome's example:

```nu
{kind: "circle", radius: 2.0}
```

Nothing checks the tag. Nothing checks that `match` covers every variant.

This PR adds a `type` keyword:

```nu
type UserId = int
type Shape = enum<circle: record<radius: float>, point>
```

Aliases are structural. Enums are nominal. Construct a value:

```nu
let c = Shape.circle {radius: 2.0}
$c | describe  # => Shape
```

Match on it. Record patterns bind the payload:

```nu
match $c {
    {kind: "circle", radius: $r} => { $r * $r }
    {kind: "point"} => 0.0
}
```

The parser catches three errors. `Shape.cirle` gives "unknown variant". A `match` that misses `point` gives "missing variants: point". A wrong payload type gives a type error before run time.

Types export from modules. `use shapes.nu *` brings `Shape` and its constructors.

## How it works

- `type` stores the definition in the current scope. It follows the same rules as `def` and `module`. It shadows on redeclaration. `export type` marks it for a module. `use` imports it.
- An alias stores a shape. `UserId` in a signature means `int`. The runtime check is identical.
- An enum stores a name and a variant list. An enum value carries the type name as its identity. `describe` returns `Shape`, not `record`.
- `Shape.point` is not a command lookup. The parser rewrites it to a call to an internal `enum-construct` command. The parser checks the variant name and the payload type. The command checks the payload again at run time.
- An enum value reads as a record. `Shape.circle {radius: 2.0}` behaves as `{kind: "circle", radius: 2.0}`. Cell paths, `to json`, `to nuon`, and `match` patterns all operate on this record.
- `match` checks coverage at parse time. The scrutinee is a declared enum. The parser collects the variants that the arms name. It reports the rest. A wildcard, a variable, or a guarded arm changes the count: wildcards and variables cover all variants. A guarded arm covers none, because the guard can fail.

## Paths this PR chooses

- Nominal enums, not structural unions. Two enums with identical variants stay different types.
- `kind` as the tag field. This is the convention users already apply by hand.
- Custom values as the representation. Enums get a type name and a base record. No new `Value` variant is necessary.
- Parse-time checks where the parser knows the type. Run-time checks cover the `any` cases.

## Paths that stay open

- Generics. `type result<a>` needs more machinery, but `type` does not block it.
- Traits and interfaces.
- Stricter inference. The annotation rules do not change.
- Qualified names in signatures, like `mod.Shape`.
- Payload constructors without parentheses, like `f Shape.circle 3`.
- A `Result` type for intentional errors.
- Validation at the `from json` boundary. A deserialized enum is a plain record today.

## Paths this PR closes

- Enums are not interchangeable with their base records. `{kind: "circle", radius: 2.0}` is not a `Shape`. `Shape.circle {radius: 2.0}` is.
- `kind` is a reserved key in the base record. A record payload with its own `kind` field loses that field.
- `type` is a keyword in command position. It follows the same trade-off as `def` and `let`.
- Serialization is one-way. `to json` writes the base record. `from json` does not rebuild the enum.
