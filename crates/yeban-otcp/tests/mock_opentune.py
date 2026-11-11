#!/usr/bin/env python3
"""Mock OpenTune Standalone：供 yeban-otcp 集成测试用的假服务端。

用法（驱动按设计稿 §2.1 的参数启动）：
    mock_opentune.py --control-api \
        --control-api-dir=<dir> \
        --control-api-ready-file=<dir>/ready.json \
        --control-api-launch-nonce=<nonce>

行为由场景文件控制：<control_root>/mock-scenario.json，其中
control_root 是 --control-api-dir 的父目录：
    {"mode": "happy" | "render_fail" | "modal" | "dialog_commits"
             | "revision_conflict" | "f0_slow" | "export_off_pitch",
     "build_commit": "mock-build-001"}

方法调用轨迹写进 <control_root>/mock-trace-<job_id>.jsonl（每行一个
JSON），测试据此断言模态框解散 / app.quit 等行为。

只用标准库。⛔ 不是 OpenTune 代码：它是测试替身，不进任何发布物。
"""

import json
import math
import os
import socket
import struct
import sys
import time
import wave

REQUIRED_CAPS = [
    "session.v1", "read.v1", "jobs.v1", "import.v1",
    "export.v1", "ui.bridge", "edit.v1",
]

NOTES_BEFORE = [
    {"index": 0, "label": "A4", "startContentSec": 0.0, "endContentSec": 1.0,
     "startTimelineSec": 0.0, "endTimelineSec": 1.0,
     "detectedMidi": 69.4, "targetMidi": 69.0, "offsetCents": 40.0,
     "retuneSpeed": 0.5, "vibratoDepth": 0.0, "vibratoRateHz": 5.0,
     "pitchDriftScale": 1.0, "voiced": True, "corrected": False},
    {"index": 1, "label": "B4", "startContentSec": 1.0, "endContentSec": 2.0,
     "startTimelineSec": 1.0, "endTimelineSec": 2.0,
     "detectedMidi": 71.2, "targetMidi": 71.0, "offsetCents": 20.0,
     "retuneSpeed": 0.5, "vibratoDepth": 0.0, "vibratoRateHz": 5.0,
     "pitchDriftScale": 1.0, "voiced": True, "corrected": False},
]

NOTES_AFTER = [
    dict(n, targetMidi=n["targetMidi"], offsetCents=0.0, corrected=True)
    for n in NOTES_BEFORE
]


def rpc_error(code, kind, reason, detail, retryable=False, executed=False, extra=None):
    data = {"kind": kind, "reason": reason, "retryable": retryable,
            "detail": detail, "executed": executed}
    if extra:
        data.update(extra)
    return {"code": code, "message": kind, "data": data}


def write_wav_float32(path, sample_rate, freqs):
    """freqs: [(start_sec, end_sec, freq_hz), ...]，写 float32 单声道 WAV。"""
    frames = []
    total = max(end for _, end, _ in freqs)
    count = int(total * sample_rate)
    for i in range(count):
        t = i / sample_rate
        freq = freqs[-1][2]
        for start, end, f in freqs:
            if start <= t < end:
                freq = f
                break
        frames.append(0.5 * math.sin(2 * math.pi * freq * t))
    data = b"".join(struct.pack("<f", s) for s in frames)
    header = b"RIFF" + struct.pack("<I", 36 + len(data)) + b"WAVE"
    header += b"fmt " + struct.pack("<IHHIIHH", 16, 3, 1, sample_rate,
                                    sample_rate * 4, 4, 32)
    header += b"data" + struct.pack("<I", len(data))
    with open(path, "wb") as fh:
        fh.write(header + data)


