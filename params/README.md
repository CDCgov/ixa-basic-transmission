This folder contains sets of parameters you can use to run this model.

See `src/params.rs` for the definition of each parameter.

By default, the model will use parameters default in `default.toml`.

To run another set of parameters, run this command:

```sh
cargo run -- --params params/large_pop.toml
```

Note that parameter files _extend_ the default set.
