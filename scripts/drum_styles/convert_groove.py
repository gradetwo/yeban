#!/usr/bin/env python3
"""Convert groove genre sequencer patterns into Yeban drum-machine style presets.

Source & license
----------------
Source data: gradetwo/groove, src/data/genres/*.ts (sequencer_pattern fields).
The groove repository is MIT licensed, author Wang. This mechanical
conversion + tuning was produced on 2026-11-17 for Yeban drum machine
phase 1. Only the four drum tracks (kick / snare / hihat / percussion)
are converted; melodic tracks (bass / chords / lead / fx) are ignored.

Groove step semantics (verified against the groove sources)
-----------------------------------------------------------
- steps[i] > 0 means a hit on step i; velocity[i] == 0 (or a missing
  velocity array entry) means no hit.
- On the hihat track, steps[i] == 3 is a triplet ratchet: the step is
  played as 3 equal sub-hits. Step values 1 and 2 only mean "hit".
- Ratchet semantics in the output: one grid cell plays `ratchet` equal
  sub-hits, expanded at the event layer; sub-hit velocities ramp by
  0.85 + 0.15 * i / n (same convention as groove).

Tuning rules applied after the mechanical conversion
----------------------------------------------------
1. Expand to 2 bars: bar 2 is a copy of bar 1 with one deterministic,
   per-style change (end-of-bar open hat, or an end-of-bar kick
   variation). No randomness anywhere.
2. Velocity shaping: a drum lane whose hit velocities are all equal
   gets a grid formula instead (downbeat 100, bar-start downbeat 112,
   snare backbeat 110, off-8th 85, weak 16ths alternating 75/85, plus
   a +4 phrase lift in bar 2). Lanes with original variation keep
   their original velocities.
3. swing_permille is a fixed per-style value (500 = straight). The
   groove swing percentage converts as 500 + 5 * pct; when it differs
   from the fixed value the original is noted in `tuning`.
4. Hat rolls: trap/drill get a ratchet roll on the last two hat cells
   of bar 2 (trap: 2 = 32nd-note split, drill: 3); phonk keeps its
   source ratchets and gains one more.
5. Fills: one 16-step fill bar per style. Acoustic kits: snare density
   ramp (8ths -> 16ths) + crash (GM 49) on the last cell + tom run
   (48/45/43). Electronic kits: snare/clap density ramp + open hat
   (GM 46) on the last cell. Lofi uses a sparse snare variation fill.
6. Tiers: kick/snare main hits on quarter positions = 0 (core); other
   real hits = 1; weak decorative hits (velocity < 60 on hat /
   percussion lanes) = 2.

Re-running this script produces byte-identical output files.
"""

import json
import re
from pathlib import Path

GROOVE_GENRES_DIR = Path.home() / "workspace" / "groove" / "src" / "data" / "genres"
REPO_ROOT = Path(__file__).resolve().parents[2]
OUT_DIR = REPO_ROOT / "crates" / "yeban-theory" / "drum_styles"
CONVERTED_DATE = "2026-11-17"

# GM key per lane role name.
ROLE_BY_GM = {
    36: "kick",
    37: "rim",
    38: "snare",
    39: "clap",
    42: "hihat",
    43: "tom_low",
    45: "tom_mid",
    46: "hihat_open",
    48: "tom_hi",
    49: "crash",
    82: "shaker",
}

# Percussion-track instrument -> GM key, checked in order (substring
# match). "shaker" must precede "rim" so rim_shaker lands on 82.
PERCUSSION_GM_RULES = [
    ("clap", 39),
    ("open_hat", 46),
    ("openhat", 46),
    ("closed_hat", 42),
    ("shaker", 82),
    ("rimshot", 37),
    ("rim", 37),
    ("cowbell", 56),
    ("tambourine", 54),
    ("crash", 49),
    ("ride", 51),
    ("tom_hi", 48),
    ("tom_mid", 45),
    ("tom_low", 43),
    ("snare", 38),
    ("kick", 36),
]

