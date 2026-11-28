import contextlib
import http.client
import io
import subprocess
import sys
import tempfile
import threading
import unittest
from pathlib import Path
from unittest.mock import MagicMock, patch
from urllib.request import urlopen

SCRIPTS_DIR = Path(__file__).resolve().parent
if str(SCRIPTS_DIR) not in sys.path:
    sys.path.insert(0, str(SCRIPTS_DIR))

import preview


class TestParseArgs(unittest.TestCase):
    def test_defaults(self) -> None:
        args = preview.parse_args([])
        self.assertEqual(args.mode, "native")
        self.assertTrue(args.release)
        self.assertEqual(args.host, "127.0.0.1")
        self.assertEqual(args.port, 8080)
        self.assertFalse(args.no_open)
        self.assertFalse(args.build_only)

    def test_all_flags(self) -> None:
        args = preview.parse_args(
            [
                "--mode",
                "web",
                "--debug",
                "--host",
                "0.0.0.0",
                "--port",
                "9090",
                "--no-open",
                "--build-only",
            ]
        )
        self.assertEqual(args.mode, "web")
        self.assertFalse(args.release)
        self.assertEqual(args.host, "0.0.0.0")
        self.assertEqual(args.port, 9090)
        self.assertTrue(args.no_open)
        self.assertTrue(args.build_only)

    def test_explicit_native_release_flags(self) -> None:
        args = preview.parse_args(["--mode", "native", "--release"])
        self.assertEqual(args.mode, "native")
        self.assertTrue(args.release)

    def test_release_and_debug_are_mutually_exclusive(self) -> None:
        stderr = io.StringIO()
        with contextlib.redirect_stderr(stderr):
            with self.assertRaises(SystemExit) as ctx:
                preview.parse_args(["--release", "--debug"])
        self.assertEqual(ctx.exception.code, 2)
        self.assertIn("not allowed with argument", stderr.getvalue())


class TestCargoCommands(unittest.TestCase):
    def test_web_debug_command(self) -> None:
        self.assertEqual(
            preview.cargo_build_command("web", False),
            [
                "cargo",
                "build",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "dashboard-web",
            ],
        )

    def test_web_release_command(self) -> None:
        self.assertEqual(
            preview.cargo_build_command("web", True),
            [
                "cargo",
                "build",
                "--target",
                "wasm32-unknown-unknown",
                "-p",
                "dashboard-web",
                "--release",
            ],
        )

    def test_native_debug_command(self) -> None:
        self.assertEqual(
            preview.cargo_build_command("native", False),
            ["cargo", "build", "-p", "dashboard-pi", "--bin", "dashboard-pi"],
        )

    def test_native_release_command(self) -> None:
        self.assertEqual(
            preview.cargo_build_command("native", True),
            [
                "cargo",
                "build",
                "-p",
                "dashboard-pi",
                "--bin",
                "dashboard-pi",
                "--release",
            ],
        )

    def test_native_run_command(self) -> None:
        self.assertEqual(
            preview.native_run_command(False),
            ["cargo", "run", "-p", "dashboard-pi", "--bin", "dashboard-pi"],
        )
        self.assertEqual(
            preview.native_run_command(True),
            [
                "cargo",
                "run",
                "-p",
                "dashboard-pi",
                "--bin",
                "dashboard-pi",
                "--release",
            ],
        )

    def test_invalid_mode_raises_value_error(self) -> None:
        with self.assertRaises(ValueError):
            preview.cargo_build_command("unknown", False)


class TestBrowserUrl(unittest.TestCase):
    def test_normalizes_wildcard_and_ipv6_hosts(self) -> None:
        self.assertEqual(
            preview.browser_url("127.0.0.1", 8080),
            "http://127.0.0.1:8080/",
        )
        self.assertEqual(
            preview.browser_url("0.0.0.0", 8080),
            "http://127.0.0.1:8080/",
        )
        self.assertEqual(
            preview.browser_url("", 8080),
            "http://127.0.0.1:8080/",
        )
        self.assertEqual(
            preview.browser_url("::", 8080),
            "http://[::1]:8080/",
        )
        self.assertEqual(
            preview.browser_url("::1", 8080),
            "http://[::1]:8080/",
        )


