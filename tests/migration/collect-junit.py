"""Freeze collected frontend test identities from successful baseline JUnit reports."""

import hashlib
import json
from pathlib import Path
import subprocess
import xml.etree.ElementTree as element_tree


ROOT = Path(__file__).resolve().parents[2]
BASELINE = "6bb1220059c82c19a53a1418fc376fc842fea834"
REPORTS = (
    ("unit", "app/test-results/vitest/migration-baseline.xml", "app/", 249),
    ("browser", "app/test-results/vitest-browser/migration-baseline.xml", "app/", 7),
    ("fixture", "app/test-results/playwright-fixtures/junit.xml", "app/tests/e2e/", 32),
    ("full-stack", "app/test-results/playwright-full-stack/junit.xml", "app/tests/e2e/full-stack/", 6),
)


def main():
    """Validate reports and source identities before publishing the frozen inventory."""
    layers = []
    for layer, report_path, prefix, expected_count in REPORTS:
        raw = (ROOT / report_path).read_bytes()
        document = element_tree.fromstring(raw)
        tests = []
        sources = {}
        for suite in document.iter("testsuite"):
            for case in suite.findall("testcase"):
                if any(case.find(tag) is not None for tag in ("failure", "error", "skipped")):
                    raise ValueError(f"Unsuccessful baseline case: {case.attrib}")
                source_path = prefix + case.attrib["classname"].replace("\\", "/")
                committed = subprocess.check_output(
                    ["git", "show", f"{BASELINE}:{source_path}"], cwd=ROOT
                )
                current = (ROOT / source_path).read_bytes()
                if current.replace(b"\r\n", b"\n") != committed.replace(b"\r\n", b"\n"):
                    raise ValueError(f"Changed baseline source: {source_path}")
                sources[source_path] = hashlib.sha256(committed).hexdigest()
                tests.append({
                    "id": f"{layer}::{source_path}::{case.attrib['name']}",
                    "source": source_path,
                    "name": case.attrib["name"],
                    "project": suite.get("hostname"),
                    "baselineStatus": "Passed",
                    "disposition": "preserved",
                    "groups": ["M24"],
                    "leaves": ["C31"],
                    "owner": "T11",
                    "firstProof": "G2",
                    "finalIntegration": "T14/G2",
                    "reason": "Keep the original frontend assertion and exercise it against the Go runtime",
                })
        if len(tests) != expected_count or len({case["id"] for case in tests}) != expected_count:
            raise ValueError(f"Unexpected or duplicate collected cases: {layer}")
        layers.append({"layer": layer, "report": report_path,
                       "reportSha256": hashlib.sha256(raw).hexdigest(),
                       "sourceHashes": sources, "tests": tests})
    target = ROOT / "tests/data/migration/frontend-tests.json"
    target.write_text(json.dumps({"baseline": BASELINE, "layers": layers}, indent=2) + "\n", encoding="utf-8")
    print(f"Recorded {sum(len(layer['tests']) for layer in layers)} frontend cases")


if __name__ == "__main__":
    main()
