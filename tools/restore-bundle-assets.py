#!/usr/bin/env python3
"""Restore the published bundle paths before replacing the Workers Assets deployment."""

import concurrent.futures
import hashlib
import json
from pathlib import Path
import re
import subprocess
import sys

TARGETS = ("aarch64-unknown-linux-musl", "x86_64-unknown-linux-musl")
LEGACY_AMD64_ONLY = {"v0.5.1-canary.2"}
VERSION = re.compile(r"v\d+\.\d+\.\d+(?:-canary\.\d+)?")
DIGEST = re.compile(r"[0-9a-f]{64}")
CHUNK_LIMIT = 25 * 1024 * 1024


def fetch(origin, relative, output, *, optional=False, limit=CHUNK_LIMIT):
    temp = output.with_name(output.name + ".download")
    response = subprocess.run(
        ["curl", "--silent", "--show-error", "--retry", "3", "--retry-delay", "1",
         "--connect-timeout", "10", "--max-time", "90", "--max-filesize", str(limit),
         "--output", str(temp), "--write-out", "%{http_code}", f"{origin}/{relative}"],
        capture_output=True, text=True, check=False,
    )
    if response.returncode == 0 and response.stdout == "200":
        temp.replace(output)
        return True
    temp.unlink(missing_ok=True)
    if optional and response.returncode == 0 and response.stdout == "404":
        return False
    raise RuntimeError(f"cannot restore {relative}: HTTP {response.stdout}, curl {response.returncode}")


def parse_manifest(text, archive):
    version, checksum, length, *records = text.splitlines()
    if version != "TUNNEL_AGENT_BUNDLE_PARTS=1" or not checksum.startswith("ARCHIVE_SHA256=") or not length.startswith("ARCHIVE_BYTES="):
        raise ValueError(f"invalid manifest for {archive}")
    digest, size = checksum.split("=", 1)[1], int(length.split("=", 1)[1])
    if not DIGEST.fullmatch(digest) or not 1 <= len(records) <= 64:
        raise ValueError(f"invalid archive identity for {archive}")
    chunks = []
    for index, record in enumerate(records):
        kind, part_digest, part_length, name = record.split()
        part_size = int(part_length)
        if (kind != "PART" or name != f"{archive}.part-{index:04d}"
                or not DIGEST.fullmatch(part_digest) or not 0 < part_size <= CHUNK_LIMIT):
            raise ValueError(f"invalid chunk record for {archive}")
        chunks.append((part_digest, part_size, name))
    if sum(part_size for _, part_size, _ in chunks) != size:
        raise ValueError(f"invalid archive size for {archive}")
    return digest, chunks


def restore_archive(origin, base, version, target, legacy):
    archive = f"tunnel-agent-{version}-{target}.tar.gz"
    prefix = f"bundles/{version}"
    files = base / prefix
    files.mkdir(parents=True, exist_ok=True)
    manifest = files / f"{archive}.parts"
    fetch(origin, f"{prefix}/{manifest.name}", manifest, limit=64 * 1024)
    checksum, chunks = parse_manifest(manifest.read_text(encoding="ascii"), archive)
    inventory = files / f"{archive}.inventory.json"
    if not fetch(origin, f"{prefix}/{inventory.name}", inventory, optional=legacy, limit=1024 * 1024):
        inventory.unlink(missing_ok=True)
    joined = hashlib.sha256()
    for digest, size, name in chunks:
        path = files / name
        if not path.is_file() or hashlib.sha256(path.read_bytes()).hexdigest() != digest:
            fetch(origin, f"{prefix}/{name}", path)
        content = path.read_bytes()
        if len(content) != size or hashlib.sha256(content).hexdigest() != digest:
            raise ValueError(f"chunk checksum or length differs for {name}")
        joined.update(content)
    if joined.hexdigest() != checksum:
        raise ValueError(f"archive checksum differs for {archive}")


def restore(origin, base, legacy_path, release):
    if not VERSION.fullmatch(release):
        raise ValueError("invalid release version")
    legacy = set(legacy_path.read_text(encoding="ascii").split())
    index = base / "bundles/index.json"
    index.parent.mkdir(parents=True, exist_ok=True)
    past = json.loads(index.read_text()) if fetch(origin, "bundles/index.json", index, optional=True, limit=64 * 1024) else []
    if not isinstance(past, list) or len(past) > 200 or any(not isinstance(tag, str) or not VERSION.fullmatch(tag) for tag in past):
        raise ValueError("invalid published bundle index")
    versions = legacy | set(past)
    if any(not VERSION.fullmatch(tag) for tag in versions):
        raise ValueError("invalid legacy bundle index")
    with concurrent.futures.ThreadPoolExecutor(max_workers=8) as pool:
        jobs = [pool.submit(restore_archive, origin, base, version, target, version in legacy)
                for version in sorted(versions) for target in TARGETS
                if version not in LEGACY_AMD64_ONLY or target == TARGETS[1]]
        for job in concurrent.futures.as_completed(jobs):
            job.result()
    index.write_text(json.dumps(sorted(versions | {release}), separators=(",", ":")) + "\n", encoding="ascii")
    print(f"Restored {len(versions)} published bundle versions before adding {release}.")


if __name__ == "__main__":
    restore(sys.argv[1].rstrip("/"), Path(sys.argv[2]), Path(sys.argv[3]), sys.argv[4])