FIXED_GM_BY_TRACK = {"kick": 36, "snare": None, "hihat": 42}  # snare: by instrument


def gm_for_snare_track(instrument: str) -> int:
    name = instrument.lower()
    if "clap" in name:
        return 39
    if "rimshot" in name or "rim" in name:
        return 37
    return 38


def gm_for_percussion(instrument: str):
    name = instrument.lower()
    for needle, gm in PERCUSSION_GM_RULES:
        if needle in name:
            return gm, True
    return 82, False  # unknown instrument: keep 82, caller notes it


def load_genres(filename: str) -> dict:
    text = (GROOVE_GENRES_DIR / filename).read_text(encoding="utf-8")
    m = re.search(r"=\s*\[", text)
    start = text.index("[", m.start())
    end = text.rindex("]")
    return {g["id"]: g for g in json.loads(text[start : end + 1])}


def shaped_velocity(role_track: str, step: int) -> int:
    """Grid velocity formula for lanes whose source velocities are flat."""
    bar, pos = divmod(step, 16)
    sub = pos % 4
    if role_track == "snare" and pos in (4, 8, 12):
        v = 110  # backbeat (incl. half-time beat-3 backbeat)
    elif sub == 0:
        v = 112 if pos == 0 else 100  # downbeat; bar start accented
    elif sub == 2:
        v = 85  # off-8th
    elif sub == 1:
        v = 75  # weak 16ths alternate 75/85
    else:
        v = 85
    return min(127, v + bar * 4)  # +4 phrase lift in bar 2


def tier_for(track_id: str, pos: int, velocity: int) -> int:
    if track_id in ("kick", "snare"):
        return 0 if pos % 4 == 0 else 1
    # hihat / percussion lanes
    if velocity < 60:
        return 2
    return 1


def build_bar1_lanes(genre: dict):
    """Mechanical conversion of one groove bar.

    Returns (lanes, notes) where lanes maps gm_key ->
    {step_index: (velocity, ratchet, tier)} for steps 0..15, and notes
    collects tuning notes about unrecognized instruments.
    """
    notes = []
    lanes: dict[int, dict[int, tuple[int, int, int]]] = {}
    sp = genre["sequencer_pattern"]
    for track in sp["tracks"]:
        track_id = track["track_id"]
        if track_id not in ("kick", "snare", "hihat", "percussion"):
            continue  # melodic tracks are ignored
        instrument = track.get("instrument", "")
        if track_id == "percussion":
            gm, known = gm_for_percussion(instrument)
            if not known:
                notes.append(
                    f"percussion instrument '{instrument}' unrecognized; kept on GM 82 (shaker)"
                )
        elif track_id == "snare":
            gm = gm_for_snare_track(instrument)
        else:
            gm = FIXED_GM_BY_TRACK[track_id]
        steps = track["steps"]
        velocities = track.get("velocity") or []
        lane = lanes.setdefault(gm, {})
        for i in range(16):
            step_val = steps[i] if i < len(steps) else 0
            vel = velocities[i] if i < len(velocities) else (100 if step_val > 0 else 0)
            if step_val <= 0 or vel <= 0:
                continue
            ratchet = 3 if (track_id == "hihat" and step_val == 3) else 1
            tier = tier_for(track_id, i, vel)
            if i in lane:  # same-key merge: louder hit wins
                old_vel, old_ratchet, old_tier = lane[i]
                if vel > old_vel or (vel == old_vel and tier < old_tier):
                    lane[i] = (vel, ratchet, tier)
            else:
                lane[i] = (vel, ratchet, tier)
    return lanes, notes


def expand_two_bars(lanes16):
    """Copy bar 1 (steps 0..15) to bar 2 (steps 16..31)."""
    out = {}
    for gm, lane in lanes16.items():
        new_lane = dict(lane)
        for step, hit in lane.items():
            new_lane[step + 16] = hit
        out[gm] = new_lane
    return out


