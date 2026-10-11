import { Box, Button, Group } from "@mantine/core";
import { Icon, iconFolderPlus, iconPlus } from "@surrealdb/ui";
import { SurrealWordmark } from "./SurrealLogo";

export function CampaignHero({
    onCreateFile,
    onCreateFolder,
    onOpenCanvas,
}: {
    onCreateFile: () => void;
    onCreateFolder: () => void;
    onOpenCanvas: () => void;
}) {
    return (
        <Box className="campaign-container">
            {/* The Cover */}
            <header className="cover">
                <div className="cover-in">
                    <div className="wm">
                        <SurrealWordmark />
                    </div>
                    <div>
                        <div className="eyebrow">
                            SurrealFS · Virtual Agent Filesystem · SurrealDB 3.x
                        </div>
                        <h1>Stop choosing.</h1>
                        <p className="sub">The answer is always the same.</p>
                    </div>
                </div>
            </header>

            {/* The Core Manifesto Band */}
            <section className="band alt">
                <div className="in">
                    <p className="deck-lede">
                        Databases and filesystems have always forced a choice: speed or durability, in-memory or
                        persistent, memory or accuracy, files or database, POSIX or graph, simplicity or
                        scale. Across SurrealFS, <span className="pink">the answer is always yes.</span>
                    </p>

                    <div className="qs">
                        Speed or durability?<br />
                        In-memory or persistent?<br />
                        Memory or accuracy?<br />
                        Files or database?<br />
                        Simplicity or scale?
                    </div>

                    <div className="bigyes">
                        Yes<i className="sq" />
                    </div>

                    <p className="how">
                        SurrealFS unifies high-performance virtual filesystem semantics, ACID transactional storage,
                        instant zero-copy workspace branching, and vector semantic memory directly within SurrealDB 3.x.
                    </p>

                    {/* Feature Cards Grid */}
                    <div className="grid3" style={{ marginTop: "clamp(32px, 4vw, 56px)" }}>
                        <div className="card">
                            <div className="status new">POSIX</div>
                            <div className="when">VIRTUAL FS CORE</div>
                            <h4>Hierarchical Filesystem</h4>
                            <p>
                                Pure-engine filesystem with paths, BLAKE3 deduplication, advisory locking, and
                                microsecond CRUD.
                            </p>
                            <Group gap="xs" mt="md">
                                <Button
                                    size="xs"
                                    className="btn-pink"
                                    leftSection={<Icon path={iconPlus} size="xs" />}
                                    onClick={onCreateFile}
                                >
                                    New File
                                </Button>
                                <Button
                                    size="xs"
                                    className="btn-outline"
                                    leftSection={<Icon path={iconFolderPlus} size="xs" />}
                                    onClick={onCreateFolder}
                                >
                                    New Folder
                                </Button>
                            </Group>
                        </div>

                        <div className="card">
                            <div className="status keep">SPATIAL</div>
                            <div className="when">BRAIN STUDIO 2.0</div>
                            <h4>Force-Directed Canvas</h4>
                            <p>
                                Visualize nodes, entity relations, and real-time agent locks on an interactive
                                2D physics graph.
                            </p>
                            <Group gap="xs" mt="md">
                                <Button
                                    size="xs"
                                    className="btn-purple"
                                    onClick={onOpenCanvas}
                                >
                                    Open Canvas ✦
                                </Button>
                            </Group>
                        </div>

                        <div className="card">
                            <div className="status plan">AGENTS</div>
                            <div className="when">COLLABORATION</div>
                            <h4>Autonomous Swarms</h4>
                            <p>
                                Multi-agent actor mailboxes, transparent CRDT file compaction, and vector memory
                                recall.
                            </p>
                            <Group gap="xs" mt="md">
                                <span className="mono" style={{ color: "var(--pink)" }}>
                                    USE THE AGENT PANEL →
                                </span>
                            </Group>
                        </div>
                    </div>
                </div>
            </section>
        </Box>
    );
}
