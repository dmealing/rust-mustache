Mustache
========

> **This is a maintained fork** of [nickel-org/rust-mustache](https://github.com/nickel-org/rust-mustache),
> published as the `mustache-ng` crate. The library is still imported as
> `mustache`, so existing `use mustache::...` code keeps working. Many thanks to
> the nickel-org maintainers and the original author for the foundation.

Inspired by [ctemplate][1] and [et][2], [Mustache][3] is a framework-agnostic way
to render logic-free views.

As ctemplates says, "It emphasizes separating logic from presentation: it is
impossible to embed application logic in this template language."

rust-mustache is a rust implementation of Mustache.

## Documentation

The different Mustache tags are documented at [mustache(5)][4].

Documentation for this library is [here][5].

## Spec support

This crate is tested against the official [Mustache spec](https://github.com/mustache/spec).
Supported: the required modules (comments, delimiters, interpolation, inverted
sections, partials, sections) and the optional lambdas module. Not supported:
the optional inheritance and dynamic names modules. Their spec tests are kept
in the suite but ignored (`cargo test -- --ignored` runs them).

Behaviour changes from `mustache` 0.9: standalone tags are now stripped
correctly when they follow another standalone tag, and a boolean interpolated
with `{{flag}}` renders as `true` or `false` (it used to render nothing).

## Install

Install it through Cargo!

```toml
[dependencies]
mustache-ng = "0.10"
serde = { version = "1", features = ["derive"] } # for the example below
```

# Basic example

```rust
use std::io;
use mustache::MapBuilder;
use serde::Serialize;

#[derive(Serialize)]
struct Planet {
    name: String,
}

fn main() {
    // First the string needs to be compiled.
    let template = mustache::compile_str("hello {{name}}").unwrap();

    // You can either use an encodable type to print "hello Mercury".
    let planet = Planet { name: "Mercury".into() };
    template.render(&mut io::stdout(), &planet).unwrap();
    println!("");

    // ... or you can use a builder to print "hello Venus".
    let data = MapBuilder::new()
        .insert_str("name", "Venus")
        .build();

    template.render_data(&mut io::stdout(), &data).unwrap();
    println!("");

    // ... you can even use closures.
    let mut planets = vec!("Jupiter", "Mars", "Earth");

    let data = MapBuilder::new()
        .insert_fn("name", move |_| {
            planets.pop().unwrap().into()
        })
        .build();

    // prints "hello Earth"
    template.render_data(&mut io::stdout(), &data).unwrap();
    println!("");

    // prints "hello Mars"
    template.render_data(&mut io::stdout(), &data).unwrap();
    println!("");

    // prints "hello Jupiter"
    template.render_data(&mut io::stdout(), &data).unwrap();
    println!("");
}
```

## Testing

Simply clone and run:

```bash
# If you want to run the test cases, you'll need the spec as well.
git submodule init
git submodule update

cargo test

```

## Releasing

If cutting a new release, please follow something along the lines of the below:

```bash
# Assuming master is the current release commit, ideally it will be a commit
# announcing the release and the only change would be the version number.

# Ensure everything looks good
cargo publish --dry-run

# Actually publish
cargo publish

# Tag the release, prefix it with 'v' for easy tag searching, i.e. git tag --list 'v*'
git tag vX.Y.Z

git push --tags origin master
```

[1]: http://code.google.com/p/google-ctemplate/
[2]: http://www.ivan.fomichev.name/2008/05/erlang-template-engine-prototype.html
[3]: https://mustache.github.io/
[4]: https://mustache.github.io/mustache.5.html
[5]: https://docs.rs/mustache-ng

# License

See LICENSE File