def apply_shaping(lanes32, gm_track: dict):
    """Apply the grid velocity formula to lanes that are flat.

    Flat = all hit velocities in the (pre-expansion) bar are equal.
    Returns list of gm keys that were shaped.
    """
    shaped = []
    for gm, lane in lanes32.items():
        bar1_vels = {v for s, (v, _r, _t) in lane.items() if s < 16}
        if len(bar1_vels) <= 1 and any(s < 16 for s in lane):
            track_id = gm_track[gm]
            for step, (_v, ratchet, _t) in list(lane.items()):
                nv = shaped_velocity(track_id, step)
                lane[step] = (nv, ratchet, tier_for(track_id, step % 16, nv))
            shaped.append(gm)
    return shaped


def lane_steps_array(lane: dict, length: int):
    steps = []
    for i in range(length):
        if i in lane:
            v, r, t = lane[i]
            steps.append([v, r, t])
        else:
            steps.append([0, 1, 0])
    return steps


def lanes_json(lanes: dict, length: int):
    return [
        {"role": ROLE_BY_GM[gm], "gm_key": gm, "steps": lane_steps_array(lanes[gm], length)}
        for gm in sorted(lanes)
    ]


# ---------------------------------------------------------------------------
# Per-style bar-2 variations (rule 1) and hat rolls (rule 4).
# Each function mutates the 32-step lane map and returns a tuning note.
# ---------------------------------------------------------------------------

def var_open_hat(step: int, note: str):
    def apply(lanes):
        # Convert the closed-hat hit at `step` into an open-hat hit
        # (keeping its velocity and ratchet).
        if 42 in lanes and step in lanes[42]:
            v, r, _t = lanes[42].pop(step)
            lanes.setdefault(46, {})[step] = (v, r, 1)
        return note

    return apply


def var_trap_kick(lanes):
    lanes.setdefault(36, {})[30] = (95, 1, 1)
    return "bar 2 variation: extra kick at step 30 (end-of-bar kick variation)"


def var_lofi_kick(lanes):
    lanes.setdefault(36, {})[30] = (85, 1, 1)
    return "bar 2 variation: soft kick pickup at step 30 (end-of-bar kick variation)"


def var_edm_open_hat_tail(lanes):
    lanes.setdefault(46, {})[31] = (70, 1, 1)
    return "bar 2 variation: extra open hat on the last cell (step 31)"


def roll_last_two(ratchet: int, note: str):
    def apply(lanes):
        if 42 in lanes:
            for step in (30, 31):
                if step in lanes[42]:
                    v, _r, t = lanes[42][step]
                    lanes[42][step] = (v, ratchet, t)
        return note

    return apply


def roll_phonk_extra(lanes):
    # Keep all source ratchets; add one roll on the last hat cell of bar 2.
    if 42 in lanes and 31 in lanes[42]:
        v, _r, t = lanes[42][31]
        lanes[42][31] = (v, 2, t)
    return "hat rolls: source triplet ratchets kept; one extra ratchet=2 roll added at step 31"


# ---------------------------------------------------------------------------
# Fills (rule 5). Each returns (fill_lanes_dict, tuning note).
# ---------------------------------------------------------------------------

def fill_acoustic(main_gms, snare_gm=38):
    lanes = {gm: {} for gm in main_gms}
    snare = lanes.setdefault(snare_gm, {})
    for i, v in enumerate([80, 88, 96, 104]):  # 8ths, first half
        snare[i * 2] = (v, 1, 1)
    for j, v in enumerate([100, 105, 110, 115, 118, 121, 124, 127]):  # 16ths ramp
        snare[8 + j] = (v, 1, 1)
    lanes.setdefault(48, {})[12] = (100, 1, 1)  # tom_hi
    lanes.setdefault(45, {})[13] = (105, 1, 1)  # tom_mid
    lanes.setdefault(43, {})[14] = (110, 1, 1)  # tom_low
    lanes.setdefault(49, {})[15] = (120, 1, 1)  # crash on last cell
    if 36 in lanes:
        lanes[36][0] = (100, 1, 0)
    return lanes, "fill: snare density ramp (8ths to 16ths) + tom run 48/45/43 + crash 49 on last cell"


