#!/usr/bin/env python3
"""为 `yeban_import_audio` 的端到端复跑准备**两个确定性的输入**。

这个脚本**不是**被测代码，它是判据/复跑的**夹具生成器**（与
`crates/yeban-mcp/verify/render_pure.rs` 同一定位：本机可执行、零第三方依赖）。

```text
python3 crates/yeban-mcp/verify/e2e_audio_clip_seed.py --out target/e2e-audio-clip
```

写出三件东西：

| 文件 | 内容 | 用途 |
| :--- | :--- | :--- |
| `seed-clean.yeban` | 模型样本 `project.filled.json` **去掉**那条"只声明、没有字节"的音频资产（片段 + 摆放 + `assets` 索引项），打进 `ARCH-SEC-003` 容器（`project.json` + `history.dag`，`stored`） | 端到端主链路的起点：有音轨、有 MIDI 材料、**没有**悬空的资产声明 |
| `seed-dangling.yeban` | 同一个样本**原样**（`assets` 索引声明了 `8bce…2c33`，容器里没有 `assets/8bce…2c33`） | 证明渲染的 `bytesPresent` 判据读的是**会话 CAS 池**，不是 `project.assets` 索引 |
| `kick.wav` | 48 kHz / 单声道 / 16-bit PCM，样本由公式生成（不用随机数） | `yeban_import_audio` 的 `path` 来源 |

## 为什么主链路起点要"去掉那条声明"

`project.filled.json` 里的音频片段指向 `AssetHash::of_bytes(b"yeban-kick-sample")` ——
那 17 个字节**不是**音频容器，样本里也没有它的载荷。把它留在起点，渲染就会如实地为
它报一次 `bytesPresent: false`；而端到端判据要证明的是"**本次导入的**片段被真的渲染"，
所以起点必须**没有**悬空声明。这不是掩盖缺陷：`seed-dangling.yeban` 与既有判据
`tests/render_audio_clips.rs::a_declared_asset_without_payload_is_registered_rather_than_faked`
都保留着那条如实上报的路径。

## 确定性

两个容器都由同一份 JSON 文本派生；ZIP 条目按**固定顺序**写、压缩法固定为 `stored`
（`yeban_model::container` 的读取路径**拒绝** `deflate` 与 data descriptor）。
WAV 的样本值是帧号的纯函数，因此本脚本两次运行的输出逐字节相同。
"""

import argparse
import hashlib
import json
import pathlib
import struct
import sys
import zipfile

# `yeban-model` 的容器布局常量（`crates/yeban-model/src/container/mod.rs`）。
PROJECT_JSON_NAME = "project.json"
HISTORY_DAG_NAME = "history.dag"

# 样本里那条"只声明、没有字节"的音频资产（见模块文档）。
DANGLING_HASH = "8bce5ff4ef4d123c308027f18c51101d4925d1194c3f4ef98248923cde682c33"
AUDIO_CLIP_ID = "01J8ZQ00000000000000000011"
AUDIO_PLACEMENT_ID = "01J8ZQ00000000000000000051"
AUDIO_TRACK_ID = "01J8ZQ00000000000000000003"

# WAV 参数（写进报告的常量）。
WAV_SAMPLE_RATE = 48_000
WAV_FRAMES = 48_000  # 1 s（@128 BPM / 960 PPQ = 2048 tick，正好是"素材全长"缺省时值）
WAV_AMPLITUDE = 12_000


def default_project_json() -> pathlib.Path:
    """模型规范样本的位置（由 `cargo run -p yeban-model --example export_schema_samples` 产出）。"""
    repo = pathlib.Path(__file__).resolve().parents[3]
    return repo / "target" / "schema-samples" / "project.filled.json"