class MockServer:
    def __init__(self, control_dir, ready_file, nonce, scenario):
        self.control_dir = control_dir
        self.ready_file = ready_file
        self.nonce = nonce
        self.scenario = scenario
        self.mode = scenario.get("mode", "happy")
        self.build_commit = scenario.get("build_commit", "mock-build-001")
        # control_dir 是 <control_root>/control/<job_id>，故控制根是上两级。
        self.control_root = os.path.dirname(os.path.dirname(control_dir))
        self.job_id = os.path.basename(control_dir)
        self.trace_path = os.path.join(
            self.control_root, f"mock-trace-{self.job_id}.jsonl")
        self.token = "mock-token-xyz"
        self.server_proof = "mock-proof-xyz"
        self.instance_id = "mock-inst-1"
        self.port = None
        self.pid = os.getpid()
        self.jobs = {}          # jobId -> {"kind", "polls", "result"|"error"}
        self.contents = {}      # contentId -> {"revision", "autotuned", "f0_polls"}
        self.last_content_id = None
        self.import_count = 0
        self.source_duration = 2.0
        self.modal_open = False
        self.dialogs = []
        # 每个 content 各触发一次场景行为（自测与任务是不同的 content，
        # 各自走一遍模态 / 冲突路径）。
        self.auto_tune_seen = set()

    def trace(self, method, extra=None):
        entry = {"method": method, "ts": time.time()}
        if extra:
            entry.update(extra)
        with open(self.trace_path, "a") as fh:
            fh.write(json.dumps(entry) + "\n")

    def serve(self):
        sock = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        sock.bind(("127.0.0.1", 0))
        sock.listen(1)
        self.port = sock.getsockname()[1]
        discovery_name = f"{self.pid}.json"
        now_ms = int(time.time() * 1000)
        # 发现文件：按协议 §5.4 的 14 个成员全写，与 9ef45f9 真二进制
        # 在 yanshi-5 上的实测形状一致（build 是对象 {commit, tag}）。
        discovery = {
            "protocolMajors": [1], "pid": self.pid,
            "processStartTimeMs": now_ms,
            "instanceId": self.instance_id,
            "port": self.port,
            "token": self.token, "serverProof": self.server_proof,
            "launchNonce": self.nonce,
            "appVersion": "1.8.2",
            "build": {"commit": self.build_commit, "tag": None},
            "buildTag": None,
            "sourceUrl": "https://github.com/YuFeng926/OpenTune",
            "startedAtMs": now_ms,
            "flavor": "standalone",
        }
        with open(os.path.join(self.control_dir, discovery_name), "w") as fh:
            json.dump(discovery, fh)
        # ready 文件：协议 §5.5 闭合，只含 {pid, instanceId}，无端口。
        with open(self.ready_file, "w") as fh:
            json.dump({"pid": self.pid,
                       "instanceId": self.instance_id}, fh)
        conn, _ = sock.accept()
        stream = conn.makefile("rwb")
        while True:
            line = stream.readline()
            if not line:
                break
            try:
                req = json.loads(line.decode("utf-8"))
            except json.JSONDecodeError:
                continue
            req_id = req.get("id")
            method = req.get("method", "")
            params = req.get("params", {})
            self.trace(method)
            result, error = self.dispatch(method, params)
            if method == "app.quit":
                resp = {"jsonrpc": "2.0", "id": req_id, "result": result}
                stream.write((json.dumps(resp) + "\n").encode("utf-8"))
                stream.flush()
                time.sleep(0.1)
                conn.close()
                sock.close()
                sys.exit(0)
            if error is not None:
                resp = {"jsonrpc": "2.0", "id": req_id, "error": error}
            else:
                resp = {"jsonrpc": "2.0", "id": req_id, "result": result}
            stream.write((json.dumps(resp) + "\n").encode("utf-8"))
            stream.flush()
        conn.close()
        sock.close()

    def dispatch(self, method, params):
        if method == "session.hello":
            if params.get("token") != self.token:
                return None, rpc_error(-32000, "INVALID_ARGUMENT", "bad_token",
                                       "token 不符")
            return {"protocol": "1.0", "instanceId": self.instance_id,
                    "pid": self.pid, "serverProof": self.server_proof,
                    "appVersion": "mock-1.0", "flavor": "standalone",
                    "capabilities": REQUIRED_CAPS + ["prefs.write"],
                    "limits": {"maxJobWaitMs": 30000}}, None
        if method == "session.info":
            return {"build": {"commit": self.build_commit},
                    "flavor": "standalone",
                    "capabilities": REQUIRED_CAPS + ["prefs.write"],
                    "processing": {"vocoderWeight": "hifigan.onnx",
                                   "hybridMode": True, "backend": "cpu",
                                   "tuningHz": 440, "vocoderReady": True,
                                   "vocoderReconfiguring": False},
                    "editingScheme": "openDyne",
                    "import": {"formats": ["wav"]},
                    "export": {"formats": ["wav"]},
                    "limits": {"maxJobWaitMs": 30000}}, None
        if method == "import.start":
            self.import_count += 1
            content_id = f"c-mock-{self.import_count}"
            path = params.get("items", [{}])[0].get("path", "")
            try:
                with wave.open(path, "rb") as wav:
                    self.source_duration = wav.getnframes() / wav.getframerate()
            except (wave.Error, OSError, FileNotFoundError):
                pass
            self.contents[content_id] = {"revision": "rev-1",
                                         "autotuned": False, "f0_polls": 0}
            self.last_content_id = content_id
            job_id = f"job-import-{self.import_count}"
            self.jobs[job_id] = {"kind": "import", "polls": 0,
                                 "result": {}, "contentId": content_id}
            return {"jobId": job_id}, None
        if method == "project.get":
            contents = [{"contentId": "c-demo"}]
            if self.last_content_id:
                contents.append({"contentId": self.last_content_id})
            return {"contents": contents}, None
        if method == "content.getDetails":
            content_id = params.get("contentId", "")
            state = self.contents.get(content_id)
            if state is None:
                return None, rpc_error(-32000, "NOT_FOUND", "stale_handle",
                                       "content 不存在")
            state["f0_polls"] += 1
            if self.mode == "f0_slow" and state["f0_polls"] <= 2:
                return {"analysis": {"f0State": "analyzing"},
                        "viaPlacementId": "p:mock:0:1"}, None
            return {"analysis": {"f0State": "ready"},
                    "viaPlacementId": "p:mock:0:1"}, None
        if method == "content.getNotes":
            content_id = params.get("contentId", "")
            state = self.contents.get(content_id)
            if state is None:
                return None, rpc_error(-32000, "NOT_FOUND", "stale_handle",
                                       "content 不存在")
            notes = NOTES_AFTER if state["autotuned"] else NOTES_BEFORE
            return {"asOf": {"arrangementRevision": "arr-1",
                             "contents": [{"contentId": content_id,
                                           "revision": state["revision"],
                                           "gridRevision": "g1",
                                           "keyRevision": "k1"}]},
                    "derived": [], "contentId": content_id,
                    "placementId": "p:mock:0:1", "tuningHz": 440,
                    "totalCount": len(notes), "notes": notes,
                    "nextCursor": None}, None
        if method == "content.autoTune":
            content_id = params.get("contentId", "")
            state = self.contents.get(content_id)
            if state is None:
                return None, rpc_error(-32000, "NOT_FOUND", "stale_handle",
                                       "content 不存在")
            first_for_content = content_id not in self.auto_tune_seen
            self.auto_tune_seen.add(content_id)
            if first_for_content and self.mode == "modal":
                self.modal_open = True
                self.dialogs = [{"kind": "message", "closeCommits": False}]
                return None, rpc_error(-32000, "BUSY", "modal_open",
                                       "模态框打开")
            if first_for_content and self.mode == "dialog_commits":
                self.modal_open = True
                self.dialogs = [{"kind": "confirm", "closeCommits": True}]
                return None, rpc_error(-32000, "BUSY", "modal_open",
                                       "模态框打开")
            if first_for_content and self.mode == "revision_conflict":
                state["revision"] = "rev-1b"
                return None, rpc_error(-32000, "REVISION_CONFLICT",
                                       "revision_conflict", "修订已变")
            if params.get("expectedRevision") != state["revision"]:
                return None, rpc_error(-32000, "REVISION_CONFLICT",
                                       "revision_conflict", "修订已变")
            state["autotuned"] = True
            state["revision"] = "rev-2"
            return {"applied": True, "noOp": False, "dryRun": False,
                    "changes": [0, 1], "created": [], "removed": [],
                    "idMap": [], "revisionDiff": {}, "tokens": {
                        "arrangementRevision": "arr-2",
                        "contents": [{"contentId": content_id,
                                      "revision": "rev-2"}],
                        "placements": []},
                    "undo": {}, "dirty": True, "ui": {},
                    "sideEffects": {}, "warnings": [], "job": None,
                    "omittedCount": 0,
                    "mode": "snapNotes",
                    "keyUsed": params.get("key", {}),
                    "paramsUsed": {}, "noteCount": {"before": 2, "after": 2}}, None
        if method == "content.editNotes":
            content_id = params.get("contentId", "")
            state = self.contents.get(content_id)
            if state is None:
                return None, rpc_error(-32000, "NOT_FOUND", "stale_handle",
                                       "content 不存在")
            if params.get("expectedRevision") != state["revision"]:
                return None, rpc_error(-32000, "REVISION_CONFLICT",
                                       "revision_conflict", "修订已变")
            dry_run = bool(params.get("dryRun"))
            if not dry_run:
                state["revision"] = "rev-3"
            return {"applied": not dry_run, "noOp": False, "dryRun": dry_run,
                    "changes": [], "created": [], "removed": [],
                    "idMap": [], "revisionDiff": {}, "tokens": {
                        "arrangementRevision": "arr-2",
                        "contents": [{"contentId": content_id,
                                      "revision": state["revision"]}],
                        "placements": []},
                    "undo": {}, "dirty": True, "ui": {},
                    "sideEffects": {}, "warnings": [], "job": None,
                    "omittedCount": 0}, None
        if method == "export.start":
            job_id = "job-export-1"
            self.jobs[job_id] = {"kind": "export", "polls": 0,
                                 "path": params.get("path", ""),
                                 "contentId": self.last_content_id}
            return {"jobId": job_id}, None
        if method == "job.get":
            job = self.jobs.get(params.get("jobId", ""))
            if job is None:
                return None, rpc_error(-32000, "NOT_FOUND", "job_not_found",
                                       "任务不存在")
            job["polls"] += 1
            if job["kind"] == "import":
                if job["polls"] < 2:
                    return {"job": {"jobId": params["jobId"], "kind": "import",
                                    "state": "running", "cancellable": True,
                                    "createdAtMs": 0, "updatedAtMs": 0,
                                    "finishedAtMs": None, "progress": None,
                                    "result": None, "error": None}}, None
                return {"job": {"jobId": params["jobId"], "kind": "import",
                                "state": "succeeded", "cancellable": False,
                                "createdAtMs": 0, "updatedAtMs": 0,
                                "finishedAtMs": 1, "progress": None,
                                "result": job["result"], "error": None}}, None
            # export job
            if self.mode == "render_fail":
                if job["polls"] < 2:
                    return {"job": {"jobId": params["jobId"], "kind": "export",
                                    "state": "running", "cancellable": True,
                                    "createdAtMs": 0, "updatedAtMs": 0,
                                    "finishedAtMs": None, "progress": None,
                                    "result": None, "error": None}}, None
                self.modal_open = True
                self.dialogs = [{"kind": "message", "closeCommits": False}]
                error = rpc_error(
                    -32000, "RENDER_FAILED", "render_chunk_failed",
                    "a render block of a content of the export failed twice",
                    retryable=False, executed=False,
                    extra={"block": {"startSample": 128000,
                                     "expectedSamples": 27136,
                                     "actualSamples": 0}})
                return {"job": {"jobId": params["jobId"], "kind": "export",
                                "state": "failed", "cancellable": False,
                                "createdAtMs": 0, "updatedAtMs": 0,
                                "finishedAtMs": 1, "progress": None,
                                "result": None, "error": error["data"]}}, None
            if job["polls"] < 2:
                return {"job": {"jobId": params["jobId"], "kind": "export",
                                "state": "running", "cancellable": True,
                                "createdAtMs": 0, "updatedAtMs": 0,
                                "finishedAtMs": None, "progress": None,
                                "result": None, "error": None}}, None
            content_id = job.get("contentId") or self.last_content_id
            revision = self.contents.get(content_id, {}).get("revision", "rev-2")
            path = job.get("path", "")
            if self.mode == "export_off_pitch":
                write_wav_float32(path, 44100,
                                  [(0.0, self.source_duration, 466.16)])
            else:
                half = self.source_duration / 2.0
                write_wav_float32(path, 44100,
                                  [(0.0, half, 440.0),
                                   (half, self.source_duration, 493.88)])
            result = {"path": path, "sampleRateHz": 44100, "channelCount": 1,
                      "sampleFormat": "float32",
                      "durationSec": self.source_duration,
                      "startTimelineSec": 0.0,
                      "contents": [{"contentId": content_id,
                                    "revision": revision}],
                      "dryContents": [],
                      "processing": {"vocoderWeight": "hifigan.onnx",
                                     "hybridMode": True, "backend": "cpu",
                                     "tuningHz": 440}}
            return {"job": {"jobId": params["jobId"], "kind": "export",
                            "state": "succeeded", "cancellable": False,
                            "createdAtMs": 0, "updatedAtMs": 0,
                            "finishedAtMs": 1, "progress": None,
                            "result": result, "error": None}}, None
        if method == "job.cancel":
            return {"jobId": params.get("jobId"), "cancelled": True}, None
        if method == "ui.getState":
            return {"busy": {"pointerDown": False,
                             "modalOpen": self.modal_open,
                             "importInProgress": False,
                             "exportInProgress": False,
                             "projectOperation": False},
                    "dialogs": self.dialogs,
                    "onboarding": {}, "ui": {}, "uiScale": 1.0}, None
        if method == "ui.dismiss":
            for dialog in self.dialogs:
                if dialog.get("closeCommits"):
                    return None, rpc_error(
                        -32000, "INVALID_ARGUMENT", "dialog_commits_on_close",
                        "关闭该对话框会提交")
            closed = [d.get("kind", "") for d in self.dialogs]
            self.dialogs = []
            self.modal_open = False
            return {"sideEffects": {"dialogClosed": closed}}, None
        if method == "app.quit":
            return {"quitting": True, "discardedUnsavedChanges": True,
                    "projectPath": None, "exitDeadlineMs": 5000,
                    "warnings": []}, None
        return None, rpc_error(-32601, "UNSUPPORTED", "unknown_method",
                               f"未知方法 {method}")


def main():
    args = sys.argv[1:]
    opts = {}
    i = 0
    while i < len(args):
        if args[i].startswith("--"):
            key = args[i][2:]
            if "=" in key:
                name, value = key.split("=", 1)
                opts[name] = value
            elif i + 1 < len(args) and not args[i + 1].startswith("--"):
                opts[key] = args[i + 1]
                i += 1
            else:
                # 无值开关（如 --control-api）：不吃下一个参数。
                opts[key] = "true"
        i += 1
    control_dir = opts["control-api-dir"]
    ready_file = opts["control-api-ready-file"]
    nonce = opts["control-api-launch-nonce"]
    # control_dir 是 <control_root>/control/<job_id>，故控制根是上两级。
    control_root = os.path.dirname(os.path.dirname(control_dir))
    scenario_path = os.path.join(control_root, "mock-scenario.json")
    scenario = {}
    if os.path.exists(scenario_path):
        with open(scenario_path) as fh:
            scenario = json.load(fh)
    server = MockServer(control_dir, ready_file, nonce, scenario)
    server.serve()


if __name__ == "__main__":
    main()
