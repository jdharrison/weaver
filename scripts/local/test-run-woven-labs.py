"""Launcher selection regressions using only temporary fake build/client commands."""

import os
from pathlib import Path
import subprocess
import tempfile
import unittest


LAUNCHER = Path(__file__).with_name("run-woven-labs.sh").resolve()


class TargetSelectionTests(unittest.TestCase):
    def run_launcher(self, args=(), **overrides):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "target/debug").mkdir(parents=True)
            cargo = root / "cargo"
            cargo.write_text('#!/bin/sh\nprintf "build\\n" >> calls\n')
            cargo.chmod(0o700)
            binary = root / "target/debug/woven-lab"
            binary.write_text(
                '#!/bin/sh\nprintf "%s %s %s %s\\n" "$WOVEN_LAB_TARGET" '
                '"$WOVEN_LAB_CLIENT" "$WOVEN_LAB_RATE_HZ" "$WOVEN_LAB_URL" >> calls\n'
            )
            binary.chmod(0o700)
            env = {key: value for key, value in os.environ.items()
                   if not key.startswith("WOVEN_LAB_")}
            env.update(PATH=f"{root}:{os.defpath}", WOVEN_LAB_DELAY_SECONDS="0")
            # Dummy files for fake-client preflight only; never used as TLS/auth material.
            for variable in ["WOVEN_LAB_CA_PEM_FILE", "WOVEN_LAB_TOKEN_FILE"]:
                if overrides.get(variable) == "fixture":
                    path = root / variable
                    path.write_text("not a certificate or credential")
                    overrides[variable] = str(path)
            env.update(overrides)
            result = subprocess.run(
                ["sh", str(LAUNCHER), *args], cwd=root, env=env,
                capture_output=True, text=True, timeout=10,
            )
            calls = root / "calls"
            return result, calls.read_text().splitlines() if calls.exists() else []

    def assert_rejected(self, args=(), message="", **env):
        result, calls = self.run_launcher(args, **env)
        self.assertEqual(result.returncode, 2, result.stderr)
        self.assertIn(message, result.stderr)
        self.assertEqual(calls, [], "rejected targets must not build or launch")

    def test_legacy_count_and_rate(self):
        result, calls = self.run_launcher(["2", "30"])
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls[0], "build")
        self.assertCountEqual(calls[1:], [
            "local lab-1 30 quic://127.0.0.1:8081",
            "local lab-2 30 quic://127.0.0.1:8081",
        ])

    def test_default_local_and_count(self):
        result, calls = self.run_launcher()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(len(calls), 5)
        self.assertTrue(all(line.startswith("local lab-") for line in calls[1:]))

    def test_explicit_local_overrides_cloud_environment(self):
        result, calls = self.run_launcher(
            ["local", "1", "20"], WOVEN_LAB_TARGET="cloud",
            WOVEN_LAB_URL="quic://127.0.0.1:9001",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["build", "local lab-1 20 quic://127.0.0.1:9001"])

    def test_environment_local_with_count_fallback(self):
        result, calls = self.run_launcher(WOVEN_LAB_TARGET="local", WOVEN_LAB_COUNT="1")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["build", "local lab-1 10 quic://127.0.0.1:8081"])

    def test_remote_missing_address_never_uses_local_url(self):
        for target in ["remote", "cloud"]:
            self.assert_rejected([target], message="requires WOVEN_LAB_REMOTE_URL",
                                 WOVEN_LAB_URL="quic://127.0.0.1:8081")
        self.assert_rejected(["1", "10"], message="requires WOVEN_LAB_REMOTE_URL",
                             WOVEN_LAB_TARGET="cloud")

    def test_remote_requires_duration_and_files(self):
        for target in ["remote", "cloud"]:
            self.assert_rejected([target], message="require WOVEN_LAB_DURATION_SECONDS",
                                 WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081")
            self.assert_rejected([target], message="readable WOVEN_LAB_CA_PEM_FILE",
                                 WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
                                 WOVEN_LAB_DURATION_SECONDS="30")

    def test_remote_preflight_caps_and_url(self):
        valid = dict(WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
                     WOVEN_LAB_DURATION_SECONDS="30", WOVEN_LAB_CA_PEM_FILE="fixture",
                     WOVEN_LAB_TOKEN_FILE="fixture")
        for duration in ["", "0", "301", "1.5", "NaN", "-1", "999999999999999999999"]:
            self.assert_rejected(["remote"], message="WOVEN_LAB_DURATION_SECONDS",
                                 **(valid | {"WOVEN_LAB_DURATION_SECONDS": duration}))
        for rate in ["0", "121", "NaN", "10,", "10,121"]:
            self.assert_rejected(["remote"], message="publish rate",
                                 **(valid | {"WOVEN_LAB_RATES": rate}))
        self.assert_rejected(["remote"], message="credentials",
                             **(valid | {"WOVEN_LAB_REMOTE_URL": "quic://secret@woven.example.test:8081"}))
        self.assert_rejected(["remote", "17"], **valid)

    def test_remote_and_cloud_launch_only_explicit_remote_url(self):
        for target, url_variable in [("remote", "WOVEN_LAB_REMOTE_URL"),
                                     ("cloud", "WOVEN_LAB_CLOUD_URL")]:
            result, calls = self.run_launcher(
                [target, "1", "120"], **{url_variable: "quic://woven.example.test:8081"},
                WOVEN_LAB_URL="quic://127.0.0.1:9001",
                WOVEN_LAB_DURATION_SECONDS="1", WOVEN_LAB_CA_PEM_FILE="fixture",
                WOVEN_LAB_TOKEN_FILE="fixture")
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(calls, ["build", f"{target} lab-1 120 quic://woven.example.test:8081"])
            self.assertNotIn("woven.example.test", result.stdout)

    def test_invalid_selection(self):
        for target in ["", "web", "LOCAL", " local "]:
            with self.subTest(target=target):
                self.assert_rejected(message="WOVEN_LAB_TARGET must be local, remote or cloud",
                                     WOVEN_LAB_TARGET=target)
        self.assert_rejected(["web"], message="target must be local, remote or cloud")

    def test_invalid_count_and_extra_arguments(self):
        for args in [["local", "0"], ["local", "17"], ["local", "bad"],
                     ["local", "1", "10", "extra"]]:
            with self.subTest(args=args):
                self.assert_rejected(args)

    def test_help_does_not_build(self):
        result, calls = self.run_launcher(["--help"], WOVEN_LAB_TARGET="cloud")
        self.assertEqual(result.returncode, 0)
        self.assertIn("[local|remote|cloud] [count] [rate_hz]", result.stdout)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