def strip_dangling_audio(project: dict) -> dict:
    """去掉那条"只声明、没有字节"的音频资产（片段 + 摆放 + `assets` 索引项）。

    只删键、不改键序：`project.json` 的字段顺序因此与模型写出的一致。
    """
    pool = project["clip_pool"]
    if AUDIO_CLIP_ID not in pool:
        raise SystemExit(f"样本里没有音频片段 {AUDIO_CLIP_ID}（样本已变，请更新本脚本）")
    del pool[AUDIO_CLIP_ID]

    clips = project["tracks"][AUDIO_TRACK_ID]["clips"]
    if AUDIO_PLACEMENT_ID not in clips:
        raise SystemExit(f"样本里没有音频摆放 {AUDIO_PLACEMENT_ID}")
    del clips[AUDIO_PLACEMENT_ID]

    assets = project["assets"]
    if DANGLING_HASH not in assets:
        raise SystemExit(f"样本里没有资产声明 {DANGLING_HASH}")
    del assets[DANGLING_HASH]
    return project


def write_container(path: pathlib.Path, project: dict) -> bytes:
    """按 `ARCH-SEC-003` 的布局写一个 `.yeban` 容器（`project.json` + `history.dag`）。

    压缩法固定 `stored`：模型层的读取路径拒绝 `deflate`（本线不引入 `flate2`）。
    """
    project_json = json.dumps(
        project, ensure_ascii=False, separators=(",", ":")
    ).encode("utf-8")
    history_dag = b""  # 空图谱 ⇒ 打开时建一条根提交（`store::decode_history_dag`）
    with zipfile.ZipFile(path, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, payload in ((PROJECT_JSON_NAME, project_json), (HISTORY_DAG_NAME, history_dag)):
            info = zipfile.ZipInfo(name, date_time=(1980, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_STORED
            info.external_attr = 0o644 << 16
            archive.writestr(info, payload)
    return path.read_bytes()


def wav_s16() -> bytes:
    """48 kHz / 单声道 / 16-bit PCM；样本值 = 帧号的纯函数（无随机数）。"""
    samples = []
    for frame in range(WAV_FRAMES):
        # 两个整数频率的叠加，再取整：同一帧号永远给出同一个样本。
        value = (frame * 7) % 200 - 100
        samples.append(value * (WAV_AMPLITUDE // 100))
    data = b"".join(struct.pack("<h", sample) for sample in samples)
    header = b"".join(
        [
            b"RIFF",
            struct.pack("<I", 36 + len(data)),
            b"WAVEfmt ",
            struct.pack("<IHHIIHH", 16, 1, 1, WAV_SAMPLE_RATE, WAV_SAMPLE_RATE * 2, 2, 16),
            b"data",
            struct.pack("<I", len(data)),
        ]
    )
    return header + data


def sha256(payload: bytes) -> str:
    return hashlib.sha256(payload).hexdigest()


def main() -> int:
    parser = argparse.ArgumentParser(description="生成音频片段端到端复跑的夹具")
    parser.add_argument("--out", required=True, help="输出目录（会被创建）")
    parser.add_argument(
        "--project-json",
        default=None,
        help="模型样本 project.filled.json 的路径（缺省 = <repo>/target/schema-samples/…）",
    )
    args = parser.parse_args()

    out = pathlib.Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    source = pathlib.Path(args.project_json) if args.project_json else default_project_json()
    if not source.is_file():
        print(
            f"缺少模型样本 {source}；先跑 `cargo run -p yeban-model --example export_schema_samples`",
            file=sys.stderr,
        )
        return 2

    raw = json.loads(source.read_text(encoding="utf-8"))

    dangling = out / "seed-dangling.yeban"
    dangling_bytes = write_container(dangling, json.loads(json.dumps(raw)))

    clean_project = strip_dangling_audio(json.loads(json.dumps(raw)))
    clean = out / "seed-clean.yeban"
    clean_bytes = write_container(clean, clean_project)

    kick = out / "kick.wav"
    kick_bytes = wav_s16()
    kick.write_bytes(kick_bytes)

    print(f"project.json source : {source}")
    print(f"out dir             : {out}")
    for path, payload in ((dangling, dangling_bytes), (clean, clean_bytes), (kick, kick_bytes)):
        print(f"{path.name:20s} bytes={len(payload):8d} sha256={sha256(payload)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
