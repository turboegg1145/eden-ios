#!/usr/bin/env python3
"""Build self-contained (retail data embedded) Eden IPAs.

The CI workflow produces a small shell IPA (``Eden-iOS-arm64.ipa``). This tool
injects the retail archives into ``Payload/Eden.app/gamedata`` and writes a
single IPA, or <= 1.9 GiB volumes when the package exceeds GitHub's 2 GiB
per-asset limit.

The work is split into resumable steps so each one fits in a short shell
timeout on a low-RAM host:

    python3 tools/package_ipa.py stage    --mode all
    python3 tools/package_ipa.py zip      --mode all
    python3 tools/package_ipa.py finalize --mode all

``build`` runs all three. ``cache`` writes the raw tars consumed by
``ios-ipa.yml`` when dispatched with ``embed=main|pm|all``.
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
PM_DIR_NAME = "eden PLUS MOSAIC"

IPA_NAMES = {
    "main": "Eden-Full-Main-iOS-arm64.ipa",
    "pm": "Eden-Full-PlusMosaic-iOS-arm64.ipa",
    "all": "Eden-Full-AllInOne-iOS-arm64.ipa",
}

# GitHub rejects assets above 2 GiB; stay a little below.
MAX_ASSET = 1_900 * 1024 * 1024
HARD_LIMIT = 2 * 1024 * 1024 * 1024


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as fh:
        for chunk in iter(lambda: fh.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def sha256_parts(paths: list[Path]) -> str:
    digest = hashlib.sha256()
    for path in paths:
        with path.open("rb") as fh:
            for chunk in iter(lambda: fh.read(1 << 20), b""):
                digest.update(chunk)
    return digest.hexdigest()


def run(cmd: list[str], **kwargs) -> subprocess.CompletedProcess:
    return subprocess.run(cmd, check=True, **kwargs)


def copy_file(src: Path, dst: Path) -> None:
    """Bounded-memory copy.

    WSL's 9p/drvfs returns ENOMEM after ~500 MiB of cached reads on a 2.8 GiB
    host, so read with fadvise(DONTNEED) and flush the destination regularly.
    """
    in_fd = os.open(src, os.O_RDONLY)
    out_fd = os.open(dst, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o644)
    total = 0
    try:
        while True:
            chunk = os.read(in_fd, 1 << 20)
            if not chunk:
                break
            written = 0
            while written < len(chunk):
                written += os.write(out_fd, chunk[written:])
            os.posix_fadvise(in_fd, total, len(chunk), os.POSIX_FADV_DONTNEED)
            total += len(chunk)
            if total % (256 << 20) < (1 << 20):
                os.fsync(out_fd)
                os.posix_fadvise(out_fd, 0, total, os.POSIX_FADV_DONTNEED)
    finally:
        os.close(in_fd)
        os.close(out_fd)
    shutil.copystat(src, dst)


def split_file(path: Path, chunk: int = MAX_ASSET) -> list[Path]:
    """Split ``path`` into ``<name>.partNN`` volumes and remove the original."""
    parts: list[Path] = []
    with path.open("rb") as fh:
        index = 1
        while True:
            data = fh.read(chunk)
            if not data:
                break
            part = path.with_name(f"{path.name}.part{index:02d}")
            part.write_bytes(data)
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


def zip_directory(tail_paths: list[Path]) -> dict:
    """Parse the zip central directory that lives at the end of the package."""
    tail = tail_paths[-1].read_bytes()[-4 << 20 :]
    index = tail.rfind(b"PK\x05\x06")
    if index < 0:
        raise SystemExit("zip end-of-central-directory record not found")
    (_, _, _, _, entries, cd_size, cd_off, _) = struct.unpack_from(
        "<IHHHHIIH", tail, index
    )
    cd_start = index - cd_size
    if cd_start < 0:
        raise SystemExit("central directory is larger than the verification window")
    result = {"entries": entries, "files": {}, "gamedata_bytes": 0, "gamedata_files": 0}
    offset = cd_start
    for _ in range(entries):
        if tail[offset : offset + 4] != b"PK\x01\x02":
            raise SystemExit("corrupt zip central directory")
        (
            _,
            _ver_made,
            _ver_need,
            _flags,
            _method,
            _mtime,
            _mdate,
            _crc,
            _csize,
            usize,
            nlen,
            elen,
            clen,
            _disk,
            _iattr,
            eattr,
            _lho,
        ) = struct.unpack_from("<IHHHHHHIIIHHHHHII", tail, offset)
        name = tail[offset + 46 : offset + 46 + nlen].decode("utf-8", "replace")
        offset += 46 + nlen + elen + clen
        mode = (eattr >> 16) & 0o777
        result["files"][name] = {"bytes": usize, "mode": mode}
        if name.startswith("Payload/Eden.app/gamedata/"):
            result["gamedata_bytes"] += usize
            result["gamedata_files"] += 1
    return result


def stage_work(
    mode: str,
    shell: Path,
    game: Path,
    font: Path,
    out: Path,
    only: str | None = None,
    append: bool = False,
) -> Path:
    out = out.resolve()
    work = out / f".work-{mode}"
    if append:
        if not (work / "Payload/Eden.app/Eden").is_file():
            raise SystemExit(f"{work} is not staged; drop --append")
    else:
        if work.exists():
            shutil.rmtree(work)
        work.mkdir(parents=True)
        print(f"==> extracting {shell}")
        run(["unzip", "-q", str(shell), "-d", str(work)])
    app = work / "Payload/Eden.app"
    binary = app / "Eden"
    if not binary.is_file():
        raise SystemExit(f"{shell} has no Payload/Eden.app/Eden")

    gamedata = app / "gamedata"
    gamedata.mkdir(exist_ok=True)

    def add(src: Path, dst: Path) -> None:
        if not src.is_file():
            raise SystemExit(f"missing required file: {src}")
        dst.parent.mkdir(parents=True, exist_ok=True)
        copy_file(src, dst)

    source = only or mode
    if source in ("main", "all"):
        for name in MAIN_ARCHIVES + [MAIN_EXE]:
            add(game / name, gamedata / name)
    if source in ("pm", "all"):
        for name in MAIN_ARCHIVES + [PM_EXE]:
            add(game / PM_DIR_NAME / name, gamedata / PM_DIR_NAME / name)

    copy_file(font, app / "font.ttf")
    binary.chmod(0o755)
    if not macho_has_signature(binary):
        raise SystemExit("Payload/Eden.app/Eden lost its code signature")
    staged = sum(p.stat().st_size for p in work.rglob("*") if p.is_file())
    print(f"==> staged {work} ({staged:,} bytes)")
    return work


def zip_work(mode: str, out: Path) -> Path:
    out = out.resolve()
    work = out / f".work-{mode}"
    if not (work / "Payload/Eden.app/Eden").is_file():
        raise SystemExit(f"{work} is not staged; run the stage step first")
    ipa = (out / IPA_NAMES[mode]).resolve()
    if ipa.exists():
        ipa.unlink()
    print(f"==> zip -0 -> {ipa}")
    # -0: .paz data is already compressed; store it for speed and fidelity.
    run(["zip", "-qry", "-X", "-0", str(ipa), "Payload"], cwd=work)
    print(f"    {ipa.stat().st_size:,} bytes")
    return ipa


def finalize(mode: str, out: Path, shell: Path, font: Path, do_split: bool = True) -> dict:
    out = out.resolve()
    name = IPA_NAMES[mode]
    ipa = out / name
    parts = sorted(out.glob(f"{name}.part*"))
    if ipa.exists() and parts:
        raise SystemExit(f"both {ipa.name} and split volumes exist")
    if not ipa.exists() and not parts:
        raise SystemExit(f"{name} not found in {out}")
    if ipa.exists():
        size = ipa.stat().st_size
        digest = sha256_file(ipa)
        if size > MAX_ASSET and do_split:
            print(f"==> {size:,} bytes exceeds 1.9 GiB; splitting")
            parts = split_file(ipa)
            digest = sha256_parts(parts)
    else:
        size = sum(p.stat().st_size for p in parts)
        digest = sha256_parts(parts)

    if any(p.stat().st_size > HARD_LIMIT for p in parts):
        raise SystemExit("a volume is still above the 2 GiB release limit")

    meta = zip_directory(parts if parts else [ipa])
    exe = meta["files"].get("Payload/Eden.app/Eden", {})
    if exe.get("mode") != 0o755:
        raise SystemExit(f"unexpected executable mode in zip: {oct(exe.get('mode', 0))}")
    for required in ("Payload/Eden.app/font.ttf", "Payload/Eden.app/Info.plist"):
        if required not in meta["files"]:
            raise SystemExit(f"{required} missing from the package")
    gamedata_entries = [
        n
        for n in meta["files"]
        if n.startswith("Payload/Eden.app/gamedata/") and not n.endswith("/")
    ]
    if not gamedata_entries:
        raise SystemExit("no gamedata embedded")

    manifest = {
        "name": name,
        "mode": mode,
        "bytes": size,
        "sha256": digest,
        "split": bool(parts),
        "parts": [
            {"file": p.name, "bytes": p.stat().st_size, "sha256": sha256_file(p)}
            for p in parts
        ],
        "executable_mode": "-rwxr-xr-x",
        "code_signature": True,
        "zip_entries": meta["entries"],
        "font": {"file": "Payload/Eden.app/font.ttf", "bytes": font.stat().st_size},
        "gamedata": {
            "files": meta["gamedata_files"],
            "bytes": meta["gamedata_bytes"],
            "entries": gamedata_entries,
        },
        "shell": {"file": shell.name, "sha256": sha256_file(shell)},
    }
    path = out / f"{name}.manifest.json"
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n")
    print(
        json.dumps(
            {k: manifest[k] for k in ("name", "bytes", "split", "gamedata")},
            indent=2,
        )
    )
    work = out / f".work-{mode}"
    if work.exists():
        shutil.rmtree(work)
    return manifest


def split_package(mode: str, out: Path, shell: Path, font: Path) -> dict:
    """Split an already-built IPA, then verify and write the manifest."""
    out = out.resolve()
    name = IPA_NAMES[mode]
    ipa = out / name
    if not ipa.is_file():
        raise SystemExit(f"{ipa} not found")
    for stale in out.glob(f"{name}.part*"):
        stale.unlink()
    print(f"==> splitting {ipa.name} ({ipa.stat().st_size:,} bytes)")
    split_file(ipa)
    return finalize(mode, out, shell, font)


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
    staging = out / ".cache-stage"
    if staging.exists():
        shutil.rmtree(staging)
    for name, arcroot, src_dir, files in targets:
        tar_path = out / name
        if tar_path.exists():
            tar_path.unlink()
        stage = staging / arcroot
        stage.mkdir(parents=True, exist_ok=True)
        for filename in files:
            copy_file(src_dir / filename, stage / filename)
        with tarfile.open(tar_path, "w", format=tarfile.GNU_FORMAT) as tar:
            for filename in files:
                tar.add(stage / filename, arcname=f"{arcroot}/{filename}")
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
    shutil.rmtree(staging, ignore_errors=True)
    (out / "gamedata-cache.manifest.json").write_text(
        json.dumps(manifest, indent=2, sort_keys=True) + "\n"
    )
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)

    def common(p: argparse.ArgumentParser) -> None:
        p.add_argument("--mode", choices=["main", "pm", "all"], required=True)
        p.add_argument("--shell", default=str(DEFAULT_SHELL))
        p.add_argument("--game", default=str(DEFAULT_GAME))
        p.add_argument("--out", default=str(DEFAULT_OUT))
        p.add_argument("--font", default=str(DEFAULT_FONT))

    def cmd_stage(a: argparse.Namespace) -> int:
        stage_work(
            a.mode,
            Path(a.shell),
            Path(a.game),
            Path(a.font),
            Path(a.out),
            only=a.only,
            append=a.append,
        )
        return 0

    def cmd_zip(a: argparse.Namespace) -> int:
        zip_work(a.mode, Path(a.out))
        return 0

    def cmd_finalize(a: argparse.Namespace) -> int:
        finalize(a.mode, Path(a.out), Path(a.shell), Path(a.font), do_split=not a.no_split)
        return 0

    def cmd_split(a: argparse.Namespace) -> int:
        split_package(a.mode, Path(a.out), Path(a.shell), Path(a.font))
        return 0

    def cmd_build(a: argparse.Namespace) -> int:
        stage_work(a.mode, Path(a.shell), Path(a.game), Path(a.font), Path(a.out))
        zip_work(a.mode, Path(a.out))
        finalize(a.mode, Path(a.out), Path(a.shell), Path(a.font))
        return 0

    stage_p = sub.add_parser("stage", help="extract the shell and copy retail data")
    common(stage_p)
    stage_p.add_argument("--only", choices=["main", "pm"], help="stage one source only")
    stage_p.add_argument("--append", action="store_true", help="add to an existing work dir")
    stage_p.set_defaults(func=cmd_stage)

    zip_p = sub.add_parser("zip", help="zip a staged work directory")
    zip_p.add_argument("--mode", choices=["main", "pm", "all"], required=True)
    zip_p.add_argument("--out", default=str(DEFAULT_OUT))
    zip_p.set_defaults(func=cmd_zip)

    fin_p = sub.add_parser("finalize", help="split if needed, verify and write the manifest")
    fin_p.add_argument("--mode", choices=["main", "pm", "all"], required=True)
    fin_p.add_argument("--shell", default=str(DEFAULT_SHELL))
    fin_p.add_argument("--font", default=str(DEFAULT_FONT))
    fin_p.add_argument("--out", default=str(DEFAULT_OUT))
    fin_p.add_argument("--no-split", action="store_true")
    fin_p.set_defaults(func=cmd_finalize)

    split_p = sub.add_parser("split", help="split a built IPA and write the manifest")
    split_p.add_argument("--mode", choices=["main", "pm", "all"], required=True)
    split_p.add_argument("--shell", default=str(DEFAULT_SHELL))
    split_p.add_argument("--font", default=str(DEFAULT_FONT))
    split_p.add_argument("--out", default=str(DEFAULT_OUT))
    split_p.set_defaults(func=cmd_split)

    build_p = sub.add_parser("build", help="stage + zip + finalize")
    common(build_p)
    build_p.set_defaults(func=cmd_build)

    cache_p = sub.add_parser("cache", help="tar the retail data for CI")
    cache_p.add_argument("--game", default=str(DEFAULT_GAME))
    cache_p.add_argument("--out", default=str(DEFAULT_OUT / "cache"))
    cache_p.set_defaults(func=make_cache)

    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
