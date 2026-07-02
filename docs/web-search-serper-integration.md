# Web Search: Serper Integration & Verification Guide

> **Update (Phase 2.0):** Firecrawl was removed entirely and full-page
> fetching/scraping was dropped — web search is now **snippet-only**. The
> provider set is **Serper** (default), **Brave**, **Tavily**, **Exa**, and
> **SerpAPI**, all single-key with a free tier. The scrape stage, the Firecrawl
> credit guard, and the `fetch_content`/`daily_credit_budget` UI are gone (the
> settings/commands remain as deprecated no-ops for back-compat). Sections below
> that describe Firecrawl scraping or the credit budget are **historical**; the
> dispatch shape, file map, and recipe still apply to adding any snippet
> provider. See [`PROGRESS.md`](./PROGRESS.md) Phase 2.0.

This is the **how** companion to
[`web-search-serper-migration.md`](./web-search-serper-migration.md) (the why).
It documents exactly how Serper is wired into the assistant's web search, what to
check, and how to verify a change to this area without breaking the build.

> **Read this before touching web search.** Treat the checklist as mandatory and
> the "Be thorough" section as a standing instruction: a provider touches the
> Rust backend, the settings schema, the generated bindings, the React UI, the
> i18n strings, the secret store, the README, and the tests. Miss one and you
> ship a half-wired provider.

---

## 1. Serper API reference (what we call)

