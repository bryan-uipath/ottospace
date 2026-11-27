# Roofs by kind of file

Asked: "i wonder if there could be any visual differentiation between file types". Agreed: a thin
roof cap coloured by the file's kind; the walls keep the folder's colour (`city_colours`). "we can
build the roof caps directly".

State: **built**, behind `city_kinds` (Experimental, off; with `code_city`).

## Decided

- **Kinds.** A closed set: code, test, docs, config, data. Sorted by path in the core
  (`city_kinds::RULES`, one table, first match wins, case ignored): a test folder (`tests/`,
  `test/`, `__tests__/`, `spec/`, `e2e/`) or name (`*_test.*`, `*.test.*`, `*.spec.*`,
  `test_*.py`) first; then docs by extension (`md`, `txt`, `rst`…); config (`toml`, `yaml`,
  `lock`, dotfiles and dot folders, `Makefile`, `package.json`, `*config*.json`); data (other
  `json`, `csv`, `sql`, pictures, sound, fonts); code by extension; last, docs by name
  (`README`, `LICENSE`, so `notice.rs` stays code).
- **Other: no cap.** A file no rule names keeps the plain roof of its folder's colour: a cap
  says what a file is, so none says "not one of these", and the city stays quieter.
- **The cap.** The box's top slice in the kind's colour, in the same instanced draw (a colour and
  a height per instance, the fragment shader's choice): no extra draw call. A quarter of the
  building's shorter side thick (it scales with the lots), at most a third of its height; it
  grows with the rise. With strata, on the top stratum only.
- **Colours.** Lighter and stronger than any folder's walls (lightness 0.34), duller than a lit
  building, none in amber's band (25° to 65°: "needs you"), told apart by lightness as well as
  hue: code `#7d8ca1` (dim slate: the most common, the quietest), test `#ca7dc3` (pink), docs
  `#60be7f` (green), config `#47b1c2` (cyan), data `#a99ad6` (pale lavender).
- **Lit wins.** A lit building (an agent at its file) has no cap: its task's colour on every face.
- **Closed boxes: none.** A district closed by the level of detail stands for many kinds; it
  keeps its plinth's colours.
- **Legend.** No legend line on the city (it stays quiet, [original.md](original.md)): a
  hovered file's label leads with its kind's swatch and name, `■ test  src/a_test.rs`.
- **Off.** No caps computed, every instance's cap height 0: the same pixels as before.

## State

`city.kinds`: how many buildings drawn each kind's cap (`code`, `test`, `docs`, `config`,
`data`), null with the flag off; `city.hovered.kind`. Never a path.

## Cost

Sorting once a layout (on the picture's thread, with the folder colours): an index of the rules'
extensions, so a file tries only the rules before its extension's. About 5 ms for 9,500 files and
30 ms for 50,000 (release, `CITY_KINDS=1 … a_turn_is_timed`). Per picture: four floats more per
box instance; the turn's times within noise.

## Later

- A legend line, if hover is not enough.
- The user's own rules (`city_kinds` patterns in the settings), as `city_folder_colors` is.
