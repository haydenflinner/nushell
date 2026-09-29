# Named Types and Sum Types for Nushell

A design proposal and prototype for <https://github.com/nushell/nushell/issues/11108>.

## The problem space

From the issue thread and its comments, the concrete user pains are:

1. **Refactoring fear.** `def f [x: record<bar: int>] { $x.baz }` — a typo'd or
   renamed field compiles fine and explodes at runtime. Once a script passes a
   few hundred lines, people reach for Python or Go instead.
2. **Record types can't be named.** `record<foo: string, bar: int>` gets
   copy-pasted across signatures and silently drifts.
3. **Mutually exclusive shapes.** Users encode "circle vs rectangle" as
   `{kind: "circle"}` by convention; nothing checks the tag set, nothing checks
   `match` coverage, and `{kind: "teapot"}` slips through.
4. **Boundaries produce `any`.** Data entering via `from json`/`open` is
   unshaped; the typed-pipeline story collapses at exactly the point real data
   arrives.
5. **Signatures can't express intent.** `f: closure` and "this function
   intentionally throws" are inexpressible.

Items 1–3 are addressable with a backwards-compatible additive change. Items
4–5 (boundary validation, generics, inference, error types) are larger and are
explicitly left to follow-up work. Maintainer feedback on the issue already
ranked named types and sum types as the safe, deferrable additions — while
strictness of inference is the urgent 1.0 question. This proposal takes the
safe slice first.

## Proposal

### `type` — named type declarations

```nu
type Point = record<x: int, y: int>

def distance [a: Point, b: Point]: nothing -> int {
    (($a.x - $b.x) ** 2 + ($a.y - $b.y) ** 2) | math sqrt | math floor
}
```

- `type` is a new parser keyword, like `alias`, `def`, `const`.
- The right-hand side is any existing type expression: `record<...>`,
  `table<...>`, `list<int>`, `oneof<int, string>`, `int`, or another named type.
- Aliases are **structural**: `Point` is interchangeable with
  `record<x: int, y: int>` anywhere a type is expected — signatures, `let`
  annotations, `desc` variables. No nominal wrapper, no conversion ceremony.
- Alias bodies can reference previously declared named types:
  `type Pair = record<a: Point, b: Point>`.
- Redeclaring a name in the same scope shadows it, matching `def`/`alias`.
- Unknown names in type position keep producing `ParseError::UnknownType` —
  named types only make the lookup succeed.

This alone closes most of pains 1–2: `def f [x: Point]` enforces the record
structure at call time today (runtime annotation enforcement), and signatures
show the readable name instead of an expanded record.

### `enum<...>` — nominal sum types

```nu
type Shape = enum<
    circle: record<radius: float>,
    rect: record<width: float, height: float>,
    point,                      # unit variant, no payload
>
```

- `enum<...>` is only legal as the right-hand side of `type`. The declared
  name is the nominal identity: `Shape` ≠ `record<kind: string>` and ≠ any
  other enum, even a structurally identical one.
- Variants may carry a payload of any type (`int`, `record<...>`, `string`,
  `list<...>`) or no payload at all.
- Values are constructed through **variant constructors** — qualified command
  names resolved at parse time:

```nu
let s = Shape.circle {radius: 3}
let p = Shape.point
```

  - `Shape.cirle` (typo) is a **parse error** listing the declared variants —
    it does not fall through to an external command.
  - Wrong payload type is a parse error when the argument's type is known, and
    a runtime error otherwise (`$x: any` payloads are checked when constructed).
  - Missing/extra payload arguments are parse errors.

- Values of an enum describe themselves nominally:
  `(Shape.point | describe) == "Shape"`.
- An enum value converts to its **base record** for display, serialization,
  cell-path access, and pattern matching:

  - `Shape.circle {radius: 3}` behaves as `{kind: "circle", radius: 3}` —
    record payloads are spread into the record.
  - `Shape.ok 42` for `ok: int` behaves as `{kind: "ok", payload: 42}` —
    non-record payloads live under `payload`.
  - `Shape.point` behaves as `{kind: "point"}`.
  - This means `to json`/`to nuon`, `table` display, `$s.radius`, `get`, and
    `select` all work on enum values with no extra machinery. Round-tripping
    through serialization produces the plain record (nominal identity is lost
    across a serialize/deserialize boundary — same trade-off as custom values
    like `semver`).

