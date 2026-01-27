from surrealfs_py import PySurrealFs  # pyright: ignore[reportAttributeAccessIssue]


def main() -> None:
    fs = PySurrealFs.mem()
    fs.mkdir("/demo", True)
    fs.write_file("/demo/hello.txt", "hello\nworld")

    print("ls:\n" + fs.ls("/demo"))
    print("cat:\n" + fs.cat("/demo/hello.txt"))
    print("tail:\n" + fs.tail("/demo/hello.txt", 1))
    print("nl:\n" + fs.nl("/demo/hello.txt", 1))


if __name__ == "__main__":
    main()
