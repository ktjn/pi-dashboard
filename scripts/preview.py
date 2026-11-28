import argparse
import os
import shutil
import subprocess
import webbrowser
from collections.abc import Mapping, Sequence
from functools import partial
from http.server import SimpleHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_PREVIEW_DIR = REPO_ROOT / ".preview"


class PreviewRequestHandler(SimpleHTTPRequestHandler):
    extensions_map = {
        **SimpleHTTPRequestHandler.extensions_map,
        ".wasm": "application/wasm",
        ".js": "application/javascript",
    }

    def end_headers(self) -> None:
        self.send_header("Cache-Control", "no-store")
        super().end_headers()


def parse_args(argv: Sequence[str] | None = None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Build and preview the pi-dashboard 3D prototype locally.",
    )
    parser.add_argument(
        "--mode",
        choices=("native", "web"),
        default="native",
        help="Preview mode: windowed native dashboard-pi (default) or wasm32 web playground.",
    )
    profile_group = parser.add_mutually_exclusive_group()
    profile_group.add_argument(
        "--release",
        dest="release",
        action="store_true",
        default=True,
        help="Build with Cargo release profile (default).",
    )
    profile_group.add_argument(
        "--debug",
        dest="release",
        action="store_false",
        help="Build with Cargo debug profile.",
    )
    parser.add_argument(
        "--host",
        default="127.0.0.1",
        help="HTTP bind host for web preview mode.",
    )
    parser.add_argument(
        "--port",
        type=int,
        default=8080,
        help="HTTP bind port for web preview mode.",
    )
    parser.add_argument(
        "--no-open",
        action="store_true",
        help="Do not automatically open the browser in web preview mode.",
    )
    parser.add_argument(
        "--build-only",
        action="store_true",
        help="Build and stage preview artifacts without starting the preview.",
    )
    return parser.parse_args(argv)


def cargo_build_command(mode: str, release: bool) -> list[str]:
    if mode == "web":
        command = [
            "cargo",
            "build",
            "--target",
            "wasm32-unknown-unknown",
            "-p",
            "dashboard-web",
        ]
    elif mode == "native":
        command = ["cargo", "build", "-p", "dashboard-pi", "--bin", "dashboard-pi"]
    else:
        raise ValueError(f"unsupported preview mode: {mode}")

    if release:
        command.append("--release")
    return command


def native_run_command(release: bool) -> list[str]:
    command = ["cargo", "run", "-p", "dashboard-pi", "--bin", "dashboard-pi"]
    if release:
        command.append("--release")
    return command


def browser_url(host: str, port: int) -> str:
    if host in ("0.0.0.0", ""):
        url_host = "127.0.0.1"
    elif host == "::":
        url_host = "[::1]"
    elif ":" in host and not (host.startswith("[") and host.endswith("]")):
        url_host = f"[{host}]"
    else:
        url_host = host
    return f"http://{url_host}:{port}/"


def native_run_env(base_env: Mapping[str, str]) -> dict[str, str]:
    env = dict(base_env)
    env.setdefault("PI_DASHBOARD_WINDOWED", "1")
    env.setdefault("DISPLAY_POWER_BACKEND", "null")
    return env


def stage_web_bundle(repo_root: Path, release: bool, preview_dir: Path) -> Path:
    profile = "release" if release else "debug"
    web_dir = repo_root / "dashboard-web" / "web"
    sources = (
        web_dir / "index.html",
        web_dir / "mq_js_bundle.js",
        repo_root
        / "target"
        / "wasm32-unknown-unknown"
        / profile
        / "dashboard-web.wasm",
    )

    for source in sources:
        if not source.is_file():
            raise FileNotFoundError(f"missing required preview artifact: {source}")

    preview_dir.mkdir(parents=True, exist_ok=True)
    for source in sources:
        shutil.copy2(source, preview_dir / source.name)

    return preview_dir


def create_preview_server(
    preview_dir: Path,
    host: str,
    port: int,
) -> ThreadingHTTPServer:
    handler = partial(PreviewRequestHandler, directory=str(preview_dir))
    return ThreadingHTTPServer((host, port), handler)


def run_web_preview(
    args: argparse.Namespace,
    repo_root: Path = REPO_ROOT,
    preview_dir: Path = DEFAULT_PREVIEW_DIR,
) -> int:
    subprocess.run(
        cargo_build_command("web", args.release),
        cwd=repo_root,
        check=True,
    )
    staged_dir = stage_web_bundle(repo_root, args.release, preview_dir)
    if args.build_only:
        print(f"Staged web preview bundle in {staged_dir}", flush=True)
        return 0

    with create_preview_server(staged_dir, args.host, args.port) as server:
        bound_host = str(server.server_address[0])
        bound_port = int(server.server_address[1])
        url = browser_url(bound_host, bound_port)
        print(f"Serving pi-dashboard web preview at {url}", flush=True)
        if not args.no_open:
            webbrowser.open(url)
        try:
            server.serve_forever()
        except KeyboardInterrupt:
            pass
    return 0


def run_native_preview(
    args: argparse.Namespace,
    repo_root: Path = REPO_ROOT,
) -> int:
    if args.build_only:
        subprocess.run(
            cargo_build_command("native", args.release),
            cwd=repo_root,
            check=True,
        )
        return 0

    subprocess.run(
        native_run_command(args.release),
        cwd=repo_root,
        env=native_run_env(os.environ),
        check=True,
    )
    return 0


def main(argv: Sequence[str] | None = None) -> int:
    args = parse_args(argv)
    try:
        if args.mode == "web":
            return run_web_preview(args)
        return run_native_preview(args)
    except KeyboardInterrupt:
        return 0
    except subprocess.CalledProcessError as exc:
        return exc.returncode


if __name__ == "__main__":
    raise SystemExit(main())
