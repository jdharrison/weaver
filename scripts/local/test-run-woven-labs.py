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
                'case "$WOVEN_LAB_TARGET" in '
                'managed-local) managed_url="$WOVEN_LAB_MANAGED_URL" ;; '
                'cloud) managed_url="$WOVEN_LAB_CLOUD_URL" ;; '
                '*) managed_url="" ;; esac; '
                'if [ -n "$managed_url" ]; then '
                'printf "managed-config %s %s %s\\n" "$managed_url" '
                '"$WOVEN_LAB_NAMESPACE_ID" "$WOVEN_LAB_SESSION_ID" >> calls; fi\n'
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
                    path.chmod(0o600 if variable == "WOVEN_LAB_TOKEN_FILE" else 0o644)
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

    def test_managed_local_requires_scope_and_files_then_uses_managed_endpoint(self):
        self.assert_rejected(
            ["managed-local"], message="require WOVEN_LAB_NAMESPACE_ID",
        )
        base = dict(
            WOVEN_LAB_NAMESPACE_ID="9007199254740993",
            WOVEN_LAB_SESSION_ID="2",
        )
        self.assert_rejected(
            ["managed-local"], message="readable WOVEN_LAB_CA_PEM_FILE", **base,
        )
        for invalid in ["0", "01", "-1", "1.5", "space id"]:
            self.assert_rejected(
                ["managed-local"], message="canonical nonzero decimal",
                **(base | {"WOVEN_LAB_SESSION_ID": invalid}),
            )
        result, calls = self.run_launcher(
            ["managed-local", "2", "10"], **base,
            WOVEN_LAB_CA_PEM_FILE="fixture", WOVEN_LAB_TOKEN_FILE="fixture",
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertCountEqual(calls[1:], [
            "managed-local lab-1 10 quic://127.0.0.1:18082",
            "managed-config quic://127.0.0.1:18082 9007199254740993 2",
            "managed-local lab-2 10 quic://127.0.0.1:18082",
            "managed-config quic://127.0.0.1:18082 9007199254740993 2",
        ])

    def test_remote_missing_address_never_uses_local_url(self):
        self.assert_rejected(["remote"], message="requires WOVEN_LAB_REMOTE_URL",
                             WOVEN_LAB_URL="quic://127.0.0.1:8081")

    def test_cloud_missing_address_never_uses_local_or_remote_url(self):
        self.assert_rejected(
            ["cloud"], message="requires WOVEN_LAB_CLOUD_URL",
            WOVEN_LAB_URL="quic://127.0.0.1:8081",
            WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
        )
        self.assert_rejected(
            ["1", "10"], message="requires WOVEN_LAB_CLOUD_URL",
            WOVEN_LAB_TARGET="cloud",
        )

    def test_remote_requires_duration_and_files(self):
        self.assert_rejected(["remote"], message="require WOVEN_LAB_DURATION_SECONDS",
                             WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081")
        self.assert_rejected(["remote"], message="readable WOVEN_LAB_CA_PEM_FILE",
                             WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
                             WOVEN_LAB_DURATION_SECONDS="30")

    def test_cloud_requires_duration_scope_and_files(self):
        base = dict(
            WOVEN_LAB_CLOUD_URL="quic://woven.example.test:8081",
            WOVEN_LAB_NAMESPACE_ID="11",
            WOVEN_LAB_SESSION_ID="17",
        )
        self.assert_rejected(["cloud"], message="require WOVEN_LAB_DURATION_SECONDS",
                             **base)
        self.assert_rejected(
            ["cloud"], message="readable WOVEN_LAB_CA_PEM_FILE",
            **(base | {"WOVEN_LAB_DURATION_SECONDS": "30"}),
        )

    def test_remote_preflight_caps_and_url(self):
        valid = dict(WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
                     WOVEN_LAB_DURATION_SECONDS="30", WOVEN_LAB_CA_PEM_FILE="fixture",
                     WOVEN_LAB_TOKEN_FILE="fixture")
        for duration in ["", "0", "601", "1.5", "NaN", "-1", "999999999999999999999"]:
            self.assert_rejected(["remote"], message="WOVEN_LAB_DURATION_SECONDS",
                                 **(valid | {"WOVEN_LAB_DURATION_SECONDS": duration}))
        for rate in ["0", "121", "NaN", "10,", "10,121"]:
            self.assert_rejected(["remote"], message="publish rate",
                                 **(valid | {"WOVEN_LAB_RATES": rate}))
        self.assert_rejected(["remote"], message="credentials",
                             **(valid | {"WOVEN_LAB_REMOTE_URL": "quic://secret@woven.example.test:8081"}))
        self.assert_rejected(["remote", "17"], **valid)

    def test_remote_launches_only_explicit_remote_url(self):
        result, calls = self.run_launcher(
            ["remote", "1", "120"],
            WOVEN_LAB_REMOTE_URL="quic://woven.example.test:8081",
            WOVEN_LAB_URL="quic://127.0.0.1:9001",
            WOVEN_LAB_DURATION_SECONDS="1", WOVEN_LAB_CA_PEM_FILE="fixture",
            WOVEN_LAB_TOKEN_FILE="fixture")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, ["build", "remote lab-1 120 quic://woven.example.test:8081"])
        self.assertNotIn("woven.example.test", result.stdout)

    def test_cloud_launches_managed_scope_only_with_explicit_cloud_url(self):
        result, calls = self.run_launcher(
            ["cloud", "1", "120"],
            WOVEN_LAB_CLOUD_URL="quic://woven.example.test:8081",
            WOVEN_LAB_REMOTE_URL="quic://wrong.example.test:8081",
            WOVEN_LAB_URL="quic://127.0.0.1:9001",
            WOVEN_LAB_NAMESPACE_ID="11", WOVEN_LAB_SESSION_ID="17",
            WOVEN_LAB_DURATION_SECONDS="600", WOVEN_LAB_CA_PEM_FILE="fixture",
            WOVEN_LAB_TOKEN_FILE="fixture")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(calls, [
            "build",
            "cloud lab-1 120 quic://woven.example.test:8081",
            "managed-config quic://woven.example.test:8081 11 17",
        ])
        self.assertNotIn("woven.example.test", result.stdout)

    def test_invalid_selection(self):
        for target in ["", "web", "LOCAL", " local "]:
            with self.subTest(target=target):
                self.assert_rejected(
                    message="WOVEN_LAB_TARGET must be local, managed-local, remote or cloud",
                    WOVEN_LAB_TARGET=target,
                )
        self.assert_rejected(
            ["web"], message="target must be local, managed-local, remote or cloud"
        )

    def test_invalid_count_and_extra_arguments(self):
        for args in [["local", "0"], ["local", "17"], ["local", "bad"],
                     ["local", "1", "10", "extra"]]:
            with self.subTest(args=args):
                self.assert_rejected(args)

    def test_help_does_not_build(self):
        result, calls = self.run_launcher(["--help"], WOVEN_LAB_TARGET="cloud")
        self.assertEqual(result.returncode, 0)
        self.assertIn("[local|managed-local|remote|cloud] [count] [rate_hz]", result.stdout)
        self.assertEqual(calls, [])


if __name__ == "__main__":
    unittest.main()