class TestNativeRunEnv(unittest.TestCase):
    def test_sets_defaults_without_mutating_input(self) -> None:
        base = {"PATH": "/usr/bin"}
        result = preview.native_run_env(base)
        self.assertEqual(base, {"PATH": "/usr/bin"})
        self.assertEqual(result["PATH"], "/usr/bin")
        self.assertEqual(result["PI_DASHBOARD_WINDOWED"], "1")
        self.assertEqual(result["DISPLAY_POWER_BACKEND"], "null")

    def test_preserves_existing_overrides(self) -> None:
        base = {
            "PI_DASHBOARD_WINDOWED": "0",
            "DISPLAY_POWER_BACKEND": "sysfs",
        }
        result = preview.native_run_env(base)
        self.assertEqual(result["PI_DASHBOARD_WINDOWED"], "0")
        self.assertEqual(result["DISPLAY_POWER_BACKEND"], "sysfs")


class TestStageWebBundle(unittest.TestCase):
    def _populate_repo(
        self, repo_root: Path, profile: str, include_wasm: bool = True
    ) -> None:
        web_dir = repo_root / "dashboard-web" / "web"
        web_dir.mkdir(parents=True, exist_ok=True)
        (web_dir / "index.html").write_bytes(b"<html>preview</html>")
        (web_dir / "mq_js_bundle.js").write_bytes(b"console.log('mq');")

        if include_wasm:
            wasm_dir = repo_root / "target" / "wasm32-unknown-unknown" / profile
            wasm_dir.mkdir(parents=True, exist_ok=True)
            (wasm_dir / "dashboard-web.wasm").write_bytes(b"\x00asm\x01\x00\x00\x00")

    def test_stages_debug_bundle_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo_root = Path(tmp) / "repo"
            preview_dir = Path(tmp) / "preview"
            self._populate_repo(repo_root, "debug")

            staged = preview.stage_web_bundle(repo_root, False, preview_dir)

            self.assertEqual(staged, preview_dir)
            self.assertEqual(
                (preview_dir / "index.html").read_bytes(),
                b"<html>preview</html>",
            )
            self.assertEqual(
                (preview_dir / "mq_js_bundle.js").read_bytes(),
                b"console.log('mq');",
            )
            self.assertEqual(
                (preview_dir / "dashboard-web.wasm").read_bytes(),
                b"\x00asm\x01\x00\x00\x00",
            )

    def test_stages_release_bundle_files(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo_root = Path(tmp) / "repo"
            preview_dir = Path(tmp) / "preview"
            self._populate_repo(repo_root, "release")

            staged = preview.stage_web_bundle(repo_root, True, preview_dir)

            self.assertEqual(staged, preview_dir)
            self.assertEqual(
                (preview_dir / "dashboard-web.wasm").read_bytes(),
                b"\x00asm\x01\x00\x00\x00",
            )

    def test_raises_on_missing_wasm_artifact(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            repo_root = Path(tmp) / "repo"
            preview_dir = Path(tmp) / "preview"
            self._populate_repo(repo_root, "debug", include_wasm=False)

            with self.assertRaises(FileNotFoundError) as ctx:
                preview.stage_web_bundle(repo_root, False, preview_dir)

            self.assertIn("dashboard-web.wasm", str(ctx.exception))


class TestRunWebPreview(unittest.TestCase):
    def test_build_only_stages_without_starting_server(self) -> None:
        args = preview.parse_args(["--build-only", "--release"])
        repo_root = Path("/repo")
        preview_dir = Path("/repo/.preview")

        with (
            patch.object(preview.subprocess, "run") as mock_run,
            patch.object(
                preview, "stage_web_bundle", return_value=preview_dir
            ) as mock_stage,
            patch.object(preview, "create_preview_server") as mock_server,
            patch.object(preview.webbrowser, "open") as mock_open,
        ):
            rc = preview.run_web_preview(
                args, repo_root=repo_root, preview_dir=preview_dir
            )

        self.assertEqual(rc, 0)
        mock_run.assert_called_once_with(
            preview.cargo_build_command("web", True),
            cwd=repo_root,
            check=True,
        )
        mock_stage.assert_called_once_with(repo_root, True, preview_dir)
        mock_server.assert_not_called()
        mock_open.assert_not_called()

    def test_opens_normalized_browser_url_and_closes_server_on_interrupt(self) -> None:
        args = preview.parse_args(["--host", "0.0.0.0", "--port", "8080"])
        repo_root = Path("/repo")
        preview_dir = Path("/repo/.preview")

        mock_server = MagicMock()
        mock_server.__enter__.return_value = mock_server
        mock_server.server_address = ("0.0.0.0", 8080)
        mock_server.serve_forever.side_effect = KeyboardInterrupt

        with (
            patch.object(preview.subprocess, "run"),
            patch.object(preview, "stage_web_bundle", return_value=preview_dir),
            patch.object(
                preview, "create_preview_server", return_value=mock_server
            ) as mock_create,
            patch.object(preview.webbrowser, "open") as mock_open,
        ):
            rc = preview.run_web_preview(
                args, repo_root=repo_root, preview_dir=preview_dir
            )

        self.assertEqual(rc, 0)
        mock_create.assert_called_once_with(preview_dir, "0.0.0.0", 8080)
        mock_open.assert_called_once_with("http://127.0.0.1:8080/")
        mock_server.__enter__.assert_called_once()
        mock_server.__exit__.assert_called_once()

    def test_no_open_skips_browser_launch(self) -> None:
        args = preview.parse_args(["--no-open"])
        repo_root = Path("/repo")
        preview_dir = Path("/repo/.preview")

        mock_server = MagicMock()
        mock_server.__enter__.return_value = mock_server
        mock_server.server_address = ("127.0.0.1", 8080)
        mock_server.serve_forever.side_effect = KeyboardInterrupt

        with (
            patch.object(preview.subprocess, "run"),
            patch.object(preview, "stage_web_bundle", return_value=preview_dir),
            patch.object(preview, "create_preview_server", return_value=mock_server),
            patch.object(preview.webbrowser, "open") as mock_open,
        ):
            rc = preview.run_web_preview(
                args, repo_root=repo_root, preview_dir=preview_dir
            )

        self.assertEqual(rc, 0)
        mock_open.assert_not_called()
        mock_server.__exit__.assert_called_once()

    def test_closes_server_when_browser_open_raises(self) -> None:
        args = preview.parse_args([])
        repo_root = Path("/repo")
        preview_dir = Path("/repo/.preview")

        mock_server = MagicMock()
        mock_server.__enter__.return_value = mock_server
        mock_server.server_address = ("127.0.0.1", 8080)

        with (
            patch.object(preview.subprocess, "run"),
            patch.object(preview, "stage_web_bundle", return_value=preview_dir),
            patch.object(preview, "create_preview_server", return_value=mock_server),
            patch.object(
                preview.webbrowser,
                "open",
                side_effect=RuntimeError("browser unavailable"),
            ),
        ):
            with self.assertRaises(RuntimeError):
                preview.run_web_preview(
                    args, repo_root=repo_root, preview_dir=preview_dir
                )

        mock_server.__enter__.assert_called_once()
        mock_server.__exit__.assert_called_once()


class TestRunNativePreview(unittest.TestCase):
    def test_build_only_runs_cargo_build(self) -> None:
        args = preview.parse_args(["--mode", "native", "--build-only"])
        repo_root = Path("/repo")

        with patch.object(preview.subprocess, "run") as mock_run:
            rc = preview.run_native_preview(args, repo_root=repo_root)

        self.assertEqual(rc, 0)
        mock_run.assert_called_once_with(
            [
                "cargo",
                "build",
                "-p",
                "dashboard-pi",
                "--bin",
                "dashboard-pi",
                "--release",
            ],
            cwd=repo_root,
            check=True,
        )

    def test_build_only_debug_runs_cargo_build_without_release(self) -> None:
        args = preview.parse_args(["--mode", "native", "--debug", "--build-only"])
        repo_root = Path("/repo")

        with patch.object(preview.subprocess, "run") as mock_run:
            rc = preview.run_native_preview(args, repo_root=repo_root)

        self.assertEqual(rc, 0)
        mock_run.assert_called_once_with(
            ["cargo", "build", "-p", "dashboard-pi", "--bin", "dashboard-pi"],
            cwd=repo_root,
            check=True,
        )

    def test_run_mode_runs_cargo_run_with_native_env(self) -> None:
        args = preview.parse_args(["--mode", "native", "--release"])
        repo_root = Path("/repo")

        with (
            patch.dict(preview.os.environ, {"EXISTING": "yes"}, clear=True),
            patch.object(preview.subprocess, "run") as mock_run,
        ):
            rc = preview.run_native_preview(args, repo_root=repo_root)

        self.assertEqual(rc, 0)
        mock_run.assert_called_once_with(
            [
                "cargo",
                "run",
                "-p",
                "dashboard-pi",
                "--bin",
                "dashboard-pi",
                "--release",
            ],
            cwd=repo_root,
            env={
                "EXISTING": "yes",
                "PI_DASHBOARD_WINDOWED": "1",
                "DISPLAY_POWER_BACKEND": "null",
            },
            check=True,
        )


class TestMain(unittest.TestCase):
    def test_defaults_to_native_release_preview(self) -> None:
        with (
            patch.object(preview, "run_native_preview", return_value=0) as mock_native,
            patch.object(preview, "run_web_preview", return_value=0) as mock_web,
        ):
            self.assertEqual(preview.main([]), 0)

        mock_web.assert_not_called()
        mock_native.assert_called_once()
        parsed_args = mock_native.call_args.args[0]
        self.assertEqual(parsed_args.mode, "native")
        self.assertTrue(parsed_args.release)

    def test_propagates_called_process_error_returncode(self) -> None:
        err = subprocess.CalledProcessError(returncode=101, cmd=["cargo"])
        with patch.object(preview, "run_web_preview", side_effect=err):
            self.assertEqual(preview.main(["--mode", "web"]), 101)

    def test_returns_zero_on_keyboard_interrupt(self) -> None:
        with patch.object(preview, "run_native_preview", side_effect=KeyboardInterrupt):
            self.assertEqual(preview.main(["--mode", "native"]), 0)


class TestPreviewHttpServer(unittest.TestCase):
    def test_serves_html_wasm_and_js_with_expected_headers(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            preview_dir = Path(tmp)
            (preview_dir / "index.html").write_bytes(b"<html>ok</html>")
            (preview_dir / "mq_js_bundle.js").write_bytes(b"function load() {}")
            (preview_dir / "dashboard-web.wasm").write_bytes(b"\x00asm\x01\x00\x00\x00")

            server = preview.create_preview_server(preview_dir, "127.0.0.1", 0)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                host, port = server.server_address[:2]
                base_url = f"http://{host}:{port}"

                with urlopen(f"{base_url}/index.html", timeout=5) as resp:
                    self.assertEqual(resp.status, 200)
                    self.assertEqual(resp.headers.get("Cache-Control"), "no-store")
                    self.assertEqual(resp.read(), b"<html>ok</html>")

                with urlopen(f"{base_url}/dashboard-web.wasm", timeout=5) as resp:
                    self.assertEqual(resp.status, 200)
                    self.assertEqual(resp.headers.get_content_type(), "application/wasm")
                    self.assertEqual(resp.headers.get("Cache-Control"), "no-store")
                    self.assertEqual(resp.read(), b"\x00asm\x01\x00\x00\x00")

                with urlopen(f"{base_url}/mq_js_bundle.js", timeout=5) as resp:
                    self.assertEqual(resp.status, 200)
                    self.assertEqual(
                        resp.headers.get_content_type(), "application/javascript"
                    )
                    self.assertEqual(resp.headers.get("Cache-Control"), "no-store")
                    self.assertEqual(resp.read(), b"function load() {}")
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)

    def test_path_traversal_returns_404_and_stays_within_preview_dir(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "Cargo.toml").write_bytes(b"[workspace]\nsecret = true\n")
            preview_dir = root / ".preview"
            preview_dir.mkdir()
            (preview_dir / "index.html").write_bytes(b"<html>ok</html>")

            server = preview.create_preview_server(preview_dir, "127.0.0.1", 0)
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                host, port = server.server_address[:2]
                for target in (
                    "/../Cargo.toml",
                    "/../../Cargo.toml",
                    "/%2e%2e/Cargo.toml",
                ):
                    conn = http.client.HTTPConnection(host, port, timeout=5)
                    try:
                        conn.request("GET", target)
                        resp = conn.getresponse()
                        body = resp.read()
                        self.assertEqual(resp.status, 404, target)
                        self.assertNotIn(b"secret = true", body)
                    finally:
                        conn.close()
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)


if __name__ == "__main__":
    unittest.main()
