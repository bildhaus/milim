# slugify(input, options)

Converts human text into a URL slug.

## Rules

1. `input` must be a string; otherwise throw a `TypeError`.
2. Decompose accented letters (Unicode NFKD) and drop the combining marks, so
   `"Crème Brûlée"` becomes `"creme-brulee"`.
3. Lowercase everything.
4. Replace `&` with the word `and` (surrounded by separators as needed).
5. Every run of characters that are not `a-z` or `0-9` becomes a single
   separator.
6. Remove leading and trailing separators.
7. `options.separator` defaults to `"-"`. It must be exactly one character
   from `-`, `_`, or `.`; anything else throws a `RangeError`.
8. `options.maxLength` (a positive integer, default `80`) caps the length. When
   the slug is longer, cut it at the last separator at or before `maxLength`
   so words are never split. If the first word alone is longer than
   `maxLength`, hard-cut that word at `maxLength`. The result never ends with
   a separator. A `maxLength` that is not a positive integer throws a
   `RangeError`.
9. If nothing is left, return `"n-a"` joined with the chosen separator
   (`"n-a"`, `"n_a"`, or `"n.a"`).

## Examples

| Input | Options | Output |
|---|---|---|
| `"Hello, World!"` | | `"hello-world"` |
| `"  Rock & Roll  "` | | `"rock-and-roll"` |
| `"Crème Brûlée"` | `{ separator: "_" }` | `"creme_brulee"` |
| `"The quick brown fox"` | `{ maxLength: 13 }` | `"the-quick"` |
| `"Supercalifragilistic"` | `{ maxLength: 5 }` | `"super"` |
| `"!!!"` | | `"n-a"` |