### `match` — destructuring and exhaustiveness

Existing record patterns already destructure the base record form:

```nu
match $shape {
    {kind: "point"} => "origin"
    {kind: "circle", radius: $r} => ($r * $r * 3.14159)
    {kind: "rect", width: $w, height: $h} => ($w * $h)
}
```

Two new checks layer on top, applied only when the scrutinee's type is a known
declared enum (a `Type::Custom` that resolves to an `enum` definition):

- **Exhaustiveness (parse error).** If the scrutinee is `Shape` and no arm
  covers variant `rect`, the parse fails with the missing variants listed.
  Wildcards (`_`), variable bindings, `{kind: $k}`, and or-patterns containing
  them count as covering everything. A guarded arm (`{kind: "circle"} if ...`)
  does not contribute coverage — the guard can fail at runtime.
- **Unknown variants (parse error).** `{kind: "octagon"}` when `octagon` was
  never declared is flagged at parse time instead of silently never matching.

`match` on non-enum values is completely unchanged.

## How it maps to the code

- `OverlayFrame` gains a `types` map alongside `decls`/`modules`; types
  participate in scopes, overlays, and the REPL's permanent-state merge exactly
  like modules (no per-item visibility in v1).
- `StateWorkingSet::find_type_name` mirrors `find_module`; `parse_shape_name`
  consults it when the built-in table misses.
- Two new `SyntaxShape` variants:
  - `SyntaxShape::Named(name, inner)` — an alias; displays as `name`,
    `to_type()` resolves to `inner`'s type.
  - `SyntaxShape::Custom(name)` — a declared enum; `to_type()` is
    `Type::Custom(name)`, which is already nominal (`Custom == Custom` iff
    names match).
- Enum values are `Value::Custom` wrapping an `EnumValue` (`type_name()` is
  the enum name — the `semver` precedent). `to_base_value()` yields the
  `{kind: ..., ...}` record.
- `enum-construct` is an internal command in `nu-cmd-lang`
  (`enum-construct <path> [payload]`). The parser rewrites `Shape.circle args`
  — when `Shape` resolves to an enum — into a call to it, validating the
  variant and payload at parse time; the command re-validates the payload at
  runtime when the type def is visible in `EngineState`.
- The pattern matcher resolves `Value::Custom` through `to_base_value()` for
  record/list patterns — any custom value with a record-shaped base becomes
  matchable, enums included.
- Exhaustiveness runs in `parse_match_block_expression`, fed by the scrutinee
  positional's type (mirroring how `def`'s block parameter already peeks at the
  previous argument).

## Compatibility

- Every construct is additive. Existing scripts contain no `type` keyword
  (it's a new reserved word in command position only — `type` as a bare word
  argument to a command is unaffected, same as `def`).
- Enum coverage only activates for scrutinees typed as a declared enum; a
  plain `{kind: "x"}` record behaves exactly as before.
- `x: Point` annotations enforce at runtime the same way existing structural
  types do (`enforce-runtime-annotations` rules apply as-is).

## Deliberately out of scope (for this slice)

- Generics / parametric polymorphism (`result<a>`, `closure<a,b>`).
- Type inference beyond what the parser already does.
- Traits / typeclasses.
- Exhaustiveness for structural `oneof<...>` (only declared enums have a closed
  variant set).
- First-class types as runtime values.
- Boundary validation of `open`/`from` data beyond what signatures already do.
- `Result`-style error typing in signatures.

Each is a plausible follow-up; none blocks this slice.

## Known limitations of the prototype

- `use mod [TypeName]` and `use mod *` bring types in; bare `use mod` does not
  yet qualify type names (there is no `mod Type` spelling in signatures yet).
- A `type` declared inside a `def` body is scoped to that body at parse time;
  the runtime can't re-validate payloads for such types (parse-time checks
  still apply).
- Serializing an enum value produces its base record; deserializing does not
  restore the nominal type.
- `kind` is a reserved discriminator key: record payloads containing their own
  `kind` field are shadowed in the base record.
