# A folder drawn as a pile (design)

The owner, 2026-11-23: "there is a better metaphor for my notes' raw folder that needs to be
organized. Instead of buildings, almost it should be a pile of recyclables or stuff that needs
to be sifted through", and "we could make a setting for folder for the viz - just like we
ignore some folders, we might want a different visual representation".

## The setting

`city_draw`, beside `city_exclude` (#411), with the same keys and the same matching rules:
- `*` for every checkout, or a checkout's path;
- an entry is a folder's name (anywhere) or a path from the checkout's root;
- nothing outside the checkout, no patterns.

Each entry names how that folder is drawn, from a closed set: `city` (the default) or `pile`.

```json
"city_draw": { "~/notes": { "raw": "pile" } }
```

Nothing guesses which folders are piles: only the setting says so.

## The pile

- **Where:** a lot beside the built districts, at the city's edge, so one picture shows the
  sorted city and the unsorted heap.
- **What:** each file is an item dropped loosely: overlapping, unaligned, newest on top. The
  heap is deterministic from the files: each item's place and turn come from a hash of its path
  and its order of arrival (modification time, then path). The same files give the same heap
  between reads; one file added lands on top and moves no other.
- **At a glance:**
  - **Age:** fresh items bright; old ones dusty, desaturated and lower.
  - **Kind:** by extension and a peek at the first line. A link file (a URL alone) is a slip,
    an image a flat card, a transcript a long strip, a note a folded sheet. Four shapes, each a
    few boxes, all instanced.
  - **The size of the job:** the heap's height grows with its count (square root, so 300 items
    is not a tower).
  - **Later:** a tint for a guessed destination folder.
- **Cheap:** one instance per item in the existing pipeline (the city's box shader, a box or two
  per item). No per-item text until the camera is near, under the same name rule as the city.

## Sorting out of it

Moving a file out of the pile's folder takes it off the heap at the next read. If it lands in a
drawn folder, it appears there as a building, and its arrival can pulse. The reward follows the
user's act ("Joy follows the user"): nothing moves by itself.

This is the spatial face of the decision triage "Sort" mode (PR #449): the heap is what Sort
works through. The link between them is not built now.

## In the core, with tests

- `city_draw` matching is the same function as `city_exclude`'s, with tests.
- Pile layout is pure: items' places from paths and times. Tests: stable under adding one file
  (no other item moves); newest on top; height monotone in count.

## Slice 1

The setting; the heap layout; age and kind looks; CI pictures of a made-up `raw` folder beside a
small city, before and after one file is sorted out.

## Later

Destination tint; a hover that names an item; the Sort mode link; piles inside a district (not
only at the edge).

Made-up file names only, in pictures and tests. The owner's notes are never read.
