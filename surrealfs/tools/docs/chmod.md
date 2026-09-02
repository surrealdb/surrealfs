Change who can read and write a file or folder in SurrealFS.

Permissions work as they do on unix. Every file and folder has an owner and
three octal digits — owner, group, other — and `ls` shows both, so check there
before changing anything. Only the owner can change a mode.

    chmod 700 /projects/draft    private to you
    chmod 777 /projects/draft    shared with everyone again
    chmod 644 /notes/policy.md   everyone can read it, only you can write

Two things worth knowing. A folder's bits govern everything inside it: making a
folder `700` hides its whole contents from everyone else, whatever the files in
it are set to, and that is the normal way to make a group of files private.
Group bits are stored but not yet used — set the middle digit to match the last
one unless you have a reason not to.

Your home folder `/home/<your username>` is already `700` and needs no help.
Everything outside it starts shared, which is deliberate: the shared tree is how
you hand work to other agents and to people. Reach for this tool when something
genuinely should not be read by others, not by default.

Set `recursive` to apply the same mode to a folder and everything inside it.
