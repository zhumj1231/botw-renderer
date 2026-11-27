"""Trigger and verify one local capture with RenderDoc's bundled Python."""
import ctypes
import json
import os
from pathlib import Path
import sys
import time
import traceback
import renderdoc as rd

report = Path(os.environ["BOTW_TC_REPORT"])
expected_pid = int(os.environ["BOTW_TC_PID"])

def foreground():
    user32 = ctypes.windll.user32
    user32.GetForegroundWindow.restype = ctypes.c_void_p
    window = user32.GetForegroundWindow()
    pid = ctypes.c_uint32(0)
    if window:
        user32.GetWindowThreadProcessId(ctypes.c_void_p(window), ctypes.byref(pid))
    return int(window or 0), pid.value

def connect():
    ident = 0
    while True:
        ident = rd.EnumerateRemoteTargets("localhost", ident)
        if not ident:
            raise RuntimeError("No free RenderDoc target for PID %d" % expected_pid)
        target = rd.CreateTargetControl("localhost", ident, "BotW frame research", False)
        if not target:
            continue
        if target.Connected() and target.GetPID() == expected_pid:
            return target, ident
        target.Shutdown()

def trigger():
    target, ident = connect()
    try:
        name = target.GetTarget()
        if "cemu" not in name.lower():
            raise RuntimeError("Expected Cemu, got " + name)
        deadline = time.monotonic() + 15
        while target.Connected() and not target.GetAPI() and time.monotonic() < deadline:
            target.ReceiveMessage(None)
        api = target.GetAPI()
        if "vulkan" not in api.lower():
            raise RuntimeError("Expected Vulkan, got " + api)
        before = foreground()
        if os.environ.get("BOTW_TC_REQUIRE_BACKGROUND"):
            deadline = time.monotonic() + 40
            while before[1] in (0, expected_pid) and time.monotonic() < deadline:
                target.ReceiveMessage(None)
                time.sleep(0.1)
                before = foreground()
            if before[1] in (0, expected_pid):
                raise RuntimeError("Cemu never entered the background during the test window")
        trigger_at = time.time()
        target.TriggerCapture(1)
        print("Triggered", ident, name, api, "foreground PID", before[1], flush=True)
        deadline = time.monotonic() + 30
        while target.Connected() and time.monotonic() < deadline:
            msg = target.ReceiveMessage(None)
            if msg.type != rd.TargetControlMessageType.NewCapture:
                continue
            capture = Path(msg.newCapture.path)
            if not capture.is_file() or capture.stat().st_mtime < trigger_at:
                continue
            after = foreground()
            return {"pid": expected_pid, "target": name, "control_ident": ident, "api": api,
                    "file": str(capture), "capture_id": msg.newCapture.captureId,
                    "foreground_pids": [before[1], after[1]], "same_foreground_window": before == after,
                    "cemu_background_at_both_checks": expected_pid not in (before[1], after[1])}
        raise RuntimeError("No new local capture received after TriggerCapture")
    finally:
        target.Shutdown()

def walk(items):
    for item in items:
        yield item
        yield from walk(item.children)

def verify(result):
    capture = rd.OpenCaptureFile()
    controller = None
    try:
        status = capture.OpenFile(result["file"], "", None)
        if status != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenFile failed: " + str(status))
        status, controller = capture.OpenCapture(rd.ReplayOptions(), None)
        if status != rd.ResultCode.Succeeded:
            raise RuntimeError("OpenCapture failed: " + str(status))
        actions = list(walk(controller.GetRootActions()))
        draws = sum(bool(a.flags & rd.ActionFlags.Drawcall) for a in actions)
        presents = [a for a in actions if a.flags & rd.ActionFlags.Present]
        if not draws or not presents:
            raise RuntimeError("Capture did not contain draw calls and a Present")
        result.update(draws=draws, presents=[a.eventId for a in presents],
                      bytes=Path(result["file"]).stat().st_size)
        out = presents[-1].copyDestination
        dim = next((t for t in controller.GetTextures() if t.resourceId == out), None)
        if dim:
            result["backbuffer"] = [dim.width, dim.height, dim.format.Name()]
    finally:
        if controller:
            controller.Shutdown()
        capture.Shutdown()

with report.with_suffix(".log").open("w",encoding="utf-8",buffering=1) as log:
    sys.stdout = sys.stderr = log
    try:
        result = trigger()
        verify(result)
        report.write_text(json.dumps(result,indent=2),encoding="utf-8")
        print(result,flush=True)
    except Exception:
        traceback.print_exc()
        sys.exit(1)
sys.exit(0)
