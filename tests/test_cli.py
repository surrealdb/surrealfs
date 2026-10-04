"""Tests for the unified surrealfs CLI."""

from __future__ import annotations

import pytest

from surrealfs import SurrealFs
from surrealfs.__main__ import main


@pytest.mark.asyncio
async def test_cli_version(capsys):
    with pytest.raises(SystemExit) as exc:
        main(["--version"])
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "surrealfs" in captured.out


@pytest.mark.asyncio
async def test_cli_help(capsys):
    with pytest.raises(SystemExit) as exc:
        main(["--help"])
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "status" in captured.out
    assert "schema" in captured.out
    assert "grep" in captured.out


@pytest.mark.asyncio
async def test_cli_grep_and_lock(
    fs: SurrealFs, surreal_url: str, namespace: str, monkeypatch, capsys
):
    # Populate a test file
    await fs.write_text("/cli-test/hello.txt", "hello world from surrealfs CLI")

    # Point CLI to the test database
    monkeypatch.setenv("SURREALDB_URL", surreal_url)
    monkeypatch.setenv("SURREALDB_NAMESPACE", namespace)
    monkeypatch.setenv("SURREALDB_DATABASE", "test")
    monkeypatch.setenv("SURREALDB_USER", "root")
    monkeypatch.setenv("SURREALDB_PASS", "root")
    monkeypatch.setenv("SURREALDB_AUTH_LEVEL", "root")

    # Test grep
    with pytest.raises(SystemExit) as exc:
        main(["grep", "hello", "--path", "/cli-test"])
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "/cli-test/hello.txt:1:hello world from surrealfs CLI" in captured.out

    # Test lock acquire and release
    with pytest.raises(SystemExit) as exc:
        main(
            [
                "lock",
                "acquire",
                "/cli-test/hello.txt",
                "--ttl",
                "30",
                "--reason",
                "CLI test",
            ]
        )
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "Acquired lease on /cli-test/hello.txt" in captured.out

    # Test lock list
    with pytest.raises(SystemExit) as exc:
        main(["lock", "list"])
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "/cli-test/hello.txt" in captured.out

    # Test lock release
    with pytest.raises(SystemExit) as exc:
        main(["lock", "release", "/cli-test/hello.txt"])
    assert exc.value.code == 0
    captured = capsys.readouterr()
    assert "Released lease on /cli-test/hello.txt" in captured.out
