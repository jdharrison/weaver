
"""Real, local-only Firefox startup smoke; stdlib only, no multiplayer interaction.

Run under the demo Hosting emulator, for example from target/local-browser-qa:
  CI=true FIREBASE_CLI_DISABLE_UPDATE_CHECK=1 firebase emulators:exec \
    --config ../../firebase.json --project demo-weaver-portfolio --only hosting \
    "python3 ../../scripts/test-browser-local.py"

No generated source/build changes. Logs, screenshot, and transient fresh profiles
are confined to target/local-browser-qa. Never uses port 8000 or existing browsers.
"""

import base64
import binascii
import hashlib
import http.client
import http.server
import json
import os
import signal
import socket
import struct
import subprocess
import sys
import tempfile
import threading
import time
from html.parser import HTMLParser
from pathlib import Path
from urllib.parse import urlsplit

ROOT = Path(__file__).resolve().parents[1]
OUTPUT = ROOT / "target" / "local-browser-qa"
URL = "http://127.0.0.1:8002/"
TOTAL_SECONDS = 65
SNAPSHOT = """
const status = document.getElementById('status');
const network = document.getElementById('network-status');
const rect = network?.getBoundingClientRect();
const style = network && getComputedStyle(network);
const canvas = document.getElementById('scene');
return {
  url: location.href, status: status?.textContent ?? '',
  networkStatus: network?.textContent ?? '',
  networkVisible: !!rect && rect.width > 0 && rect.height > 0 &&
    rect.bottom > 0 && rect.top < innerHeight && style.display !== 'none' &&
    style.visibility !== 'hidden' && !network.closest('details'),
  settingsOpen: document.getElementById('multiplayer-settings')?.open,
  joinDisabled: document.getElementById('join-network')?.disabled,
  operatorConnectDisabled: document.getElementById('connect-network')?.disabled,
  disconnectDisabled: document.getElementById('disconnect-network')?.disabled,
  reloadHidden: document.getElementById('reload-scene')?.hidden,
  secureContext: isSecureContext,
  webTransportAvailable: typeof WebTransport === 'function',
  sceneCallbacksDefined: typeof weaverSceneReady === 'function' && typeof weaverSceneFatal === 'function',
  bootstrapDisabled: ['', 'off'].includes(document.querySelector('meta[name="weaver-lobby-bootstrap"]')?.content),
  canvas: canvas ? { width: canvas.width, height: canvas.height } : null,
  resources: performance.getEntriesByType('resource').map(e => ({
    url: e.name, initiatorType: e.initiatorType, responseStatus: e.responseStatus ?? null,
    transferSize: e.transferSize, durationMs: Math.round(e.duration)
  }))
};
"""


class BrowserSmokeError(RuntimeError):
    """Expected browser/protocol/preflight failure, not a programming error."""


EXPECTED_BROWSER_ERRORS = (
    BrowserSmokeError,
    OSError,
    http.client.HTTPException,
    json.JSONDecodeError,
    UnicodeDecodeError,
)


class Metas(HTMLParser):
    def __init__(self):
        super().__init__()
        self.values = {}

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        name = attrs.get("name")
        if tag == "meta" and name is not None and name.startswith("weaver-lobby-"):
            if name in self.values:
                raise BrowserSmokeError("Duplicate lobby meta; refusing browser startup")
            self.values[name] = attrs.get("content", "")


def free_port():
    with socket.socket() as bound:
        bound.bind(("127.0.0.1", 0))
        return bound.getsockname()[1]


def http_json(port, method, path, body=None, timeout: float = 5.0, deadline=None):
    if deadline is not None:
        timeout = min(timeout, max(0.1, deadline - time.monotonic()))
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=timeout)
    try:
        data = None if body is None else json.dumps(body).encode()
        connection.request(method, path, body=data, headers={"Content-Type": "application/json"})
        response = connection.getresponse()
        data = response.read(4 * 1024 * 1024 + 1)
        if len(data) > 4 * 1024 * 1024:
            raise BrowserSmokeError("WebDriver response exceeded the local QA limit")
        decoded = json.loads(data)
        if response.status >= 400:
            value = decoded.get("value", {})
            raise BrowserSmokeError(f"WebDriver {response.status}: {value.get('error')}: {value.get('message')}")
        return decoded.get("value", decoded)
    finally:
        connection.close()


class DenyProxy(http.server.BaseHTTPRequestHandler):
    """Owned proxy rejects browser background HTTP(S) without forwarding traffic."""
    attempts = 0

    def reject(self):
        type(self).attempts += 1
        self.send_error(403, "Non-loopback network disabled for local browser QA")
        self.close_connection = True

    do_CONNECT = reject
    do_GET = reject
    do_POST = reject
    do_HEAD = reject

    def log_message(self, format: str, *args: object) -> None:
        pass


