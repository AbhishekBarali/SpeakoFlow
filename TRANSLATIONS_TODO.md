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

> Heads-up: every non-English locale is already ~half-incomplete
> (missing ~383 of ~769 keys as of this writing). That backlog predates the
> Personas work below — it's the normal state of the project.

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
