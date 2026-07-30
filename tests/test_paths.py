"""Path normalisation and glob translation — no database needed."""

from __future__ import annotations

import pytest

from surrealfs import paths


@pytest.mark.parametrize(
    ("raw", "expected"),
    [
        ("/a/b", "/a/b"),
        ("a/b", "/a/b"),
        ("//a///b//", "/a/b"),
        ("/a/./b", "/a/b"),
        ("/a/b/..", "/a"),
        ("/a/../b", "/b"),
        ("", "/"),
        ("/", "/"),
        ("/..", "/"),
        ("/../../etc/passwd", "/etc/passwd"),  # cannot escape the root
    ],
)
def test_normalize(raw, expected):
    assert paths.normalize(raw) == expected


def test_split_and_basename():
    assert paths.split("/a/b/c.md") == ["a", "b", "c.md"]
    assert paths.split("/") == []
    assert paths.basename("/a/b/c.md") == "c.md"
    assert paths.basename("/") == ""
    assert paths.parent_of("/a/b/c.md") == "/a/b"
    assert paths.parent_of("/a") == "/"
    assert paths.parent_of("/") == "/"


@pytest.mark.parametrize(
    ("pattern", "path", "matches"),
    [
        ("/*.md", "/a.md", True),
        ("/*.md", "/sub/a.md", False),  # * does not cross a slash
        ("/**/*.md", "/a.md", True),  # ** also matches zero directories
        ("/**/*.md", "/x/y/a.md", True),
        ("/notes/**/*.md", "/notes/a.md", True),
        ("/notes/**/*.md", "/notes/deep/a.md", True),
        ("/notes/**/*.md", "/other/a.md", False),
        ("/a?.txt", "/ab.txt", True),
        ("/a?.txt", "/abc.txt", False),
        ("/[ab].md", "/a.md", True),
        ("/[!ab].md", "/a.md", False),
        ("/[!ab].md", "/c.md", True),
    ],
)
def test_glob_to_regex(pattern, path, matches):
    assert bool(paths.glob_to_regex(pattern).match(path)) is matches


@pytest.mark.parametrize(
    ("pattern", "prefix"),
    [
        ("/notes/**/*.md", "/notes/"),
        ("/notes/deep/*.md", "/notes/deep/"),
        ("/*.md", "/"),
        ("/a/b/c.md", "/a/b/"),
        ("/**/*", "/"),
    ],
)
def test_literal_prefix(pattern, prefix):
    assert paths.literal_prefix(pattern) == prefix


def test_literal_prefix_is_a_real_ancestor_of_matches():
    """The prefix must never exclude a path the pattern would match."""
    pattern = "/notes/**/*.md"
    regex = paths.glob_to_regex(pattern)
    prefix = paths.literal_prefix(pattern)
    for candidate in ["/notes/a.md", "/notes/x/y/b.md"]:
        assert regex.match(candidate)
        assert candidate.startswith(prefix)