class BrowserEvents:
    """Minimal loopback WebDriver BiDi WebSocket for console/network observation."""
    def __init__(self, url):
        address = urlsplit(url)
        if address.scheme != "ws" or address.hostname != "127.0.0.1" or not address.port:
            raise BrowserSmokeError("Refusing a non-loopback WebDriver BiDi URL")
        self.connection = socket.create_connection(("127.0.0.1", address.port), timeout=3)
        self.connection.settimeout(0.15)
        self.buffer = bytearray()
        self.entries = []
        self.requests = []
        self.subscribed = False
        key = base64.b64encode(os.urandom(16)).decode()
        self.connection.sendall((
            f"GET {address.path} HTTP/1.1\r\nHost: 127.0.0.1:{address.port}\r\n"
            f"Upgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\n"
            "Sec-WebSocket-Version: 13\r\n\r\n"
        ).encode())
        self.connection.settimeout(3)
        while b"\r\n\r\n" not in self.buffer:
            packet = self.connection.recv(4096)
            if not packet:
                raise BrowserSmokeError("WebDriver BiDi closed during its handshake")
            self.buffer.extend(packet)
            if len(self.buffer) > 16_384:
                raise BrowserSmokeError("Oversize BiDi handshake")
        headers, remaining = bytes(self.buffer).split(b"\r\n\r\n", 1)
        expected = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest())
        if b" 101 " not in headers.split(b"\r\n", 1)[0] or expected not in headers:
            raise BrowserSmokeError("WebDriver BiDi handshake failed")
        self.buffer = bytearray(remaining)
        self.connection.settimeout(0.15)
        self.send(json.dumps({"id": 1, "method": "session.subscribe", "params": {
            "events": ["log.entryAdded", "network.beforeRequestSent"]
        }}).encode())
        until = time.monotonic() + 3
        while not self.subscribed and time.monotonic() < until:
            self.drain()
        if not self.subscribed:
            raise BrowserSmokeError("BiDi observation subscription did not complete")

    def send(self, payload, opcode=1):
        mask = os.urandom(4)
        size = len(payload)
        header = bytes([0x80 | opcode])
        if size < 126:
            header += bytes([0x80 | size])
        elif size < 65_536:
            header += bytes([0x80 | 126]) + struct.pack("!H", size)
        else:
            header += bytes([0x80 | 127]) + struct.pack("!Q", size)
        encoded = bytes(value ^ mask[index % 4] for index, value in enumerate(payload))
        self.connection.sendall(header + mask + encoded)

    def drain(self):
        try:
            packet = self.connection.recv(65_536)
            if packet:
                self.buffer.extend(packet)
        except TimeoutError:
            pass
        processed = 0
        while len(self.buffer) >= 2 and processed < 100:
            first, second = self.buffer[:2]
            size, offset = second & 0x7F, 2
            if size == 126:
                if len(self.buffer) < 4:
                    break
                size, offset = struct.unpack("!H", self.buffer[2:4])[0], 4
            elif size == 127:
                if len(self.buffer) < 10:
                    break
                size, offset = struct.unpack("!Q", self.buffer[2:10])[0], 10
            if size > 1024 * 1024:
                raise BrowserSmokeError("Oversize BiDi event")
            if second & 0x80:
                raise BrowserSmokeError("Unexpected masked server frame")
            if len(self.buffer) < offset + size:
                break
            payload = bytes(self.buffer[offset:offset + size])
            del self.buffer[:offset + size]
            processed += 1
            opcode = first & 0x0F
            if opcode == 9:
                self.send(payload, 10)
                continue
            if opcode == 8:
                return
            if opcode != 1 or not first & 0x80:
                raise BrowserSmokeError("Unsupported fragmented BiDi event")
            message = json.loads(payload)
            if message.get("id") == 1:
                if message.get("type") == "error":
                    raise BrowserSmokeError("BiDi observation subscription rejected")
                self.subscribed = True
            elif message.get("method") == "log.entryAdded":
                entry = message["params"]
                self.entries.append({"level": entry.get("level"), "type": entry.get("type"),
                                     "text": str(entry.get("text", ""))[:2048]})
            elif message.get("method") == "network.beforeRequestSent":
                # URLs only; never collect request/response headers, cookies, or bodies.
                self.requests.append(message["params"]["request"]["url"])

    def close(self):
        self.connection.close()


