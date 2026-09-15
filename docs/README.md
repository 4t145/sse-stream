# Developer documentation

- [Migration from 0.2.6](../CHANGELOG.md): API changes and event semantics.
- [Architecture](architecture.md): parser state, buffer ownership, encoding,
  and keep-alive scheduling.
- [Benchmarking](benchmarking.md): workloads, commands, and comparison rules.

The [project README](../README.md) contains usage examples and feature options.
Generate the public API reference with `cargo doc --all-features --open`.

Keep generated benchmark samples, profiles, and experiment archives under
`target/` or outside the checkout. The documentation describes the maintained
implementation; benchmark numbers must identify the exact measured revisions.
