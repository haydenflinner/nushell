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

Not in this version: generics, traits, type inference. htngr asked for `type result<a>`. That stays future work.
