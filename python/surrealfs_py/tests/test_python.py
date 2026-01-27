# TODO: generate types
from surrealfs_py import PySurrealFs  # type: ignore


def test_mem_roundtrip() -> None:
    fs = PySurrealFs.mem()

    assert fs.mkdir("/code", True) == ""
    assert fs.write_file("/code/readme.md", "hi there") == ""

    ls_out = fs.ls("/code").strip().splitlines()
    assert "readme.md" in ls_out[0]

    assert fs.cat("/code/readme.md") == "hi there"

    nl_out = fs.nl("/code/readme.md", 1).splitlines()
    assert nl_out == ["   1  hi there"]

    grep_out = fs.grep("hi", "/code", True).strip().splitlines()
    assert any(line.startswith("/code/readme.md:1:") for line in grep_out)


def test_read_and_edit() -> None:
    fs = PySurrealFs.mem()

    fs.mkdir("/code", True)
    fs.write_file("/code/app.txt", "alpha\nbeta\ngamma\nbeta")

    assert fs.read("/code/app.txt", 1, 2) == "beta\ngamma\n"

    diff = fs.edit("/code/app.txt", "beta", "BETA")
    assert diff.startswith("--- original")
    assert fs.cat("/code/app.txt") == "alpha\nBETA\ngamma\nbeta"

    diff_all = fs.edit("/code/app.txt", "beta", "BETA", True)
    assert diff_all.startswith("--- original")
    assert fs.cat("/code/app.txt") == "alpha\nBETA\ngamma\nBETA"
