# Drum style preset format (`yeban-drum-style`, version 1)

One JSON file per style. This format is the contract between style content
(factory presets in this directory today, downloadable packs from
`yeban-extras` later) and the generator in `yeban-theory`
(`drum_preset.rs` parses, `drum_gen.rs` generates). The parser is strict:
unknown shapes and out-of-range values are loud errors, never guesses.

## Top-level fields

| field | type | meaning |
|---|---|---|
| `format` | string | must be `"yeban-drum-style"` |
| `version` | integer | must be `1` |
| `id` | string | stable ASCII id (`a-z`, `0-9`, `-`), e.g. `trap` |
| `name` | string | English display name |
| `name_zh` | string | Chinese display name |
| `source` | object | provenance (see below) |
| `kit_family` | string | `"acoustic"` (SFZ sampled kit) or `"electronic"` (synth drum machine) |
| `bpm` | object | `{"default": int, "min": int, "max": int}`, `0 < min <= default <= max` |
| `swing_permille` | integer | default swing, 500 = straight, range 500..=1000 |
| `bars` | integer | pattern length in bars, 1..=8 (factory presets: 2) |
| `lanes` | array | drum lanes (see below) |
| `fill` | object, optional | `{"lanes": [...]}` with exactly 16 steps per lane (1 bar) |
| `fx` | object, optional | per-style insert FX layer (see below); absent = dry generation. All 14 factory presets carry one, each tuned individually |
| `bus` | object, optional | drum-bus chain (see below); absent = drum track connects straight to the master bus. All 14 factory presets carry one |
| `lane_chains` | array, optional | per-lane (per-piece) processing chains by GM key group (see below); absent = no per-lane chains |
| `automation` | array, optional | automation segments for the generated drum track (see below); absent = no automation lanes are written |
| `sfz_path` | string, optional | acoustic family only: repo-relative path of the SFZ kit program to bind; absent = the default basic kit. Each acoustic factory preset names its own program (different mic mix / kit) |

## `fx` object

The drum track's factory processing, in the shape of a Logic-style channel
strip preset: EQ + compression + insert reverb as the base, with optional
tape saturation and delay per style. Generation turns it into track insert
devices; when a target track already carries a user device of the same
family, that family is not duplicated.

| field | type | meaning |
|---|---|---|
| `eq` | object | `low_db`/`mid_db`/`high_db` −18..=18 dB; `low_hz` 20..=1000, `mid_hz` 100..=8000, `mid_q` 0.3..=6, `high_hz` 1000..=16000; `hpf_hz` 0..=500 (0 = off); `cutoff_hz` 800..=20000 (20000 = off) |
| `comp` | object | `threshold_db` −60..=0, `ratio` 1..=20, `knee_db` 0..=24, `attack_s` 0.0005..=0.2, `release_s` 0.01..=1, `makeup_db` −12..=12 |
| `output_db` | number | strip output trim −12..=12 dB (one of the style-to-style level balance points) |
| `reverb` | object | `kind` string: `room`/`plate`/`hall`/`spring`; `decay_s` 0.1..=10 (reverb time, seconds, T60); `damp`/`wet` 0..=1, `predelay_s` 0..=0.1. Generation writes `reverb_kind` + `reverb_decay_s` device params; the legacy dimensionless `size` field is gone from this format (the device still accepts `reverb_size`, presets no longer write it) |
| `tape` | object, optional | `drive`/`mix` 0..=1, `hf_hz` 2000..=20000, `asym` 0..=0.5, `wow_rate_hz` 0.05..=2, `wow_depth_ms` 0..=8, `flutter_rate_hz` 2..=15, `flutter_depth_ms` 0..=2, `seed` 0..=16777215 (wow/flutter delay-line modulation on the tape device, P7; depths 0 = no modulation, renders bit-identical) |
| `delay` | object, optional | `time_s` 0.01..=2, `feedback` 0..=0.9, `mix` 0..=0.6, `damp` 0..=1, `ping_pong` bool. Whole-track insert (no per-lane sends exist), so factory mixes stay low |

All `fx` sub-fields are required when their object is present; out-of-range
values are parse errors, never clamped guesses.

## `bus` object

The drum-bus chain. Generation (new-track path only) creates a dedicated
drum bus track (`AuxReturn`), routes the drum track into it and the bus
into the master bus, and installs two devices on the bus in chain order:

| field | type | meaning |
|---|---|---|
| `glue` | object | glue compressor: `threshold_db` −60..=0, `ratio` 1..=20 (factory values 2..=4), `knee_db` 0..=24, `attack_s` 0.0005..=0.2 (factory value 0.03, per the v2 spec: slow attack lets transients through), `release_s` 0.01..=1, `makeup_db` −12..=12. Target gain reduction is 1–2 dB, set by threshold against the kit level |
| `clipper` | object | final-stage soft clipper: `drive_db` 0..=24, `ceiling_db` −6..=0 (factory −0.35, keeping rendered true peak below −0.1 dBTP), `knee_db` 0..=6, `mix` 0..=1, `oversample` one of 1/2/4/8 |

