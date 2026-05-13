# Contributing to Heirloom

Thanks for considering a contribution. Heirloom is a small, opinionated project that's deliberately easy to extend.

## The highest-leverage contribution: a new ingester

An ingester turns one external source into Heirloom memories. They're tiny — typically 50-150 lines — and each one makes Heirloom dramatically more useful for everyone who uses that source.

### Step 1 — Pick a target

Look at the [wanted list](https://github.com/MayonaiseLover/heirloom/issues?q=is%3Aissue+label%3Aingester). Comment on the issue to claim it. If you want to add something not listed, open an issue first to check fit.

### Step 2 — Copy the template

`crates/ingesters/heirloom-fs` is the reference implementation. Copy it:

```bash
cp -r crates/ingesters/heirloom-fs crates/ingesters/heirloom-yoursource
```

Then update `Cargo.toml` and rename the struct.

### Step 3 — Implement the trait

```rust
#[async_trait]
impl Ingester for YourIngester {
    fn name(&self) -> &'static str { "yoursource" }
    fn description(&self) -> &'static str { "Reads memories from Your Source." }

    async fn ingest(&self, ctx: &IngestContext) -> anyhow::Result<IngestReport> {
        // 1. Read from the source.
        // 2. Turn each unit into a Memory with appropriate metadata.
        // 3. Use ctx.store.add_many(&batch) for efficiency.
        // 4. Honor ctx.since for incremental runs.
        todo!()
    }
}
```

### Step 4 — Wire it up

Add your ingester to `crates/heirloom-cli/Cargo.toml` and to the dispatch in `cmd_ingest` in `crates/heirloom-cli/src/main.rs`.

### Step 5 — Write tests

At minimum:
- One test that produces expected memories from a fixture
- One test that confirms a second run is idempotent (no duplicates)

Use `tempfile::TempDir` and `Store::in_memory()`. See `heirloom-fs` tests.

### Step 6 — Open a PR

Include:
- A one-line description for the README ingesters table
- A note about what to extract (e.g. "page text via readability for entries with dwell > 30s")
- Any new config options users need to set in `~/.heirloom/config.toml`

## Other ways to contribute

- **Docs.** README clarity, quickstart issues, typos — all welcome.
- **Bug reports.** Reproduction steps matter more than anything.
- **Performance.** Cold-start `heirloom search` should return in under 500ms on 10k memories. If yours doesn't, that's a bug.
- **Tests.** We aim for every public function in `heirloom-core` to have a doc-test or unit test.

## Development setup

Requires Rust 1.75+.

```bash
git clone https://github.com/MayonaiseLover/heirloom
cd heirloom
cargo test                 # run everything
cargo run -- init          # try the CLI
cargo run -- search "foo"
```

## Code style

- `cargo fmt` and `cargo clippy -- -D warnings` must pass.
- No `unwrap()` or `panic!()` outside tests. Use `anyhow::Result` in binaries, `thiserror` in libraries.
- Public items in libraries need doc comments. One example each, where it makes sense.
- Keep dependencies minimal. If you're adding a crate with more than a handful of transitive deps, justify it in the PR description.

## Code of conduct

This project follows the [Contributor Covenant](CODE_OF_CONDUCT.md). Be kind.

## License

By contributing, you agree your work is licensed under the [MIT License](LICENSE).
