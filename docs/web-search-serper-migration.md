# Web Search: Moving to Serper (and dropping DuckDuckGo)

Status: superseded by Phase 2.0 (see banner). Owner: assistant web-search pipeline.

> **Update (Phase 2.0):** the direction in this doc held up — snippet-first won —
> and was taken further. **Firecrawl was removed** (along with all full-page
> fetching/scraping and the credit guard), and the provider set grew to
> **Serper** (default), **Brave**, **Tavily**, **Exa**, and **SerpAPI** — exactly
> the snippet-first + reranker-friendly, multi-provider direction the roadmap
> below anticipated (Tavily/Exa/SearXNG were listed as future options). The
> Firecrawl-specific cost/credit discussion here is now historical. See
> [`PROGRESS.md`](./PROGRESS.md) Phase 2.0 and
> [`web-search-serper-integration.md`](./web-search-serper-integration.md).

This document explains **why** the assistant's web search now defaults to
[Serper](https://serper.dev/), why **DuckDuckGo was removed**, and what the
**future plan** is. It is the rationale companion to
[`web-search-serper-integration.md`](./web-search-serper-integration.md), which
covers the how and the verification checklist.

---

## TL;DR

- The old default (DuckDuckGo, keyless HTML scraping) was slow-ish, brittle, and
  frequently returned nothing — so the model answered from stale training data
  and got current facts wrong (e.g. naming an out-of-date prime minister).
- The bigger latency/quality problem was the **method**, not the model: a
  planner round-trip, then **scraping full pages**, then cropping the **first N
  characters** of each page and stuffing ~14–24k characters into a small model.
- Serper gives **fast (~1–2 s), Google-quality snippets** with an answer box and
  knowledge graph. That is "snippet-first" retrieval, which is what the fast
  assistants actually do — and it fits our local rerank step.
- Serper is **cheap and has a generous free tier**. The earlier worry that "one
  search burns 10–20 credits" was a misread of the old Firecrawl scrape cost.
  On Serper, **10 results = 1 credit**.
- DuckDuckGo added no value once a real provider is the default, so it was
  removed to keep the provider set small and the code clean.

---

## Background: what was actually wrong

The assistant's web search ran one retrieval pass shaped like this:

1. A **planner** LLM call to decide whether to search and rewrite the query.
2. A snippet search.
3. **Scraping the full content** of the top few results (Firecrawl path).
4. A second (answer) LLM call with a large block of scraped page text prepended.

Two things hurt the most:

- **Cropping the top N characters** of a scraped page usually captures
  navigation, infoboxes, and boilerplate — not the sentence that answers the
  question. The correct answer is very often already in the one-line **snippet**.
- **Too much context.** Feeding a small local model a big, noisy blob is slower
  (more to read before it can start) and _less_ accurate (the key sentence gets
  lost), and it costs more tokens.

On top of that, the **default provider was DuckDuckGo**, a keyless HTML endpoint
parsed with regexes. It is low quality, easy to rate-limit, and often returns an
empty page — at which point the turn answered with no web context at all, from
the model's memory. That is the classic "confident but stale" failure.

### What the fast assistants do (and why Serper fits)

This direction mirrors how the well-known answer engines work. Paraphrased from
public write-ups (content rephrased for licensing compliance; sources linked):

- **ChatGPT search** decomposes a prompt into a handful of parallel sub-queries
  ("query fan-out"), leans heavily on **Bing's index**, and also runs its own
  crawler (OAI-SearchBot). It reads results and synthesizes a cited answer.
  ([OpenAI crawler docs](https://developers.openai.com/api/docs/bots),
  [SearchGPT/Bing overlap study](https://www.seerinteractive.com/insights/87-percent-of-searchgpt-citations-match-bings-top-results))
- **Perplexity** runs a multi-stage RAG pipeline over its **own index**: hybrid
  retrieval (keyword + semantic), a multi-layer **reranker**, and citations wired
  into the prompt _before_ the model writes. Its fast model (Sonar) runs on
  Cerebras hardware at roughly 1,200 tokens/second.
  ([pipeline breakdown](https://ziptie.dev/blog/how-perplexity-ai-answers-work/),
  [Sonar + Cerebras](https://www.perplexity.ai/hub/blog/meet-new-sonar))

The lesson that matters for us: **retrieval quality (and reranking), not the
LLM, is the bottleneck**, and these systems answer from **ranked snippets/passages**,
not from full-page dumps. Note also that the **Bing Search API was retired on
August 11, 2025** ([Microsoft](https://learn.microsoft.com/en-us/lifecycle/announcements/bing-search-api-retirement)),
so "just use Bing like ChatGPT" is not an option — a third-party SERP API such as
Serper is the practical equivalent.

---

## Why Serper

- **Fast.** Roughly 1–2 second responses for Google SERP JSON.
  ([Serper](https://serper.dev/))
- **Snippet-first, which is what we want.** Most factual questions are answered
  by the snippet, the **answer box**, or the **knowledge graph** that Serper
  returns — no page fetch required. This is the single biggest speed win.
- **Cheap with a real free tier.** ~2,500 free credits, and pricing roughly
  $0.30–$1.00 per 1,000 queries; **10 results = 1 credit**.
  ([pricing summary](https://www.buildmvpfast.com/tools/api-pricing-estimator/serper))
  Even with query fan-out (2–3 sub-queries per question), personal use rarely
  leaves the free tier.
- **Drops straight into the existing pipeline.** Our snippet → rerank → answer
  flow already exists; Serper is just a better snippet source than DuckDuckGo.

### Cost reality (correcting the earlier worry)

| Concern                         | Reality                                                                                                          |
| ------------------------------- | ---------------------------------------------------------------------------------------------------------------- |
| "One search uses 10–20 credits" | That was the old **Firecrawl** path (search = 2 credits + 1/scraped page). On Serper, **10 results = 1 credit**. |
| "$1 / 1,000 is expensive"       | You run ~1,000 searches to spend ~$1; it drops toward $0.30/1k at volume.                                        |
| "Free tier won't last"          | ~2,500 free credits/month; a personal voice assistant rarely exceeds it.                                         |

---

## Why remove DuckDuckGo

- It was a **keyless HTML-scraping** endpoint parsed with brittle regexes
  (`result__a` / `result__snippet`, redirect URL decoding, percent-decoding).
- It is **low quality** and **easily rate-limited**, frequently returning an
  empty page — which silently degraded the turn to a no-context answer.
- Once Serper is the default, DuckDuckGo added **no value** and only widened the
  provider surface (more code, more edge cases). Removing it keeps the set tight:
  **Serper** (default), **Firecrawl** (optional full-page reads), **Brave**
  (independent index).

Existing users whose settings still say `duckduckgo` are **auto-migrated** to the
new default on load (the provider validator no longer accepts it), and any
unknown/legacy provider value routes to Serper at runtime, so nothing breaks.

---

## What changed in this iteration

- **New `search_serper`** in `src-tauri/src/web_search.rs`: calls
  `POST https://google.serper.dev/search` (header `X-API-KEY`, body `{q, num, tbs}`)
  and parses the **answer box**, **knowledge graph**, **organic** results, and —
  when the turn is news-y — **top stories**. It reuses the shared
  `push_candidate` / `http_client` / `truncate_chars` helpers and flags news
  results so the local recency boost applies.
- **DuckDuckGo fully removed**: the search function, its `tbs→df` mapping, the
  HTML-parsing regexes/parser, the URL/percent decoders, and the related tests.
- **Defaults & validation** (`settings.rs`, `commands/assistant.rs`): default
  provider is now `serper`; the secret-key map seeds `serper`; provider and
  api-key validators accept `serper | firecrawl | brave`.
- **UI** (`AssistantSettings.tsx`, `en/translation.json`): the provider dropdown
  lists Serper / Firecrawl / Brave, Serper requires an API key, and the copy was
  updated. Other locales fall back to English until translated.
- **README** updated to describe Serper instead of DuckDuckGo.

See the integration doc for the exact file-by-file map and verification steps.

---

## Future plan / roadmap

Ordered roughly by impact-to-effort. Serper is step 0; the rest turn "fast
snippets" into "fast _and_ well-grounded".

1. **Snippet-first by default.** Keep full-page scraping (Firecrawl) as an
   opt-in for hard questions, not the default path. Answer from snippets +
   answer box for the common case.
2. **Add a real reranker.** This is the missing piece versus Perplexity. A
   cross-encoder rerank (hosted [Cohere Rerank](https://cohere.com/rerank) or a
   local model such as `bge-reranker`) over the merged candidates, keeping only
   the top few passages, both improves accuracy and shrinks the prompt.
   ([why rerankers help](https://www.pinecone.io/learn/series/rag/rerankers/))
3. **Fail-safe grounding.** If the best reranked score is weak, say "I couldn't
   find good sources" or re-query — **never** silently fall back to training-data
   memory. This is the fix for the "confident but stale" answers.
4. **Trim the context budget.** Feed ~5 clean snippets/passages (~1.5–3k chars),
   not 14–24k characters of page text.
5. **SearXNG provider (free, self-hosted).** For users who want fully local and
   free search, add a `searxng` provider pointing at a local instance (JSON
   endpoint, no API key). Fits the project's offline-friendly philosophy.
6. **Faster answer generation (optional).** Route the answer step to a fast
   inference provider (Groq / Cerebras) for a near-instant feel, independent of
   the search provider.
7. **Better fan-out + citations.** Tune sub-query generation and consider
   lightweight per-source citations in the panel.

### Provider landscape (for future choices)

| Provider                     | Type                      | Notes                                                      |
| ---------------------------- | ------------------------- | ---------------------------------------------------------- |
| **Serper** (current default) | Google SERP               | Fast, cheap, snippet-first, generous free tier             |
| **Firecrawl**                | Search + full-page scrape | Best when the snippet truly isn't enough; costs more       |
| **Brave**                    | Independent index         | Privacy-first; `$5/1k`, no standalone free tier as of 2026 |
| **SearXNG** (planned)        | Self-hosted metasearch    | Free, no key; you run it; can be rate-limited by upstreams |
| **Tavily / Exa**             | RAG-native search         | AI-optimized snippets / neural search; future options      |

---

## Sources

- Serper — pricing & speed: <https://serper.dev/>,
  <https://www.buildmvpfast.com/tools/api-pricing-estimator/serper>
- Bing Search API retirement (Aug 11, 2025):
  <https://learn.microsoft.com/en-us/lifecycle/announcements/bing-search-api-retirement>
- ChatGPT search / OAI-SearchBot: <https://developers.openai.com/api/docs/bots>,
  <https://www.seerinteractive.com/insights/87-percent-of-searchgpt-citations-match-bings-top-results>
- Perplexity pipeline: <https://ziptie.dev/blog/how-perplexity-ai-answers-work/>
- Perplexity Sonar on Cerebras: <https://www.perplexity.ai/hub/blog/meet-new-sonar>
- Reranking in RAG: <https://www.pinecone.io/learn/series/rag/rerankers/>,
  <https://cohere.com/rerank>

_Content from external sources was paraphrased/summarized for licensing
compliance; see the links above for the originals._
