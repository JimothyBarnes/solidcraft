# Post files

A post tells SolidCraft's one G-code writer how to write a program for one machine's control. It is
a JSON file. The three posts built into the app are in this folder; a machine maker's or a shop's
own go in the **post folder**, which Post Process lists:

| System | Post folder |
|---|---|
| macOS | `~/Library/Application Support/SolidCraft/cam/posts` |
| Windows | `%LOCALAPPDATA%\SolidCraft\cam\posts` |
| Linux | `~/.local/share/solidcraft/cam/posts` (or under `$XDG_DATA_HOME`) |

Post Process's **Add post file…** copies a post file into the folder as `<id>.json`, in place of an
older version of the same post. A file in the folder is read each time Post Process opens, so an
edited post is used as it now is. A `.json` file there that is not a post is listed with the reason.
Scripts are `cam.posts.list`, `cam.posts.add` and `cam.post` with `post` set to the file's path.

**Nothing a post says can make the machine go somewhere the toolpath does not.** A post only
chooses from what the writer can do, formats numbers and adds fixed lines. Every fixed line must
read without a move (not even a rapid across at machine home), whatever its properties are set to.
Every program is read back by SolidCraft's own G-code reader and compared with its toolpath move by
move before it is shown or saved; a program that differs is refused, not written. The reader refuses
whatever it does not know (parameters, expressions, G10, G92, codes outside its subset), so a
post can't hide a move in one. None of this makes a program safe to run: prove every one on the
machine first (air cut, single block).

## A post file

Start from one of the three here: copy it, change `id`, `name`, `vendor` and `version`, and change
what your control needs.

| Field | What it is |
|---|---|
| `format` | `1` |
| `id` | 1 to 64 of `a-z`, `0-9`, `_`. The file in the post folder is named by it. |
| `name` | Shown in Post Process (up to 100 characters). |
| `description` | What it writes and why (up to 2000 characters), shown when the pointer is on the post. |
| `vendor` | Who made it (up to 100 characters). Optional. |
| `version` | Its version, as its maker numbers them (up to 32 characters). Optional. |
| `machine_kind` | `"mill"`. |
| `numbers` | Decimal places: `length_mm`, `length_inch` (1 to 6), `feed_mm`, `feed_inch` (0 to 4), `speed`, `dwell` (0 to 3); `strip_trailing_zeros` writes `10.` for `10.000`. Every length and feed has a decimal point. |
| `line_numbers` | `{"start": 10, "step": 10}`, or `null` for none. |
| `comments` | `upper_case`, and `max_length` (48 to 256). Comments are always in parentheses. |
| `modal` | `repeat_motion`, `repeat_feed`: write G0/G1/F again when unchanged. |
| `arcs` | `planes` (`"xy"`, `"zx"`, `"yz"`), `helical`, `max_sweep_degrees` (arcs are split above it), `min_radius` and `linearize_tolerance` (mm; smaller arcs and arcs in other planes become lines within it). Arcs are written with I, J, K. |
| `program_number_digits` | Digits of `{program_number}`, zero-padded; `null` for a control without program numbers. |
| `dwell` | Whether dwells are written (`G4 P<seconds>` with all its decimals). |
| `cycles` | Which drilling cycles the control takes (`drill` G81, `dwell` G82, `peck` G83, `chip_break` G73, `tap` G84, `ream` G85/G89), `g98` (`"block_start"` or `"cycle_start"`: what G98 returns to), and `motion`: the control's own cycle distances (`peck_clearance`, `chip_break_back_off`, `peck_retract` `"clear_z"` or `"r_plane"`, `above_r`), used to draw and simulate holes. A hole whose cycle is off is written as plain moves. |
| `tool_length_offset` | Words before the first move after a tool change, e.g. `"G43 H{tool}"`. |
| `xy_before_z` | After a tool change, go across in X and Y first, then down in Z (needs `tool_change_before` to send Z to machine home with G28 or G53). |
| `optional_stop` | `"M1"` or `"M01"`, written before every tool change but the first when Post Process asks; or leave it out. |
| `properties` | The choices the post offers whoever posts with it (below). Optional. |
| `sequences` | The fixed lines: `program_start`, `safe_start`, `tool_change_before`, `tool_change_after`, `operation_start`, `program_end`. Up to 50 lines each, 200 characters a line. |

### Placeholders
A fixed line may use `{program_number}`, `{program_name}`, `{units_code}` (G20 or G21),
`{work_offset}` (G54 to G59), `{tool}` (only in the tool-change lines and `tool_length_offset`), and
the id of any of the post's properties.

### Properties
Each is shown as a row in Post Process and given to `cam.post` as `properties: {id: value}`:

```json
"properties": [
  { "id": "table_home", "label": "Table forward at the end (G28 Y0.)", "kind": "bool", "default": true,
    "description": "Send the table home in Y after Z, to bring the part forward for unloading." },
  { "id": "end_code", "label": "How the program ends", "kind": "choice", "default": "M30",
    "choices": [{ "value": "M30", "label": "End and rewind" }, { "value": "M2", "label": "End" }] },
  { "id": "warm_up", "label": "Warm-up (s)", "kind": "number", "default": 0, "min": 0, "max": 600 },
  { "id": "note", "label": "Note for the operator", "kind": "text", "default": "" }
]
```

| `kind` | Value | In a line |
|---|---|---|
| `bool` | `true` or `false` | only in a `when` |
| `number` | from `min` to `max` (both needed), written with `decimals` places (0 to 4, default 0) | `{id}` |
| `choice` | one of `choices`' `value`s (1 to 50, printable ASCII without `( ) { } % ;`); Post Process shows their `label`s | `{id}` |
| `text` | up to 100 characters, cleaned as every comment is | `{id}`, only inside a comment: `({id})` |

An `id` is 1 to 32 of `a-z`, `0-9`, `_`, starting with a letter. A post has at most 50 properties.

A line can be kept or left out by a property: `{"line": "G28 Y0.", "when": "table_home"}`.

| `when` | The line is written when |
|---|---|
| `"id"` | the property is on: a `bool` that is true, a `number` that is not zero, a `text` that is not empty |
| `"!id"` | it is off |
| `"id=value"` | a `choice` is that value |
| `"id!=value"` | a `choice` is not that value |

The optional stop (for a post that has one) and the comments that name the tools and operations are
`cam.post`'s own choices for every post, not properties.

### Checked when it is read
A post file that breaks any of the rules above is refused with the reason: an unknown field, a value
out of range, an unknown placeholder, a `when` that names no property, a text property outside a
comment, a choice whose value would make a line move the machine (each value of each property is
tried), and lines that do not read. Post Process lists it with that reason, and `cam.post` refuses to
post with it.