- **Endpoint:** `POST https://google.serper.dev/search`
- **Auth:** header `X-API-KEY: <key>` (get a key + free credits at
  <https://serper.dev/>).
- **Request body (JSON):**
  - `q` (string) — the query.
  - `num` (int) — results to return (we clamp to 1–20). **Billing is 10 results
    = 1 credit**, 20–100 = 2 credits.
  - `tbs` (string, optional) — Google's time filter, passed straight through
    (e.g. `qdr:d`, `qdr:w`, `sbd:1,qdr:w`). Built by `build_tbs(...)`.
  - (Available but not currently sent: `gl`, `hl`, `page`, `location`.)
- **Response (fields we parse):**
  - `answerBox` — Google's direct answer (`answer`/`snippet`, `title`, `link`).
    Highest value for quick facts; added first.
  - `knowledgeGraph` — entity facts (`title`, `description`, `website`).
  - `topStories[]` — fresh news (`title`, `link`, `source`, `date`); only pulled
    when the planner flags the turn as news-y (`include_news`).
  - `organic[]` — web results (`title`, `link`, `snippet`, optional `date`).

Serper returns **snippets only** (no full page content), which is exactly the
snippet-first behavior we want. Full-page fetching was removed in Phase 2.0; if a
future feature ever needs it again, add a dedicated extractor rather than
scraping inside a SERP provider.

---

## 2. Where it's wired (file-by-file map)

| File                                                      | What lives here                                                                                                                                                                                                                                                                                               |
| --------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `src-tauri/src/web_search.rs`                             | `search_serper(api_key, query, max_results, tbs, include_news) -> Result<Vec<Candidate>, String>` and the provider dispatch in `snippet_search`. Reuses `push_candidate`, `http_client`, `truncate_chars`, `build_tbs`.                                                                                       |
| `src-tauri/src/settings.rs`                               | `default_assistant_web_search_provider()` → `"serper"`; `default_web_search_api_keys()` seeds `serper`/`brave`/`tavily`/`exa`/`serpapi`; `ensure_assistant_defaults` validates `serper \| brave \| tavily \| exa \| serpapi` (legacy `firecrawl`/`duckduckgo` migrate to `serper`) and seeds those key slots. |
| `src-tauri/src/commands/assistant.rs`                     | `set_assistant_web_search_provider` and `set_assistant_web_search_api_key` validators accept `serper \| brave \| tavily \| exa \| serpapi`.                                                                                                                                                                   |
| `src-tauri/src/secret_store.rs`                           | `account_web_search(provider_id)` is generic → the key is stored under `web_search:serper` automatically. No change needed.                                                                                                                                                                                   |
| `src/components/settings/assistant/AssistantSettings.tsx` | Default provider `"serper"`, `webSearchNeedsKey` includes `serper`, and the provider `Dropdown` lists Serper / Firecrawl / Brave.                                                                                                                                                                             |
| `src/i18n/locales/en/translation.json`                    | `settings.assistant.webSearch.providers.serper` plus updated `providerDescription` / `apiKeyDescription` / `fetchContentDescription`.                                                                                                                                                                         |
| `src/bindings.ts`                                         | Auto-generated by tauri-specta. The field stays `string`, so no functional change; regenerates on the next `bun run tauri dev`/`build`.                                                                                                                                                                       |
| `README.md`                                               | Web-search bullets, feature table, getting-started, and privacy lines.                                                                                                                                                                                                                                        |

### Dispatch shape

```rust
match provider {
    "brave"   => { /* search_brave(...) -> map(result_to_candidate) */ }
    "tavily"  => { /* search_tavily(...) */ }
    "exa"     => { /* search_exa(...) */ }
    "serpapi" => { /* search_serpapi(...) */ }
    // "serper" is the default; any unknown/legacy value (incl. the removed
    // "firecrawl"/"duckduckgo") also routes here so old settings keep working.
    _ => { /* search_serper(...) */ }
}
```

---

## 3. Adding or changing a search provider (the general recipe)

1. **Backend fn** in `web_search.rs` returning `Vec<Candidate>` (or
   `Vec<SearchResult>` + `result_to_candidate`). Reuse `push_candidate` so
   titles/snippets are cleaned and bounded, and set `from_news` correctly.
2. **Dispatch arm** in `snippet_search` reading the key from
   `settings.web_search_api_keys.get("<id>")`.
3. **Settings**: add the id to the provider validator and to
   `default_web_search_api_keys` + the key-seeding loop in
   `ensure_assistant_defaults`.
4. **Commands**: add the id to both validators in `commands/assistant.rs`.
5. **UI**: add a `Dropdown` option and (if keyed) include it in
   `webSearchNeedsKey`.
6. **i18n**: add `providers.<id>` to `en/translation.json` (and ideally the other
   locales).
7. **Docs**: update the README and these two files.
8. **Verify** with the checklist below.

---

## 4. Verification checklist (run all of these)

Commands that worked in this environment (Windows / PowerShell; `bunx` and
`tail` were not available, so call tools directly):

- [ ] **Format / syntax**
      `cargo fmt --manifest-path src-tauri/Cargo.toml`
- [ ] **Type-check incl. tests** (this is what catches a removed-fn referenced by
      a test) `cargo check --manifest-path src-tauri/Cargo.toml --tests`
- [ ] **Frontend types**
      `node node_modules/typescript/bin/tsc --noEmit -p tsconfig.json`
- [ ] **Lint the changed component (i18n rule)**
      `node node_modules/eslint/bin/eslint.js src/components/settings/assistant/AssistantSettings.tsx`
- [ ] **No dangling references** — grep the repo for the old provider and any
      helper you removed, e.g.
      `duckduckgo|ddg|parse_duckduckgo_html|decode_ddg_url|percent_decode|hex_val`.
      Search **`.rs`, `.ts`, `.tsx`, `.json`, `.md`** — not just Rust.
- [ ] **Bindings** — regenerate by running the app once
      (`bun run tauri dev`) if you changed a command signature or a typed enum.
      A `String` field needs no regen.
- [ ] **Manual smoke test** — Settings → Assistant → Web Search → enable, pick
      **Serper**, paste a key, click **Test search**. Confirm results come back
      and the key persists across restart (stored at `web_search:serper`).
- [ ] **Migration** — a profile with `assistant_web_search_provider = "duckduckgo"`
      loads and is silently moved to `serper` (validator rejects the old value).

Last run result for this change: `cargo fmt` ✓, `cargo check --tests` ✓ (one
pre-existing unrelated warning in `helpers/clamshell.rs`), `tsc --noEmit` ✓,
ESLint ✓, grep ✓.

---

## 5. Edge cases to confirm

- **Missing key.** `search_serper` returns a clear error; `search_with_plan`
  logs and **degrades gracefully** (the turn answers without web context). Verify
  it doesn't break the turn.
- **Rate limit / non-200.** Non-success status returns an error string
  (truncated). Confirm it's swallowed per-query, not fatal.
- **Empty results.** Some queries return no `organic`/`answerBox`. The pipeline
  should proceed with whatever candidates exist (possibly zero).
- **News vs web.** `include_news` is driven by the planner; `topStories` are only
  added then, and flagged `from_news = true` for the recency boost.
- **Freshness.** `tbs` comes from `build_tbs(...)` and is passed through verbatim;
  Serper/Google accept `qdr:*` and `sbd:1`. (Unlike Brave, no separate mapping.)
- **Dedupe.** Duplicate URLs across the answer box / knowledge graph / organic
  are de-duplicated downstream by `round_robin_merge` (keyed on URL).

---

## 6. Be thorough — check the things that aren't in front of you

This is the standing instruction the rest of the guide builds toward: **don't
assume; verify, and think laterally about what else a provider change touches.**

- **Grep across every file type**, not just the file you edited. A provider id is
  a string that appears in Rust, TS, generated bindings, JSON locales, the
  README, and tests. The DuckDuckGo removal was only "done" after a repo-wide
  grep found a **leftover test** (`parse_duckduckgo_html_extracts_pairs`) and a
  stale comment that `cargo fmt` happily ignored.
- **Run `cargo check --tests`, not just `cargo check`.** A removed function can
  still be referenced by a test and compile-fail only under `--tests`.
- **Other locales.** `de/es/fr/ja/ru/zh` still contain `providers.duckduckgo` and
  lack `providers.serper`; they fall back to English at runtime (no crash), but
  they should be translated — see
  [`CONTRIBUTING_TRANSLATIONS.md`](../CONTRIBUTING_TRANSLATIONS.md). Treat this as
  a follow-up task, not "done".
- **UI conditionals that are provider-specific.** The Firecrawl-only "daily
  credit budget" field and the "Read full pages" toggle are gated on the
  provider/feature; make sure a new provider shows the right controls (Serper has
  no per-page credit budget concept).
- **Generated files.** Don't hand-edit `src/bindings.ts` to "fix" comments;
  regenerate it from the Rust source instead.
- **Secret storage.** Confirm the key actually lands in the OS keychain under the
  expected `web_search:<id>` account and is blanked from `settings_store.json`.
- **Docs stay truthful.** If behavior changes (default provider, "works with no
  key", "snippets only"), update the README and these docs in the same change so
  they never contradict the code.
- **When in doubt, test the real path.** The "Test search" button exercises the
  exact dispatch the assistant uses; prefer it over reasoning about the code.

### Definition of done

A provider change is done when: backend + tests type-check, frontend types and
lint pass, no dangling references remain in any file type, the manual Test-search
works and the key persists, existing profiles migrate cleanly, and the docs +
(at least English) i18n strings match the new behavior.