Generating onto an existing track does not create a bus (the track keeps
its current routing); the plan/report says so explicitly.

## `lane_chains` array (optional, per-lane processing)

Per-piece processing chains addressed by GM key groups — one group is one
drum piece (or one choke family) with its own EQ/comp/space/pan, installed
alongside the whole-track `fx` chain. Absent = no per-lane chains
(generation behaves exactly as before this field existed). The format
version stays 1: the field is additive and optional.

Each entry is `{name, keys, chain}`:

| field | type | meaning |
|---|---|---|
| `name` | string | group name (e.g. `"hats"`); carried into the project's lane chain and the generation plan text |
| `keys` | array of int | GM keys of this group, strictly ascending, non-empty. Every key must be one of the preset's lane keys (main lanes ∪ fill lanes). Groups are mutually non-overlapping, ordered by first key, at most 16 groups |
| `chain` | object | the group's processing; every sub-object is optional, but a group whose chain declares nothing at all is a parse error |

`chain` sub-objects (when an object is present, all of its fields are
required; ranges are exactly the `fx` ranges of the same section — one
shared validation implementation, out-of-range is a parse error):

| field | type | meaning |
|---|---|---|
| `eq` | object, optional | same fields/ranges as `fx.eq` |
| `comp` | object, optional | same fields/ranges as `fx.comp` |
| `reverb` | object, optional | same fields/ranges as `fx.reverb` |
| `tape` | object, optional | same fields/ranges as `fx.tape` |
| `delay` | object, optional | same fields/ranges as `fx.delay` |
| `pan` | number, optional | group pan override −1..=1 |
| `sidechain` | object, optional | `{key_keys}`: the keys whose voices key this group's compressor; strictly ascending, non-empty, subset of the preset's lane keys |

Generation rules (loud by default — violations refuse generation and name
themselves, nothing is silently dropped):

- Lane chains are installed as one whole-table write per generation; on an
  existing track that already carries user lane chains, generation keeps
  the user's chains and the plan/report says so.
- A group must not split an SFZ choke (exclusive/`off_by`) group of the
  chosen acoustic program: the used keys of one choke group must sit in a
  single lane group or stay ungrouped as a whole. (Electronic kits have no
  choke groups.)
- `sidechain` declarations parse, but generation refuses them until
  lane-level sidechain evaluation lands (phase 4 of the per-lane line);
  factory presets do not ship sidechain values before phase 5.
- Lane-chain parameters are not automatable in v1.

Factory opt-ins: lane chains now ship in 12 of the 14 factory presets
(all but edm and techno). The original three were house (hats 42+46,
hat EQ), trap (closed hat 42, hat EQ) and lofi (kick 36, darkening EQ);
the acoustic presets carry per-piece chains matched to their mic
program (kick / snare / hats / crash / shaker, the hats group holding
the program's full choke family — {42, 44, 46} on the basic keymap,
{42, 44, 92} on the CC4 keymap), and drill / hiphop / phonk add an
808 bass (35) chain.

Factory pan layouts (per-style, by genre convention — no layout is
copied between styles): electronic styles place per-voice pans in the
synth parameter table (kick / snare / 808 always dead centre; trap and
drill spread a wide percussion field, phonk's cowbell leads at -0.40,
house keeps its clap dead centre with hats/ride right, techno stays
narrow within +/-0.25, lofi near-mono within +/-0.18). Acoustic styles
place group pans in `lane_chains[].chain.pan`; their kick/snare groups
carry no pan (centre, no override). Jazz's ride group leads right
(+0.45), rock is the widest acoustic kit (hats +0.30, shaker +0.15),
funk stays compact (all groups within +/-0.18), rnb is smooth and
moderate (shaker +0.35), pop puts hats at +0.22 with the shaker left
(-0.25), and boombap stays narrow (hats +0.12, shaker +0.20).

## `automation` array (optional, preset automation segments)

Each segment becomes one automation lane on the generated drum track,
written with the project's existing automation model (one
`SetAutomationLane` op per segment; the lane is evaluated by the
renderer's device-param automation at block boundaries — no second
automation semantics). A segment is
`{"target": string, "curve": string (optional), "points": [[tick, value], ...]}`:

- `target`: `track_volume` (drum-track volume, −60..=12 dB),
  `track_pan` (drum-track pan, −1..=1 — this is where an Auto-Pan
  gesture is written), or `<family>.<param>` addressing one parameter
  of the drum track's FX layer. `family` is the FX device family
  (`strip`/`tape`/`reverb`/`delay`/`transient`/`crusher`/`noise`);
  `param` is the canonical device-param spelling (e.g. `reverb_wet`,
  `delay_mix`). The legal param set and value range per family are
  exactly the params and ranges the `fx` section itself can write.
  Lane-chain and drum-bus devices are not addressable here.
