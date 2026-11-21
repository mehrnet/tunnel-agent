import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("assets", Path(__file__).with_name("restore-bundle-assets.py"))
assets = importlib.util.module_from_spec(spec)
spec.loader.exec_module(assets)


def bundle_files(version):
    for target in assets.TARGETS:
        archive = f"tunnel-agent-{version}-{target}.tar.gz"
        name = f"{archive}.part-0000"
        content = target.encode("ascii")
        digest = hashlib.sha256(content).hexdigest()
        manifest = f"TUNNEL_AGENT_BUNDLE_PARTS=1\nARCHIVE_SHA256={digest}\nARCHIVE_BYTES={len(content)}\n"
        prefix = f"bundles/{version}/"
        yield prefix + archive + ".parts", (manifest + f"PART {digest} {len(content)} {name}\n").encode()
        yield prefix + archive + ".inventory.json", b"{}"
        yield prefix + name, content


class RestoreTests(unittest.TestCase):
    def test_keeps_old_paths_and_rejects_corrupt_chunks(self):
        original = dict(bundle_files("v0.5.5")) | dict(bundle_files("v0.5.7"))
        original["bundles/index.json"] = b'["v0.5.7"]'

        def fetch(_origin, path, output, *, optional=False, limit=None):
            if path in original:
                output.write_bytes(original[path])
                return True
            if optional:
                return False
            raise RuntimeError(f"missing {path}")

        with tempfile.TemporaryDirectory() as directory, patch.object(assets, "fetch", fetch):
            base = Path(directory) / "site"
            legacy = Path(directory) / "legacy.txt"
            legacy.write_text("v0.5.5\n")
            assets.restore("https://files.example", base, legacy, "v0.5.8")
            for relative, content in original.items():
                if relative != "bundles/index.json":
                    self.assertEqual((base / relative).read_bytes(), content)
            index = base / "bundles/index.json"
            self.assertEqual(json.loads(index.read_text()), ["v0.5.5", "v0.5.7", "v0.5.8"])
            part = next(path for path in original if path.endswith(".part-0000"))
            (base / part).unlink()
            original[part] = b"broken"
            with self.assertRaisesRegex(ValueError, "chunk checksum"):
                assets.restore("https://files.example", base, legacy, "v0.5.8")


if __name__ == "__main__":
    unittest.main()
