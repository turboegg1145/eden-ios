#!/usr/bin/env python3
"""Build self-contained (retail data embedded) Eden IPAs.

The CI workflow produces a small shell IPA (``Eden-iOS-arm64.ipa``). This tool
injects the retail archives into ``Payload/Eden.app/gamedata`` and writes a
single IPA, or <= 1.9 GiB volumes when the package exceeds GitHub's 2 GiB
per-asset limit.

Examples
--------
    # main story, PM, and the combined package
    python3 tools/package_ipa.py build --mode main --shell artifacts/Eden-iOS-arm64.ipa
    python3 tools/package_ipa.py build --mode pm   --shell artifacts/Eden-iOS-arm64.ipa
    python3 tools/package_ipa.py build --mode all  --shell artifacts/Eden-iOS-arm64.ipa

    # raw data cache consumed by ios-ipa.yml (workflow_dispatch embed=...)
    python3 tools/package_ipa.py cache
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import shutil
import struct
import subprocess
import sys
import tarfile
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
DEFAULT_GAME = Path("/mnt/f/gal/【PC硬盘】【官中】eden")
DEFAULT_SHELL = REPO / "artifacts/Eden-iOS-arm64.ipa"
DEFAULT_OUT = REPO / "artifacts"
DEFAULT_FONT = REPO / "platforms/ios/EdenApp/font.ttf"

MAIN_ARCHIVES = [
    "scr.paz",
    "bg.paz",
    "sys.paz",
    "st.paz",
    "bgm.paz",
    "se.paz",
    "voice.paz",
    "mov.paz",
]
MAIN_EXE = "eden.exe"
PM_EXE = "eden_pm_en.exe"

# GitHub rejects assets above 2 GiB; stay a little below and leave room for
# the release API's own accounting.
MAX_ASSET = 1_900 * 1024 * 1024
HARD_LIMIT = 2 * 1024 * 1024 * 1024

PM_DIR_NAME = "eden PLUS MOSAIC"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, **kwargs)


def split_file(path: Path, chunk: int = MAX_ASSET) -> list[Path]:
    """Split ``path`` into ``<name>.partNN`` volumes and remove the original."""
    parts: list[Path] = []
    with path.open("rb") as fh:
        index = 1
        while True:
            chunk_data = fh.read(chunk)
            if not chunk_data:
                break
            part = path.with_name(f"{path.name}.part{index:02d}")
            part.write_bytes(chunk_data)
            parts.append(part)
            index += 1
    path.unlink()
    return parts


def macho_has_signature(binary: Path) -> bool:
    data = binary.read_bytes()[:65536]
    if len(data) < 32:
        return False
    magic, _, _, _, ncmds, _, _, _ = struct.unpack_from("<IiiIIIII", data, 0)
    if magic != 0xFEEDFACF:
        return False
    offset = 32
    for _ in range(ncmds):
        if offset + 8 > len(data):
            return False
        cmd, size = struct.unpack_from("<II", data, offset)
        if cmd == 0x1D:  # LC_CODE_SIGNATURE
            return True
        offset += size
    return False


def zip_entry_mode(ipa: Path, entry: str) -> str | None:
    out = run(["zipinfo", "-l", str(ipa)], capture_output=True, text=True).stdout
    for line in out.splitlines():
        fields = line.split()
        if len(fields) >= 4 and fields[-1] == entry:
            return fields[0]
    return None


def stage_gamedata(mode: str, game: Path, app: Path) -> dict:
    """Copy the requested retail files below ``Eden.app/gamedata``."""
    gamedata = app / "gamedata"
    if gamedata.exists():
        shutil.rmtree(gamedata)
    gamedata.mkdir(parents=True)

    copied = {"files": 0, "bytes": 0, "layout": []}

    def add(src: Path, dst: Path) -> None:
        if not src.is_file():
            raise SystemExit(f"missing required file: {src}")
        dst.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(src, dst)
        copied["files"] += 1
        copied["bytes"] += src.stat().st_size
        copied["layout"].append(str(dst.relative_to(gamedata)))

    if mode in ("main", "all"):
        for name in MAIN_ARCHIVES + [MAIN_EXE]:
            add(game / name, gamedata / name)

    if mode == "pm":
        # Keep the "PLUS" directory name: Runtime::open() then treats it as the
        # PLUS+MOSAIC edition and the title menu can switch editions cleanly.
        for name in MAIN_ARCHIVES + [PM_EXE]:
            add(game / PM_DIR_NAME / name, gamedata / PM_DIR_NAME / name)

    if mode == "all":
        for name in MAIN_ARCHIVES + [PM_EXE]:
            add(game / PM_DIR_NAME / name, gamedata / PM_DIR_NAME / name)

    return copied


def build(args: argparse.Namespace) -> int:
    mode = args.mode
    names = {
        "main": "Eden-Full-Main-iOS-arm64.ipa",
        "pm": "Eden-Full-PlusMosaic-iOS-arm64.ipa",
        "all": "Eden-Full-AllInOne-iOS-arm64.ipa",
    }
    ipa_name = names[mode]
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    work = out / f".work-{mode}"
    if work.exists():
        shutil.rmtree(work)
    work.mkdir(parents=True)

    print(f"==> extracting {args.shell}")
    run(["unzip", "-q", str(Path(args.shell).resolve()), "-d", str(work)])
    app = work / "Payload/Eden.app"
    binary = app / "Eden"
    if not binary.is_file():
        raise SystemExit(f"{args.shell} has no Payload/Eden.app/Eden")

    print(f"==> staging gamedata ({mode})")
    gamedata = stage_gamedata(mode, Path(args.game), app)
    print(f"    {gamedata['files']} files, {gamedata['bytes']:,} bytes")

    font = Path(args.font)
    if not font.is_file():
        raise SystemExit(f"missing font: {font}")
    shutil.copy2(font, app / "font.ttf")

    binary.chmod(0o755)
    if not macho_has_signature(binary):
        raise SystemExit("Payload/Eden.app/Eden lost its code signature")

    ipa = out / ipa_name
    if ipa.exists():
        ipa.unlink()
    print(f"==> zip -0 -> {ipa}")
    run(["zip", "-qry", "-X", "-0", str(ipa), "Payload"], cwd=work)

    size = ipa.stat().st_size
    digest = sha256_file(ipa)
    entry_mode = zip_entry_mode(ipa, "Payload/Eden.app/Eden")
    if entry_mode != "-rwxr-xr-x":
        raise SystemExit(f"unexpected executable mode in zip: {entry_mode}")

    parts: list[dict] = []
    if size > MAX_ASSET:
        print(f"==> {size:,} bytes exceeds the 2 GiB release limit; splitting")
        paths = split_file(ipa)
        parts = [
            {"file": p.name, "bytes": p.stat().st_size, "sha256": sha256_file(p)}
            for p in paths
        ]
        if any(p["bytes"] > HARD_LIMIT for p in parts):
            raise SystemExit("a volume is still above 2 GiB")

    manifest = {
        "name": ipa_name,
        "mode": mode,
        "bytes": size,
        "sha256": digest,
        "split": bool(parts),
        "parts": parts,
        "executable_mode": entry_mode,
        "code_signature": True,
        "font": {"file": "Payload/Eden.app/font.ttf", "bytes": font.stat().st_size},
        "gamedata": gamedata,
        "shell": {
            "file": Path(args.shell).name,
            "sha256": sha256_file(Path(args.shell)),
        },
    }
    manifest_path = out / f"{ipa_name}.manifest.json"
    manifest_path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(json.dumps({k: manifest[k] for k in ("name", "bytes", "split", "parts")}, indent=2))

    if not args.keep_work:
        shutil.rmtree(work)
    return 0


def make_cache(args: argparse.Namespace) -> int:
    """Write gamedata-main.tar / gamedata-pm.tar (+ part volumes) for CI."""
    out = Path(args.out).resolve()
    out.mkdir(parents=True, exist_ok=True)
    game = Path(args.game)
    targets = [
        ("gamedata-main.tar", "gamedata", game, MAIN_ARCHIVES + [MAIN_EXE]),
        (
            "gamedata-pm.tar",
            f"gamedata/{PM_DIR_NAME}",
            game / PM_DIR_NAME,
            MAIN_ARCHIVES + [PM_EXE],
        ),
    ]
    manifest = {"archives": []}
    for name, arcroot, src_dir, files in targets:
        tar_path = out / name
        if tar_path.exists():
            tar_path.unlink()
        with tarfile.open(tar_path, "w", format=tarfile.GNU_FORMAT) as tar:
            tar.add(src_dir / files[0], arcname=f"{arcroot}/{files[0]}")
            for filename in files[1:]:
                tar.add(src_dir / filename, arcname=f"{arcroot}/{filename}")
        size = tar_path.stat().st_size
        entry = {"file": name, "bytes": size, "sha256": sha256_file(tar_path)}
        if size > MAX_ASSET:
            parts = split_file(tar_path)
            entry["parts"] = [
                {"file": p.name, "bytes": p.stat().st_size, "sha256": sha256_file(p)}
                for p in parts
            ]
        manifest["archives"].append(entry)
        print(f"{name}: {size:,} bytes")
    (out / "gamedata-cache.manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    build_p = sub.add_parser("build", help="build a self-contained IPA")
    build_p.add_argument("--mode", choices=["main", "pm", "all"], required=True)
    build_p.add_argument("--shell", default=str(DEFAULT_SHELL))
    build_p.add_argument("--game", default=str(DEFAULT_GAME))
    build_p.add_argument("--out", default=str(DEFAULT_OUT))
    build_p.add_argument("--font", default=str(DEFAULT_FONT))
    build_p.add_argument("--keep-work", action="store_true")
    build_p.set_defaults(func=build)

    cache_p = sub.add_parser("cache", help="tar the retail data for CI")
    cache_p.add_argument("--game", default=str(DEFAULT_GAME))
    cache_p.add_argument("--out", default=str(DEFAULT_OUT / "cache"))
    cache_p.set_defaults(func=make_cache)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