def fill_electronic(main_gms, snare_gm=38):
    lanes = {gm: {} for gm in main_gms}
    snare = lanes.setdefault(snare_gm, {})
    for step, v in [(4, 90), (6, 95), (8, 100), (10, 105), (12, 110), (13, 116), (14, 122), (15, 127)]:
        snare[step] = (v, 1, 1)
    lanes.setdefault(46, {})[15] = (110, 1, 1)  # open hat, last cell (long)
    if 36 in lanes:
        lanes[36][0] = (100, 1, 0)
    return lanes, "fill: snare/clap density ramp + open hat 46 on last cell"


def fill_lofi_sparse(main_gms, snare_gm=37):
    lanes = {gm: {} for gm in main_gms}
    snare = lanes.setdefault(snare_gm, {})
    snare[4] = (100, 1, 0)
    snare[12] = (110, 1, 0)
    snare[15] = (70, 1, 1)  # ghost pickup
    if 36 in lanes:
        lanes[36][0] = (95, 1, 0)
    if 42 in lanes:
        for i, v in enumerate([70, 60, 70, 60, 70, 60, 70, 60]):
            lanes[42][i * 2] = (v, 1, 1)
    return lanes, "fill: sparse snare variation only (no crash/tom; kept restrained for lo-fi)"


# ---------------------------------------------------------------------------
# Style table.
# ---------------------------------------------------------------------------