- `points`: `[tick, value]` pairs; non-empty, ticks strictly
  ascending, integer ticks within `0..=bars * 3840` (960 PPQ; the end
  point may sit exactly on the pattern end), values finite and inside
  the target's range. Point ticks are pattern-relative; generation
  shifts them by the clip's placement start tick.
- `curve`: `linear` (default), `exponential`, `logarithmic`, or
  `s_curve` — the segment's curve shape, expanded onto every point
  when the lane is written (the project model stores a curve per
  point and has no default).

Parse-time loud rejections: unknown target/family/param, a device
target whose FX family is absent from the preset's `fx` section
(including the strip's conditional params `hpf_hz`, `cutoff_hz`,
`filter_enabled`, `output_gain_db` when the corresponding stage is
disabled), duplicate targets, empty point lists, non-ascending or
out-of-span ticks, out-of-range values, unknown curve names.

Generation conventions: a segment whose target parameter is not
found on the generated track (e.g. the target track carries a user
device of that family without the named param) fails the whole
generation loudly — a written segment is never dropped silently.
If the target track already has a user automation lane for the
resolved target, the segment is skipped (user automation wins, the
same precedence as lane chains and FX families) and the plan/report
says so. Factory use: edm only (reverb wet builds 0.10 → 0.15 → 0.22
across the 2-bar loop, `s_curve`).

## `source` object

| field | type | meaning |
|---|---|---|
| `kind` | string | e.g. `"groove-adapted"` |
| `groove_genre_id` | string | source entry id in the origin repository |
| `repo` | string | origin repository |
| `license` | string | origin license (factory presets: groove is MIT) |
| `converted` | string | conversion date `YYYY-MM-DD` |
| `tuning` | string[] | every tuning applied after mechanical conversion, one short sentence each |

## Lanes and steps

A lane is `{"role": string, "gm_key": int 0..=127, "steps": [...]}`.
`steps` has exactly `bars * 16` entries (16th-note grid). Each step is a
triple `[velocity, ratchet, tier]`:

- `velocity` 0..=127; **0 = rest**.
- `ratchet` 1..=8: the step is split into this many equal sub-hits
  (3 = triplet-style thirds — this is how hat rolls are encoded; there is
  no separate roll type). Sub-hit *k* (0-based) gets velocity factor
  `0.85 + 0.15 * k / ratchet`, so rolls crescendo slightly.
- `tier` 0/1/2: complexity layer — 0 core (always played), 1 standard,
  2 ornament. The generator's complexity knob gates tiers
  (<34 core only, 34..66 core+standard, >=67 all tiers + ghost notes).

A lane may also carry `layers` (optional): extra keys struck together
with the lane's own key on every hit step — this is how a snare+clap
stack is written (one lane, one pattern, no hand-copied second lane).
Each layer is
`{"gm_key": int 0..=127, "level_db": number -48..=12 (optional, default 0), "vel_min": int 1..=127 (optional, default 1), "vel_max": int 1..=127 (optional, default 127)}`.
The layer fires only when the step's **written** velocity lies inside
`[vel_min, vel_max]`; its velocity is the written velocity scaled by
`10^(level_db/20)` (amplitude dB) before the generator's shared
intensity/ratchet factors. Layers follow the step's tier gate and
ratchet expansion; ghost notes never trigger layers. A layer key must
not collide with any lane key or other layer key in the same lane
table (main lanes and fill lanes are separate tables), and an empty
`layers` array is invalid — violations are rejected at parse time.
Layer keys count as sounding keys for the coverage rule below.
Absent `layers` = single-key lane; generation is bit-identical to
before the field existed. Factory use: hiphop only (clap layered on
the snare lane, −3.2 dB; the fill snare carries its own windowed
layer, −2.5 dB at velocity ≥ 112).

GM keys used by the factory presets: kick 36, rim 37, snare 38, clap 39,
closed hat 42, pedal hat 44, open hat 46, toms 43/45/48, crash 49,
ride 51, ride bell 53, cowbell 56, shaker 82.

Coverage rule (loud, never silent): generation refuses a preset whose
pattern hits a key with no voice. Electronic voices are the synth kit's
13 note slots. Acoustic coverage depends on the chosen SFZ program —
the Virtuosity programs do **not** share one keymap: only
`01-basic-kit.sfz` (basic keymap) has regions on key 46; the other
factory programs use the continuous CC4-pedal hi-hat keymap (hat on
42, nothing on 46), and `08-vintage-mic.sfz` / `03-kick-mic.sfz` also
lack 56 and 82 (probed per program, 2026-11-18). Presets assigned to
those programs must not use the missing keys.

## Generation conventions

- PPQ 960; one 16th step = 240 ticks.
- Swing delays every odd 16th step by the pair offset of
  `yeban-theory`'s `swung_onset_offset(480, permille)`.
- The fill (when requested and present) replaces the **last** bar
  entirely; lanes absent from the fill rest during that bar.
- Same preset + same parameters + same seed ⇒ byte-identical note list.
  Randomness (ghost notes) is a pure hash of `(seed, gm_key, step)`.
