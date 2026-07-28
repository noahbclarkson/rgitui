# Translations

Each file here is one language. English is not a file — it lives in
`crates/rgitui_i18n/src/en.rs` and is the source of truth every translation is
checked against.

## Adding a language

Copy `zh-CN.json`, rename it to your BCP-47 language tag, and translate the
values. Nothing else needs changing: locale files are embedded at build time and
the language picker in **Settings → Appearance** lists whatever it finds.

```json
{
  "id": "de",
  "english_name": "German",
  "native_name": "Deutsch",
  "plural_rule": "one_other",
  "messages": {
    "settings.nav.appearance": "Darstellung"
  }
}
```

| Field | Meaning |
| --- | --- |
| `id` | BCP-47 tag. A system locale of `de-AT` matches a shipped `de`. |
| `english_name` | Language name in English, used in logs and bug reports. |
| `native_name` | What the picker shows. Write it as its own speakers do, so a user who cannot read the current UI language can still find it. |
| `plural_rule` | `one_other` (English, German, Spanish), `zero_one_other` (French, Brazilian Portuguese), or `other_only` (Chinese, Japanese, Korean). |
| `messages` | Key → translated string. |

## Rules the tests enforce

`cargo test -p rgitui_i18n` checks every file here:

- **Missing keys are fine.** Anything you leave out falls back to English, so a
  partial translation is worth shipping. The language picker shows how many
  strings are still untranslated.
- **Unknown keys fail the build.** A key that is not in `en.rs` is a typo or a
  leftover from a deleted message, and would never render.
- **Placeholders must survive.** If the English string contains `{count}`, your
  translation must contain `{count}` too. Placeholder names are not translated;
  the text around them is, and may be reordered freely.
- **Empty values count as untranslated**, so `""` is a safe placeholder for
  "not done yet" — it renders English rather than a blank label.

Plural messages are a `.one` / `.other` pair. Only define the forms your
`plural_rule` can select: with `other_only` just `key.other`, with `one_other`
both.

## Testing without rebuilding

Drop a locale file in your config directory and restart the app:

- Linux: `~/.config/rgitui/locales/`
- macOS: `~/Library/Application Support/rgitui/locales/`
- Windows: `%APPDATA%\rgitui\locales\`

A file there replaces a shipped language with the same `id`, so you can iterate
on a translation against a release build.
