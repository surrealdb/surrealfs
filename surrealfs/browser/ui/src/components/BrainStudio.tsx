import {
    ActionIcon,
    Badge,
    Box,
    Card,
    Group,
    Paper,
    Slider,
    Stack,
    Text,
    TextInput,
    Tooltip,
} from "@mantine/core";
import { useCallback, useEffect, useRef, useState } from "react";

import { json, send } from "../api";
import type { ActivityData, FileLockInfo, FileVersionInfo, GraphData, GraphNode } from "../types";

interface NodePosition {
    x: number;
    y: number;
    vx: number;
    vy: number;
    radius: number;
}

export function BrainStudio({
    onOpen,
    onStatus,
}: {
    onOpen: (path: string) => void;
    onStatus?: (msg: string, error?: boolean) => void;
}) {
    const canvasRef = useRef<HTMLCanvasElement | null>(null);

    const [graphData, setGraphData] = useState<GraphData>({ nodes: [], edges: [] });
    const [activity, setActivity] = useState<ActivityData>({ locks: [], recent_versions: [] });
    const [filter, setFilter] = useState("");
    const [selectedNode, setSelectedNode] = useState<GraphNode | null>(null);

    // Timeline Scrubber state
    const [timelineIndex, setTimelineIndex] = useState<number>(0);
    const [restoring, setRestoring] = useState(false);

    // Pan & Zoom
    const transformRef = useRef({ x: 0, y: 0, scale: 1 });
    const isDraggingRef = useRef(false);
    const dragStartRef = useRef({ x: 0, y: 0 });
    const nodePositionsRef = useRef<Map<string, NodePosition>>(new Map());

    // Fetch graph & activity data
    const refreshData = useCallback(async () => {
        try {
            const [gData, aData] = await Promise.all([
                json<GraphData>("/api/graph"),
                json<ActivityData>("/api/activity"),
            ]);
            setGraphData(gData);
            setActivity(aData);
            if (aData.recent_versions.length > 0) {
                setTimelineIndex(aData.recent_versions.length - 1);
            }
        } catch (err) {
            onStatus?.((err as Error).message, true);
        }
    }, [onStatus]);

    useEffect(() => {
        refreshData();
        const interval = setInterval(refreshData, 5000);
        return () => clearInterval(interval);
    }, [refreshData]);

    // Initialize node physics positions
    useEffect(() => {
        const positions = nodePositionsRef.current;
        const width = 800;
        const height = 600;

        graphData.nodes.forEach((node, idx) => {
            if (!positions.has(node.id)) {
                const angle = (idx / Math.max(1, graphData.nodes.length)) * Math.PI * 2;
                const dist = 100 + Math.random() * 200;
                positions.set(node.id, {
                    x: width / 2 + Math.cos(angle) * dist,
                    y: height / 2 + Math.sin(angle) * dist,
                    vx: 0,
                    vy: 0,
                    radius: node.is_folder ? 16 : Math.max(8, Math.min(24, Math.log2(node.size + 4) * 2)),
                });
            }
        });
    }, [graphData]);

    // Canvas render loop
    useEffect(() => {
        let animId: number;

        const render = () => {
            const canvas = canvasRef.current;
            if (!canvas) return;
            const ctx = canvas.getContext("2d");
            if (!ctx) return;

            const width = canvas.width;
            const height = canvas.height;
            const { x: panX, y: panY, scale } = transformRef.current;
            const positions = nodePositionsRef.current;

            // Simple force-directed relaxation
            graphData.edges.forEach((edge) => {
                const sourcePos = positions.get(edge.source);
                const targetPos = positions.get(edge.target);
                if (sourcePos && targetPos) {
                    const dx = targetPos.x - sourcePos.x;
                    const dy = targetPos.y - sourcePos.y;
                    const dist = Math.sqrt(dx * dy + dy * dy) || 1;
                    const targetDist = edge.relation === "contains" ? 60 : 120;
                    const force = (dist - targetDist) * 0.005;
                    const fx = (dx / dist) * force;
                    const fy = (dy / dist) * force;
                    sourcePos.x += fx;
                    sourcePos.y += fy;
                    targetPos.x -= fx;
                    targetPos.y -= fy;
                }
            });

            // Clear canvas
            ctx.clearRect(0, 0, width, height);

            ctx.save();
            ctx.translate(panX, panY);
            ctx.scale(scale, scale);

            // Draw edges
            graphData.edges.forEach((edge) => {
                const sourcePos = positions.get(edge.source);
                const targetPos = positions.get(edge.target);
                if (!sourcePos || !targetPos) return;

                ctx.beginPath();
                ctx.moveTo(sourcePos.x, sourcePos.y);
                ctx.lineTo(targetPos.x, targetPos.y);

                if (edge.relation === "contains") {
                    ctx.strokeStyle = "rgba(255, 255, 255, 0.12)";
                    ctx.lineWidth = 1;
                    ctx.setLineDash([2, 4]);
                } else if (edge.relation === "references" || edge.relation === "links_to") {
                    ctx.strokeStyle = "rgba(255, 0, 85, 0.45)"; // surreal pink
                    ctx.lineWidth = 2;
                    ctx.setLineDash([]);
                } else if (edge.relation === "implements") {
                    ctx.strokeStyle = "rgba(92, 124, 250, 0.5)"; // blue
                    ctx.lineWidth = 2;
                    ctx.setLineDash([]);
                } else {
                    ctx.strokeStyle = "rgba(81, 207, 102, 0.4)"; // green
                    ctx.lineWidth = 1.5;
                    ctx.setLineDash([]);
                }
                ctx.stroke();
            });

            ctx.setLineDash([]);

            // Draw nodes
            graphData.nodes.forEach((node) => {
                const pos = positions.get(node.id);
                if (!pos) return;

                const isLocked = activity.locks.some((l) => l.path === node.path);
                const isMatch = filter.trim() ? node.path.toLowerCase().includes(filter.toLowerCase()) : true;

                ctx.beginPath();
                ctx.arc(pos.x, pos.y, pos.radius, 0, Math.PI * 2);

                if (node.is_folder) {
                    ctx.fillStyle = isMatch ? "#fcc419" : "rgba(252, 196, 25, 0.3)";
                } else if (node.content_type.includes("markdown")) {
                    ctx.fillStyle = isMatch ? "#ff0055" : "rgba(255, 0, 85, 0.3)";
                } else if (node.content_type.includes("python") || node.content_type.includes("javascript")) {
                    ctx.fillStyle = isMatch ? "#5c7cfa" : "rgba(92, 124, 250, 0.3)";
                } else {
                    ctx.fillStyle = isMatch ? "#51cf66" : "rgba(81, 207, 102, 0.3)";
                }
                ctx.fill();

                // Lock radar beacon ring
                if (isLocked) {
                    ctx.lineWidth = 2;
                    ctx.strokeStyle = "#ff6b6b";
                    ctx.stroke();

                    ctx.beginPath();
                    ctx.arc(pos.x, pos.y, pos.radius + 6, 0, Math.PI * 2);
                    ctx.strokeStyle = "rgba(255, 107, 107, 0.5)";
                    ctx.stroke();
                }

                // Label
                ctx.fillStyle = isMatch ? "#ffffff" : "rgba(255, 255, 255, 0.4)";
                ctx.font = "10px monospace";
                ctx.textAlign = "center";
                ctx.fillText(node.label, pos.x, pos.y + pos.radius + 12);
            });

            ctx.restore();
            animId = requestAnimationFrame(render);
        };

        animId = requestAnimationFrame(render);
        return () => cancelAnimationFrame(animId);
    }, [graphData, activity, filter]);

    // Handle resize
    useEffect(() => {
        const resize = () => {
            const canvas = canvasRef.current;
            if (!canvas) return;
            canvas.width = canvas.parentElement?.clientWidth || 800;
            canvas.height = canvas.parentElement?.clientHeight || 600;
        };
        resize();
        window.addEventListener("resize", resize);
        return () => window.removeEventListener("resize", resize);
    }, []);

    // Mouse drag pan & click interactions
    const handleMouseDown = (e: React.MouseEvent<HTMLCanvasElement>) => {
        isDraggingRef.current = true;
        dragStartRef.current = { x: e.clientX, y: e.clientY };
    };

    const handleMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
        if (!isDraggingRef.current) return;
        const dx = e.clientX - dragStartRef.current.x;
        const dy = e.clientY - dragStartRef.current.y;
        dragStartRef.current = { x: e.clientX, y: e.clientY };
        transformRef.current.x += dx;
        transformRef.current.y += dy;
    };

    const handleMouseUp = (e: React.MouseEvent<HTMLCanvasElement>) => {
        isDraggingRef.current = false;

        // Check if a node was clicked
        const canvas = canvasRef.current;
        if (!canvas) return;
        const rect = canvas.getBoundingClientRect();
        const clickX = (e.clientX - rect.left - transformRef.current.x) / transformRef.current.scale;
        const clickY = (e.clientY - rect.top - transformRef.current.y) / transformRef.current.scale;

        let clicked: GraphNode | null = null;
        for (const node of graphData.nodes) {
            const pos = nodePositionsRef.current.get(node.id);
            if (pos) {
                const dist = Math.hypot(pos.x - clickX, pos.y - clickY);
                if (dist <= pos.radius + 4) {
                    clicked = node;
                    break;
                }
            }
        }
        if (clicked) {
            setSelectedNode(clicked);
            onOpen(clicked.path);
        }
    };

    const handleWheel = (e: React.WheelEvent<HTMLCanvasElement>) => {
        e.preventDefault();
        const factor = e.deltaY < 0 ? 1.1 : 0.9;
        transformRef.current.scale = Math.max(0.2, Math.min(3, transformRef.current.scale * factor));
    };

    const handleRestoreVersion = async (version: FileVersionInfo) => {
        setRestoring(true);
        try {
            await send("/api/restore", "POST", {
                path: version.path,
                generation: version.generation,
            });
            onStatus?.(`Restored ${version.path} to generation ${version.generation}`);
            refreshData();
        } catch (err) {
            onStatus?.(`Restore failed: ${(err as Error).message}`, true);
        } finally {
            setRestoring(false);
        }
    };

    const currentVersion = activity.recent_versions[timelineIndex] || null;

    return (
        <Box style={{ position: "relative", width: "100%", height: "100%", overflow: "hidden", background: "#0c0d0e" }}>
            {/* Interactive Canvas Graph */}
            <canvas
                ref={canvasRef}
                onMouseDown={handleMouseDown}
                onMouseMove={handleMouseMove}
                onMouseUp={handleMouseUp}
                onWheel={handleWheel}
                style={{ width: "100%", height: "100%", cursor: "grab" }}
            />

            {/* Top Toolbar overlay */}
            <Group
                style={{
                    position: "absolute",
                    top: 16,
                    left: 16,
                    right: 16,
                    pointerEvents: "none",
                }}
                justify="space-between"
            >
                <Group style={{ pointerEvents: "auto" }} gap="xs">
                    <TextInput
                        placeholder="Filter nodes..."
                        size="xs"
                        value={filter}
                        onChange={(e) => setFilter(e.currentTarget.value)}
                        style={{ width: 220 }}
                    />
                    <Badge variant="filled" color="dark" size="sm">
                        {graphData.nodes.length} nodes
                    </Badge>
                    <Badge variant="filled" color="dark" size="sm">
                        {graphData.edges.length} edges
                    </Badge>
                    {selectedNode && (
                        <Badge variant="light" color="cyan" size="sm">
                            Selected: {selectedNode.path}
                        </Badge>
                    )}
                </Group>

                {/* Agent Presence Radar Overlay */}
                <Card
                    withBorder
                    padding="xs"
                    radius="md"
                    style={{
                        pointerEvents: "auto",
                        background: "rgba(20, 21, 23, 0.85)",
                        backdropFilter: "blur(8px)",
                        maxWidth: 320,
                    }}
                >
                    <Group gap="xs" mb={4}>
                        <Box
                            style={{
                                width: 8,
                                height: 8,
                                borderRadius: "50%",
                                background: activity.locks.length > 0 ? "#ff6b6b" : "#51cf66",
                                boxShadow: activity.locks.length > 0 ? "0 0 8px #ff6b6b" : "none",
                            }}
                        />
                        <Text size="xs" fw={700}>
                            Agent Presence Radar
                        </Text>
                        <Badge size="xs" variant="light" color={activity.locks.length > 0 ? "red" : "gray"}>
                            {activity.locks.length} active
                        </Badge>
                    </Group>

                    {activity.locks.length === 0 ? (
                        <Text size="xs" c="dimmed">
                            No agent leases active
                        </Text>
                    ) : (
                        <Stack gap={4}>
                            {activity.locks.slice(0, 3).map((l: FileLockInfo, idx: number) => (
                                <Group key={idx} justify="space-between" gap="xs">
                                    <Text size="xs" ff="monospace" style={{ maxWidth: 160, overflow: "hidden", textOverflow: "ellipsis" }}>
                                        {l.path}
                                    </Text>
                                    <Badge size="xs" color="red">
                                        {l.holder}
                                    </Badge>
                                </Group>
                            ))}
                        </Stack>
                    )}
                </Card>
            </Group>

            {/* Bottom Timeline Playback Scrubber */}
            {activity.recent_versions.length > 0 && (
                <Paper
                    withBorder
                    p="xs"
                    radius="md"
                    style={{
                        position: "absolute",
                        bottom: 16,
                        left: 16,
                        right: 16,
                        background: "rgba(20, 21, 23, 0.9)",
                        backdropFilter: "blur(8px)",
                    }}
                >
                    <Group justify="space-between" mb={8}>
                        <Group gap="xs">
                            <Text size="xs" fw={700}>
                                ⏱ Timeline Playback Scrubber
                            </Text>
                            {currentVersion && (
                                <>
                                    <Badge size="xs" color="blue">
                                        Gen {currentVersion.generation}
                                    </Badge>
                                    <Text size="xs" ff="monospace" c="dimmed">
                                        {currentVersion.path}
                                    </Text>
                                    <Text size="xs" c="dimmed">
                                        by {currentVersion.author} ({currentVersion.op})
                                    </Text>
                                </>
                            )}
                        </Group>

                        {currentVersion && (
                            <Tooltip label="Restore this version">
                                <ActionIcon
                                    size="sm"
                                    color="surreal"
                                    variant="light"
                                    loading={restoring}
                                    onClick={() => handleRestoreVersion(currentVersion)}
                                >
                                    ↺
                                </ActionIcon>
                            </Tooltip>
                        )}
                    </Group>

                    <Slider
                        size="xs"
                        min={0}
                        max={activity.recent_versions.length - 1}
                        value={timelineIndex}
                        onChange={setTimelineIndex}
                        label={(val) => `Version ${val + 1} of ${activity.recent_versions.length}`}
                    />
                </Paper>
            )}
        </Box>
    );
}
