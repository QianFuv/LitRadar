"""Contracts for retired Python backend runtime entrypoints."""

from __future__ import annotations

import tomllib
import unittest
from pathlib import Path

import paper_scanner.api as api_package
import paper_scanner.index as index_package
import paper_scanner.notify as notify_package
import paper_scanner.push as push_package

PROJECT_ROOT = Path(__file__).resolve().parents[2]


class PythonRuntimeEntrypointRetirementContractTest(unittest.TestCase):
    """Verify Python backend modules remain references, not runtime entrypoints."""

    def test_pyproject_no_longer_exposes_backend_runtime_scripts(self) -> None:
        """Verify package metadata no longer installs Python backend commands."""
        with (PROJECT_ROOT / "pyproject.toml").open("rb") as pyproject_file:
            pyproject = tomllib.load(pyproject_file)

        scripts = pyproject.get("project", {}).get("scripts", {})

        for command in ("api", "index", "notify", "push"):
            self.assertNotIn(command, scripts)

    def test_python_backend_packages_do_not_advertise_main_exports(self) -> None:
        """Verify Python compatibility packages do not advertise runtime exports."""
        packages = (api_package, index_package, notify_package, push_package)

        for package in packages:
            self.assertEqual([], package.__all__)
