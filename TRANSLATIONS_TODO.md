# Translations — feature fix needed (deferred)

**Status: deferred on purpose.** English (`en`) is the source language and the
app is changing a lot right now, so translating into the other locales is being
saved for one pass at the end rather than chasing every UI change.

**Not a bug / nothing is broken at runtime.** i18next is configured with
`fallbackLng: "en"` (see `src/i18n/index.ts`), so any key that a locale is
missing simply renders in English. Non-English users see the odd label in
English until it's translated — no errors, no blank strings.

To see the full, current gap at any time:

```bash
bun run check:translations
```

> Heads-up: every non-English locale is already more than half-incomplete
> (each missing **566 of the 962** reference keys as of the S6 sweep,
> 2026-07-13). That backlog predates the Personas and Simplicity-Overhaul
> work below — it's the normal state of the project.

---

## Simplicity Overhaul (S0–S5) — recorded by S6 (2026-07-13)

The IA + copy overhaul rewrote and added a large number of English strings
(new namespaces `sidebar.dictation`, `sectionSubtitles.*`, `settings.dictation.*`,
`settings.assistant.brain.*`, `settings.assistant.subpages.*`, `onboarding.*`
rewrites, plus one-voice value edits across most of `settings.*`). English is
complete and the app is fully usable; the other locales fall back to English
per `fallbackLng`. Nothing is broken.

**Current gap (from `bun run check:translations`, 2026-07-13):**

- Reference (`en`): **962** keys.
- **0 / 19** locales pass. **Every** locale (ar, bg, cs, de, es, fr, he, it, ja,
  ko, pl, pt, ru, sv, tr, uk, vi, zh, zh-TW) is **missing 566 keys** (≈396
  present) **and carries 3 stale extra keys**.

**Stale extra keys to DELETE from all 19 non-English locales** (removed from
`en` when the sidebar shrank to 5 sections — they now render nothing):

- `sidebar.models`
- `sidebar.advanced`
- `sidebar.postProcessing`

**Also stale in the non-English locales (values only, when you do the pass):**
these keys still exist in `en` but their non-English values use pre-overhaul
wording — re-translate to the new vocabulary (see §3 Voice Guide in
`docs/simplicity-overhaul/PLAN.md`): "Post Process" / "AI Correction" →
**AI cleanup**; "hotkey" → **shortcut** (section heading stays "Shortcuts");
`transcribe` binding name → **Dictate**.

**Orphaned English keys — safe to prune during the same translation pass**
(kept for now: they exist in all 20 files, so deleting from `en` alone just
converts them into "extra" keys in the other 19; retire them everywhere at once):

- `onboarding.subtitle`, `onboarding.speechToText.continue`
- `settings.postProcessing.title`, `.enable.title`, `.hotkey.title`,
  `.api.title`, `.prompts.title` (the old flat "Post Process" page's group
  headings — the live AI-cleanup form uses `settings.postProcessing.api.*`,
  `.tone.*`, `.prompts.*` sub-keys, `settings.dictation.aiCleanup.*`, so only
  those five heading leaves are dead)
- `settings.advanced.groups.app`, `.transcription`, `.textReplacements`,
  `.history` (headings of the deleted `AdvancedSettings` shell; `.output` and
  `.experimental` are still live in General's fold)
- `settings.advanced.aiCorrection.*` (only the deleted `PostProcessingToggle`
  used it; the live toggle now uses `settings.dictation.aiCleanup.*`)
- `settings.debug.postProcessingToggle.*` (no debug row renders it)

**Frozen-surface copy debt (needs G8 lifted by the human — S5 flagged, S6
confirms still open):** the floating panel (`assistant.*`) and recording overlay
(`overlay.*`) were out of the overhaul's scope, so they keep pre-overhaul
vocabulary: `overlay.locked` says "hotkey" (vs the unified "shortcut") and
`assistant.character.switch` says "Switch persona" (vs "Profiles"). Resolve
these when the frozen surfaces are next touched.

---

## Pending: Personas feature (Characters → Personas rework)

Locale files to update: everything under `src/i18n/locales/<lang>/translation.json`
**except** `en/` (all 19 others). See
[CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) for the workflow.

### 1. New keys (added in `en`, need adding + translating everywhere else)

Under `settings.assistant.characters`:

- `roleLabel`
- `rolePlaceholder`
- `responseLength.label`
- `responseLength.hint`
- `responseLength.options.inherit`
- `responseLength.options.short`
- `responseLength.options.medium`
- `responseLength.options.long`

### 2. Changed English wording (Characters → Personas)

These keys already exist in the other locales, but their values still use the
old "character" wording. Re-translate them to the "persona" concept when you do
the pass:

- `sidebar.characters` — "Characters" → "Personas"
- `assistant.character.switch` — "Switch character" → "Switch persona"
- `settings.assistant.characters.title` — "Characters" → "Personas"
- `settings.assistant.characters.description`
- `settings.assistant.characters.galleryLabel` — "Your characters" → "Your personas"
- `settings.assistant.characters.editSection` — "Edit character" → "Edit persona"
- `settings.assistant.characters.newName` — "New character" → "New persona"
- `settings.assistant.characters.aiTitle` — "Describe your character" → "Describe your persona"
- `settings.assistant.characters.aiPlaceholder`
- `settings.assistant.characters.promptLabel` — "Personality (system prompt)" → "Instructions (system prompt)"
- `settings.assistant.characters.greetingLabel` — "Greeting" → "Greeting (optional)"

> Note: the internal i18n **keys** are intentionally still named `characters`
> (not `personas`). Only the displayed **values** changed. Don't rename the keys
> — that would touch code and every locale for no user benefit.
