from surrealfs_py import PySurrealFs


def test_mem_roundtrip() -> None:
    fs = PySurrealFs.mem()

    assert fs.mkdir_p("/code") == ""
    assert fs.write_file("/code/readme.md", "hi there") == ""

    ls_out = fs.ls("/code").strip().splitlines()
    assert "readme.md" in ls_out[0]

    assert fs.cat("/code/readme.md") == "hi there"

    nl_out = fs.nl("/code/readme.md", 1).splitlines()
    assert nl_out == ["   1  hi there"]

    grep_out = fs.grep("hi", "/code", True).strip().splitlines()
    assert any(line.startswith("/code/readme.md:1:") for line in grep_out)
