# textutil

## `slugify(text)`

Turns arbitrary text into a URL slug.

1. Lowercase the text.
2. Transliterate common accented Latin letters to ASCII: `é` → `e`, `ü` → `u`,
   `ñ` → `n`, `ç` → `c`, and so on. Also `ß` → `ss`, `æ` → `ae`, `œ` → `oe`,
   `ø` → `o`, `ł` → `l`. Drop any other non-ASCII character.
3. Replace every run of characters that are not `a-z` or `0-9` with a single
   hyphen.
4. Trim leading and trailing hyphens.
5. Limit the result to at most 60 characters, cutting on a word boundary: keep
   the longest run of whole words (hyphen-separated) that fits. If the first
   word alone is longer than 60 characters, cut it at 60.

Examples:

| input                       | output          |
|-----------------------------|-----------------|
| `"Hello, World!"`           | `hello-world`   |
| `"  Crème Brûlée  "`        | `creme-brulee`  |
| `"Straße --- Ærø"`          | `strasse-aero`  |
| `"***"`                     | `""`            |
