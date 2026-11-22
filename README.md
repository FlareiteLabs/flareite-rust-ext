# Rust

Highlighting for Rust files (`.rs`), and
[rust-analyzer](https://rust-analyzer.github.io) for diagnostics, hover, go to definition,
references, symbols, completion, formatting, and code actions.

## Setting up

Install rust-analyzer so that `rust-analyzer` is on the `PATH`:

```sh
rustup component add rust-analyzer
```

To use another copy, set its path:

```toml
[lsp.servers.rust-analyzer]
command = "C:/tools/rust-analyzer.exe"
```

## Every workspace in the folder

rust-analyzer loads the Cargo workspace of the folder you open. A package that workspace
excludes is a workspace of its own, which rust-analyzer wouldn't load, such as a kernel built for
another target:

```toml
[workspace]
members = ["xtask"]
exclude = ["kernel"]
```

So when you open a Rust file, the extension asks Cargo which workspace it's in
(`cargo locate-project --workspace`, which only reads manifests), and gives rust-analyzer any
workspace it wouldn't load as another workspace folder. Go to definition, hover, and completion
then work in those packages too. This is why the extension asks to read documents (to hear which
files open) and to run `cargo`.

## More settings

rust-analyzer's settings (as VS Code names them) go in `settings.toml`, or in a folder's
`.flareite/settings.toml`:

```toml
[lsp.servers.rust-analyzer.settings]
"rust-analyzer.check.command" = "clippy"
```