STYLES = [
    {
        "id": "trap", "name": "Trap", "name_zh": "陷阱 Trap",
        "file": "hiphop.ts", "groove_id": "trap-rap",
        "select_reason": "groove entry 'trap-rap' (hiphop.ts) chosen over 'edm-trap' (trap_drill.ts): pure Southern trap (half-time snare on beat 3, triplet hat rolls) without EDM festival elements",
        "kit_family": "electronic",
        "bpm": {"default": 140, "min": 130, "max": 150},
        "swing_permille": 515,
        "variation": var_trap_kick,
        "roll": roll_last_two(2, "hat roll: bar 2 last two hat cells set to ratchet=2 (32nd-note split)"),
        "fill": "electronic",
    },
    {
        "id": "drill", "name": "UK Drill", "name_zh": "钻头 Drill",
        "file": "trap_drill.ts", "groove_id": "uk-drill",
        "select_reason": "groove entry 'uk-drill' (trap_drill.ts): the canonical UK drill pattern (sliding-808 kick placement, off-grid ghost snare)",
        "kit_family": "electronic",
        "bpm": {"default": 142, "min": 140, "max": 145},
        "swing_permille": 520,
        "variation": var_open_hat(28, "bar 2 variation: closed hat at step 28 converted to open hat (end-of-bar open hat)"),
        "roll": roll_last_two(3, "hat roll: bar 2 last two hat cells set to ratchet=3"),
        "fill": "electronic",
    },
    {
        "id": "phonk", "name": "Phonk", "name_zh": "冯克 Phonk",
        "file": "hard_electro.ts", "groove_id": "phonk",
        "select_reason": "groove entry 'phonk' (hard_electro.ts) chosen over 'drift-phonk': classic Memphis phonk tempo and swing; drift-phonk is the faster rave variant",
        "kit_family": "electronic",
        "bpm": {"default": 135, "min": 130, "max": 145},
        "swing_permille": 540,
        "variation": var_open_hat(30, "bar 2 variation: closed hat at step 30 converted to open hat (end-of-bar open hat)"),
        "roll": roll_phonk_extra,
        "fill": "electronic",
    },
    {
        "id": "lofi", "name": "Lo-Fi Hip-Hop", "name_zh": "低保真 Lo-Fi",
        "file": "hiphop.ts", "groove_id": "lofi-hip-hop",
        "select_reason": "groove entry 'lofi-hip-hop' (hiphop.ts): dusty rimshot backbeat, sparse 8th-note hats, heavy source swing",
        "kit_family": "acoustic",
        "bpm": {"default": 80, "min": 70, "max": 85},
        "swing_permille": 590,
        "variation": var_lofi_kick,
        "roll": None,
        "fill": "lofi",
    },
    {
        "id": "boombap", "name": "Boom Bap", "name_zh": "经典嘻哈 Boom Bap",
        "file": "hiphop.ts", "groove_id": "boom-bap",
        "select_reason": "groove entry 'boom-bap' (hiphop.ts): the classic boom bap entry (acoustic kit, swung 16th hats)",
        "kit_family": "acoustic",
        "bpm": {"default": 90, "min": 85, "max": 95},
        "swing_permille": 570,
        "variation": var_open_hat(30, "bar 2 variation: closed hat at step 30 converted to open hat (end-of-bar open hat)"),
        "roll": None,
        "fill": "acoustic",
    },
    {
        "id": "house", "name": "House", "name_zh": "浩室 House",
        "file": "house.ts", "groove_id": "chicago-house",
        "select_reason": "groove entry 'chicago-house' (house.ts): the original classic house pattern (four-on-the-floor, clap on 2 and 4, offbeat hats)",
        "kit_family": "electronic",
        "bpm": {"default": 124, "min": 120, "max": 128},
        "swing_permille": 525,
        "variation": var_open_hat(30, "bar 2 variation: closed hat at step 30 converted to open hat (end-of-bar open hat)"),
        "roll": None,
        "fill": "electronic",
    },
    {
        "id": "techno", "name": "Techno", "name_zh": "铁克诺 Techno",
        "file": "techno.ts", "groove_id": "detroit-techno",
        "select_reason": "groove entry 'detroit-techno' (techno.ts): the classic Detroit techno pattern (driving four-on-the-floor, offbeat hats)",
        "kit_family": "electronic",
        "bpm": {"default": 128, "min": 125, "max": 135},
        "swing_permille": 510,
        "variation": var_open_hat(30, "bar 2 variation: closed hat at step 30 converted to open hat (end-of-bar open hat)"),
        "roll": None,
        "fill": "electronic",
    },
    {
        "id": "edm", "name": "EDM Big Room", "name_zh": "大房间 EDM",
        "file": "trance.ts", "groove_id": "uplifting-trance",
        "select_reason": "groove has no big room entry; groove entry 'uplifting-trance' (trance.ts) used as substitute skeleton (festival four-on-the-floor + clap) and adapted to big room traits; see adaptation note",
        "kit_family": "electronic",
        "bpm": {"default": 128, "min": 126, "max": 130},
        "swing_permille": 500,
        "variation": var_edm_open_hat_tail,
        "roll": None,
        "fill": "electronic",
        "adapt_edm": True,
    },
    {
        "id": "pop", "name": "Pop", "name_zh": "流行 Pop",
        "file": "pop_rnb.ts", "groove_id": "synth-pop",
        "select_reason": "groove entry 'synth-pop' (pop_rnb.ts): the mainstream pop pattern of the file (four-on-the-floor-leaning kick, backbeat snare, 8th hats) at a general-purpose 120 BPM",
        "kit_family": "acoustic",
        "bpm": {"default": 120, "min": 115, "max": 130},
        "swing_permille": 515,
        "variation": var_open_hat(30, "bar 2 variation: closed hat at step 30 converted to open hat (end-of-bar open hat)"),
        "roll": None,
        "fill": "acoustic",
    },
    {
        "id": "rock", "name": "Rock", "name_zh": "摇滚 Rock",
        "file": "rock_metal.ts", "groove_id": "hard-rock",
        "select_reason": "groove entry 'hard-rock' (rock_metal.ts): the classic rock pattern (acoustic kit, backbeat snare, 8th hats with an open-hat accent)",
        "kit_family": "acoustic",
        "bpm": {"default": 125, "min": 115, "max": 140},
        "swing_permille": 500,
        "variation": var_open_hat(28, "bar 2 variation: closed hat at step 28 converted to open hat (end-of-bar open hat)"),
        "roll": None,
        "fill": "acoustic",
    },
]


