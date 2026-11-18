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
| `fx` | object, optional | per-style insert FX layer (see below); absent = dry generation. All 10 factory presets carry one, each tuned individually |

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
| `reverb` | object | `size`/`damp`/`wet` 0..=1, `predelay_s` 0..=0.1 |
| `tape` | object, optional | `drive`/`mix` 0..=1, `hf_hz` 2000..=20000, `asym` 0..=0.5 |
| `delay` | object, optional | `time_s` 0.01..=2, `feedback` 0..=0.9, `mix` 0..=0.6, `damp` 0..=1, `ping_pong` bool. Whole-track insert (no per-lane sends exist), so factory mixes stay low |

All `fx` sub-fields are required when their object is present; out-of-range
values are parse errors, never clamped guesses.

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

GM keys used by the factory presets: kick 36, rim 37, snare 38, clap 39,
closed hat 42, open hat 46, toms 43/45/48, crash 49, ride 51, cowbell 56,
shaker 82.

## Generation conventions

- PPQ 960; one 16th step = 240 ticks.
- Swing delays every odd 16th step by the pair offset of
  `yeban-theory`'s `swung_onset_offset(480, permille)`.
- The fill (when requested and present) replaces the **last** bar
  entirely; lanes absent from the fill rest during that bar.
- Same preset + same parameters + same seed ⇒ byte-identical note list.
  Randomness (ghost notes) is a pure hash of `(seed, gm_key, step)`.
