## Graphify

A project knowledge graph lives in `graphify-out/`. Use it selectively for broad architecture, dependency, or cross-feature questions when a focused query can save time:

```text
graphify query "<specific architecture question>"
graphify path "<symbol A>" "<symbol B>"
graphify explain "<specific concept>"
```

Do not make Graphify the first step for localized implementation work where the relevant source files are already clear. If the first query is irrelevant or poorly scoped, stop querying and inspect the current code directly.

Graph data is a point-in-time hint. Always read currently modified or recently changed source files before editing, and treat the source as authoritative when it disagrees with the graph.

### Refresh policy: skip by default

Do **not** refresh Graphify after routine tasks. Small and medium changes should accumulate without running `graphify update .`.

Run one quick AST refresh only when:

- the user explicitly requests a Graphify refresh; or
- about 10 or more meaningful hand-written source files changed across at least two major subsystems; or
- module boundaries, major dependencies, IPC/event contracts, or a cross-cutting architecture flow changed; or
- a dedicated architecture review requires a current snapshot.

Do not count locales, generated bindings, build output, snapshots, Graphify output, formatting-only changes, documentation, or test-only churn. A feature localized to one area normally does not justify a refresh.

When justified, run `graphify update .` once at the end after implementation and validation. Never refresh more than once per task or session, never refresh to verify steering-only changes, and skip the refresh whenever the threshold is ambiguous.

Use the heavy `/graphify` semantic rebuild only when the user explicitly requests semantic extraction involving inferred document, paper, or image relationships.