def preflight():
    connection = http.client.HTTPConnection("127.0.0.1", 8002, timeout=4)
    try:
        connection.request("GET", "/")
        response = connection.getresponse()
        body = response.read(128 * 1024 + 1)
        if response.status != 200 or len(body) > 128 * 1024:
            raise BrowserSmokeError(f"Local Hosting preflight failed: HTTP {response.status}")
        parser = Metas()
        parser.feed(body.decode("utf-8"))
        if parser.values.get("weaver-lobby-bootstrap") not in ("", "off"):
            raise BrowserSmokeError("Anonymous bootstrap must be disabled for this offline smoke")
        if parser.values.get("weaver-lobby-target") != "":
            raise BrowserSmokeError("Expected empty lobby target meta for this offline smoke")
        return {"url": URL, "httpStatus": response.status, "bootstrapDisabled": True,
                "targetEmpty": True}
    finally:
        connection.close()


def main():
    if len(sys.argv) != 1:
        raise SystemExit("This smoke accepts no target arguments; only localhost:8002 is permitted")
    if not OUTPUT.is_dir():
        raise SystemExit("Create target/local-browser-qa before running this smoke")
    started = time.monotonic()
    deadline = started + TOTAL_SECONDS
    report = {"passed": False, "url": URL, "errors": [], "multiplayerTested": False}
    driver = None
    session_id = None
    events = None
    proxy = None
    profile_root = None
    driver_port = free_port()
    log = None
    try:
        report["preflight"] = preflight()
        proxy = http.server.ThreadingHTTPServer(("127.0.0.1", 0), DenyProxy)
        proxy.daemon_threads = True
        threading.Thread(target=proxy.serve_forever, daemon=True).start()
        profile_root = tempfile.TemporaryDirectory(prefix="firefox-profile-root-", dir=OUTPUT)
        log = (OUTPUT / "geckodriver.log").open("w")
        driver = subprocess.Popen([
            "/snap/bin/geckodriver", "--host", "127.0.0.1", "--port", str(driver_port),
            "--websocket-port", str(free_port()), "--profile-root", profile_root.name,
            # The snap driver cannot see the host's /usr/bin/firefox launcher.
            "--binary", "/snap/firefox/current/usr/lib/firefox/firefox", "--log", "info",
        ], stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
        wait_until = min(deadline, time.monotonic() + 8)
        while time.monotonic() < wait_until:
            if driver.poll() is not None:
                raise BrowserSmokeError(f"Owned geckodriver exited with code {driver.returncode}; see geckodriver.log")
            try:
                if http_json(driver_port, "GET", "/status", timeout=0.5).get("ready"):
                    break
            except EXPECTED_BROWSER_ERRORS:
                time.sleep(0.1)
        else:
            raise BrowserSmokeError("Owned geckodriver did not become ready within eight seconds")
        prefs = {
            "network.proxy.type": 1,
            "network.proxy.http": "127.0.0.1", "network.proxy.http_port": proxy.server_port,
            "network.proxy.ssl": "127.0.0.1", "network.proxy.ssl_port": proxy.server_port,
            "network.proxy.no_proxies_on": "127.0.0.1,localhost",
            "network.dns.disablePrefetch": True, "network.prefetch-next": False,
            "network.predictor.enabled": False, "network.trr.mode": 5,
            "network.http.speculative-parallel-limit": 0,
            "network.captive-portal-service.enabled": False,
            "network.connectivity-service.enabled": False,
            "toolkit.telemetry.enabled": False, "toolkit.telemetry.server": "",
            "datareporting.healthreport.uploadEnabled": False,
            "datareporting.policy.dataSubmissionEnabled": False,
            "app.update.auto": False, "app.update.enabled": False,
            "extensions.update.enabled": False, "browser.search.update": False,
            "browser.safebrowsing.malware.enabled": False,
            "browser.safebrowsing.phishing.enabled": False,
            "browser.safebrowsing.downloads.enabled": False,
            "browser.newtabpage.activity-stream.feeds.telemetry": False,
            "browser.newtabpage.activity-stream.feeds.snippets": False,
            "browser.newtabpage.activity-stream.feeds.system.topstories": False,
            "browser.newtabpage.activity-stream.feeds.section.topstories": False,
            "dom.push.enabled": False,
        }
        session = http_json(driver_port, "POST", "/session", {"capabilities": {"alwaysMatch": {
            "browserName": "firefox", "pageLoadStrategy": "eager", "webSocketUrl": True,
            "moz:firefoxOptions": {"args": ["-headless", "--width=1280", "--height=800"], "prefs": prefs},
        }}}, timeout=22, deadline=deadline)
        session_id = session["sessionId"]
        capabilities = session["capabilities"]
        report["browser"] = {"name": capabilities.get("browserName"), "version": capabilities.get("browserVersion"),
                             "headless": capabilities.get("moz:headless"), "freshProfile": True}
        events = BrowserEvents(capabilities["webSocketUrl"])
        base = f"/session/{session_id}"
        http_json(driver_port, "POST", base + "/timeouts", {"pageLoad": 12_000, "script": 5_000}, deadline=deadline)
        http_json(driver_port, "POST", base + "/url", {"url": URL}, timeout=14, deadline=deadline)
        until = min(deadline - 6, time.monotonic() + 25)
        snapshot = None
        while time.monotonic() < until:
            events.drain()
            snapshot = http_json(driver_port, "POST", base + "/execute/sync", {"script": SNAPSHOT, "args": []}, deadline=deadline)
            status = snapshot["status"]
            if status.startswith("Weaver ready on ") or "Scene unavailable" in status:
                break
            time.sleep(0.2)
        if snapshot is None:
            raise BrowserSmokeError("No browser startup snapshot within the bounded smoke")
        # Permit initial rendering and capture any immediate real render errors.
        time.sleep(0.3)
        events.drain()
        snapshot = http_json(driver_port, "POST", base + "/execute/sync", {"script": SNAPSHOT, "args": []}, deadline=deadline)
        report["snapshot"] = snapshot
        try:
            screenshot = http_json(driver_port, "GET", base + "/screenshot", timeout=5, deadline=deadline)
            (OUTPUT / "startup.png").write_bytes(base64.b64decode(screenshot, validate=True))
            report["screenshot"] = "target/local-browser-qa/startup.png"
        except EXPECTED_BROWSER_ERRORS + (binascii.Error,) as error:
            report["screenshotError"] = str(error)
        events.drain()
        report["console"] = events.entries
        report["pageRequests"] = events.requests
        checks = {
            "actualRendererReady": snapshot["status"].startswith("Weaver ready on "),
            "sceneCallbacksPresent": snapshot["sceneCallbacksDefined"],
            "jsLoaded": any(urlsplit(item["url"]).path.endswith("/main.js") for item in snapshot["resources"]),
            "wasmLoaded": any(urlsplit(item["url"]).path.endswith(".wasm") for item in snapshot["resources"]),
            "noHttpAssetFailures": not any((item["responseStatus"] or 0) >= 400 for item in snapshot["resources"]),
            "offlineOrExplicitlyUnsupported": snapshot["networkStatus"].startswith("Offline") or
                (not snapshot["webTransportAvailable"] and "Multiplayer unavailable" in snapshot["networkStatus"]),
            "networkStatusVisibleOutsideAdvancedForm": snapshot["networkVisible"],
            "advancedFormClosed": snapshot["settingsOpen"] is False,
            "disconnectDisabled": snapshot["disconnectDisabled"] is True,
            "joinControlsCorrect": snapshot["joinDisabled"] == snapshot["operatorConnectDisabled"] ==
                (not (snapshot["webTransportAvailable"] and snapshot["secureContext"])),
            "reloadHiddenAfterReadiness": snapshot["reloadHidden"] is True,
            "canvasAllocated": bool(snapshot["canvas"] and snapshot["canvas"]["width"] and snapshot["canvas"]["height"]),
            "noPageExternalRequests": not any(urlsplit(url).scheme in ("http", "https") and
                (urlsplit(url).hostname != "127.0.0.1" or urlsplit(url).port != 8002) for url in events.requests),
            "noConsoleErrors": not any(entry["level"] == "error" for entry in events.entries),
        }
        report["checks"] = checks
        report["errors"].extend(name for name, passed in checks.items() if not passed)
        report["passed"] = all(checks.values())
    except EXPECTED_BROWSER_ERRORS as error:
        report["errors"].append(str(error))
    finally:
        try:
            if events is not None:
                events.close()
            if session_id is not None:
                try:
                    http_json(driver_port, "DELETE", f"/session/{session_id}", timeout=2)
                except EXPECTED_BROWSER_ERRORS as error:
                    report["cleanupWarning"] = str(error)
        finally:
            # Unexpected cleanup bugs still propagate, but cannot skip owned-process cleanup.
            if driver is not None:
                # Only the new process group launched above; never pkill/killall or attach-existing.
                try:
                    os.killpg(driver.pid, signal.SIGTERM)
                except ProcessLookupError:
                    pass
                try:
                    driver.wait(timeout=2)
                except subprocess.TimeoutExpired:
                    os.killpg(driver.pid, signal.SIGKILL)
                    driver.wait(timeout=1)
            if log is not None:
                log.close()
            if proxy is not None:
                proxy.shutdown()
                proxy.server_close()
            report["blockedBrowserBackgroundRequests"] = DenyProxy.attempts
            if profile_root is not None:
                try:
                    profile_root.cleanup()
                except OSError as error:
                    report["profileCleanupWarning"] = str(error)
            report["elapsedSeconds"] = round(time.monotonic() - started, 2)
            (OUTPUT / "report.json").write_text(json.dumps(report, indent=2) + "\n")
            print(json.dumps(report, indent=2), flush=True)
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
