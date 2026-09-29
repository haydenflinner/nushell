# Type-system experiments (local, not for upstream)

Extensions layered on top of `devdocs/enum-types.md`, living on the
`type-experiments` branch. Everything here is provisional — syntax,
semantics, and commitment level are all open.

## `--as <type>` boundary validation

`from json`, `from nuon`, `from toml`, `from yaml`, `from msgpack`, and
`open` accept `--as <name>`:

```nu
struct User { name: string, age: int }
open users.json --as list<User>
'{"kind": "some", "payload": 5}' | from json --as "Option<int>"
```

The validator (`nu_protocol::validate::validate_value`) walks the value
against the declared shape: records recurse field-by-field, lists
element-by-element, `oneof` branches try in order, and enum base records
decode into real `EnumValue`s — including nested and instantiated ones
(`record<wrapped: Option<int>>`). Mismatches aggregate with JSONPath
locations (`$.users[3].name`), capped at 5.

`--as` also *materializes*: `{kind, payload}` records become real enum
values wherever a declared enum appears in the shape. The validator is
strict — signature-style coercion does not apply at the boundary.

`open --as` collects the `from <ext>` converter's output and validates
it, so it works with custom converters too. `--raw` + `--as` errors;
`--as` with no matching converter errors.

## Union shorthand

```nu
struct Id = int | string          # desugars to oneof<int, string>
```

Top-level `|` in a `type` RHS splits into `SyntaxShape::OneOf`. Pipes
inside `record<...>`/`list<...>`/other brackets are not split. Signature
use keeps normal oneof coercion (a float can coerce into a string arm);
`--as` stays strict.

## Payload-bound match arms

A payload pattern on a qualified variant binds variables at the
declared payload type, instantiated from the scrutinee:

```nu
enum Option<T> { some: T, none }
def f [o: Option<int>] {
    match $o {
        Option.some $v => $v + 1    # $v: int — `$v + "x"` is a parse error
        Option.none => 0
    }
}
```

Record payload patterns destructure deeply (`R.err {msg: $m} => $m` is
`string`); `..$rest` collects leftovers as a record. Bindings are typed
before guards and arm bodies parse, so errors surface inside the arm.

## Recursive named types

`type` pre-registers its name before parsing the RHS, so self-reference
works in both aliases and enums:

```nu
struct Json { name: string, kids: list<Json> }
enum Tree { leaf: int, node: list<Tree> }
```

Self-references resolve lazily as `SyntaxShape::Custom(name)` — the name
is looked up at check time, so the placeholder is never observed.
Degenerate cycles (`struct A = A`) can't loop validation: `--as` tracks
the unfolding chain and reports `recursive type alias`. Forward
references to *other* undeclared types still error.

Type-level checks treat records permissively against `Custom` (enum
values ARE base records; literals into recursive positions must parse)
— deliberately *not* lists/tables, which would corrupt input-type
selection (`list<int>` would pass `custom("semver")` and `into string`
inference regresses).

## Higher-kinded parameters — `F<_>`

A parameter spelled `F<_>` is a *constructor* parameter: it may be
applied inside the body and binds to a constructor head at
instantiation.

```nu
enum Pair<F<_>> { pair: record<a: F<int>, b: F<string>> }

Pair<Option>.pair {a: (Option.some 5), b: (Option.some "hi")}  # ok
Pair<Option>.pair {a: (Option.some 5), b: (Option.some 5)}     # parse error:
                                      # expected Option<string>, found Option<int>
Pair.pair {...}                         # bare: infers F = Option from payload
```

Mechanism: `TypeDef::ctor_params` marks which entries of `params` are
constructors (ordinary arity/binding machinery is unchanged). While
parsing the RHS, `working_set.type_ctor_params` makes `F<args>` parse
to `SyntaxShape::Custom("F", args)`. `substitute` rewrites a `Custom`
whose head is bound to another `Custom`: `F → Option` turns `F<int>`
into `Option<int>`; `F → any` erases to `any`. `infer_type_args` learns
the head from the payload's outer constructor for bare `Pair.pair`.

Runtime: `enum-construct`/`enum-from-record` receive the *instantiated*
name (`Pair<Option>`), strip `<...>` for the lookup, and substitute args
into payload shapes before checking — so the wrong-constructor case and
`--as "Pair<Option>"` both check nested payloads strictly.

Known limits: only arity-1 holes (`F<_>` — no `F<_, _>`); the ctor arg
must be a declared enum/alias name or primitive spelling
(`Pair<list>` etc. not tested); `F` bare (unapplied) in the body binds
like an ordinary param to the whole constructor.