def convert(style: dict) -> dict:
    genres = load_genres(style["file"])
    genre = genres[style["groove_id"]]
    sp = genre["sequencer_pattern"]

    tuning = [f"source selection: {style['select_reason']}"]

    lanes16, notes = build_bar1_lanes(genre)
    tuning.extend(notes)

    # Track-id per GM key (for shaping / fill decisions), from bar 1.
    gm_track = {}
    for track in sp["tracks"]:
        tid = track["track_id"]
        if tid == "kick":
            gm_track[36] = "kick"
        elif tid == "snare":
            gm_track[gm_for_snare_track(track.get("instrument", ""))] = "snare"
        elif tid == "hihat":
            gm_track[42] = "hihat"
        elif tid == "percussion":
            gm, _known = gm_for_percussion(track.get("instrument", ""))
            gm_track.setdefault(gm, "percussion")

    # EDM adaptation: closed hats become 1/8 open hats (big room trait).
    if style.get("adapt_edm"):
        if 42 in lanes16:
            lanes16[46] = lanes16.pop(42)
        gm_track[46] = gm_track.pop(42, "hihat")
        tuning.append(
            "adaptation: groove has no big room entry; pattern rebuilt to big room traits "
            "(four-on-the-floor kick kept, clap on 2 and 4 kept as the big clap, offbeat "
            "closed hats converted to 1/8 open hats GM 46, tempo set to 128 BPM)"
        )

    lanes = expand_two_bars(lanes16)
    tuning.append("expanded to 2 bars (bar 2 starts as a copy of bar 1)")

    shaped = apply_shaping(lanes, gm_track)
    if shaped:
        names = ", ".join(f"{ROLE_BY_GM[g]}({g})" for g in sorted(shaped))
        tuning.append(f"velocity shaping applied to flat lane(s): {names} (grid formula; varied lanes keep source velocities)")
    else:
        tuning.append("velocity shaping: not needed (every lane already has velocity variation)")

    variation_note = style["variation"](lanes)
    tuning.append(variation_note)

    if style["roll"] is not None:
        tuning.append(style["roll"](lanes))

    # Swing: fixed value; note the groove original when it converts differently.
    groove_pct = sp.get("swing", 0)
    groove_permille = 500 + 5 * groove_pct
    if groove_permille != style["swing_permille"]:
        tuning.append(
            f"swing: groove source swing {groove_pct}% converts to {groove_permille} permille; "
            f"fixed style value {style['swing_permille']} used"
        )

    # Fill.
    main_gms = sorted(lanes)
    snare_gm = next((g for g in main_gms if gm_track.get(g) == "snare"), 38)
    if style["fill"] == "acoustic":
        fill_lanes, fill_note = fill_acoustic(main_gms, snare_gm)
    elif style["fill"] == "lofi":
        fill_lanes, fill_note = fill_lofi_sparse(main_gms, snare_gm)
    else:
        fill_lanes, fill_note = fill_electronic(main_gms, snare_gm)
    tuning.append(fill_note)

    doc = {
        "format": "yeban-drum-style",
        "version": 1,
        "id": style["id"],
        "name": style["name"],
        "name_zh": style["name_zh"],
        "source": {
            "kind": "groove-adapted",
            "groove_genre_id": style["groove_id"],
            "repo": "gradetwo/groove",
            "license": "MIT",
            "converted": CONVERTED_DATE,
            "tuning": tuning,
        },
        "kit_family": style["kit_family"],
        "bpm": style["bpm"],
        "swing_permille": style["swing_permille"],
        "bars": 2,
        "lanes": lanes_json(lanes, 32),
        "fill": {"lanes": lanes_json(fill_lanes, 16)},
    }
    return doc


def main():
    OUT_DIR.mkdir(parents=True, exist_ok=True)
    for style in STYLES:
        doc = convert(style)
        path = OUT_DIR / f"{style['id']}.json"
        text = json.dumps(doc, ensure_ascii=False, indent=2) + "\n"
        path.write_text(text, encoding="utf-8")
        print(f"wrote {path}")


if __name__ == "__main__":
    main()
