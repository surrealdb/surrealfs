import react from "@vitejs/plugin-react";
import { type Plugin, defineConfig } from "vite";

// The Python server serves ../static as the whole page: index.html at `/` and
// everything else under `/assets`. In dev, vite serves the page and forwards the
// API to `just browser` on 7933, so one server owns the data either way.
const API = "http://127.0.0.1:7933";

/**
 * Drop the assets nothing links to.
 *
 * `@surrealdb/ui` imports ~800 picto and brand images at the top of its entry.
 * Tree-shaking drops the bindings, but vite emits an asset when it *loads* the
 * module, which happens first -- so the build lands 31 MB of images for the one
 * this page actually uses. Chunks and stylesheets name every asset they need, so
 * anything unnamed once the bundle is generated is dead weight.
 */
function pruneUnreferencedAssets(): Plugin {
    return {
        name: "prune-unreferenced-assets",
        generateBundle(_options, bundle) {
            const referenced = Object.values(bundle)
                .map((chunk) => (chunk.type === "chunk" ? chunk.code : String(chunk.source)))
                .join("\n");
            for (const [key, chunk] of Object.entries(bundle)) {
                const keep =
                    chunk.type === "chunk" ||
                    /\.(css|html)$/.test(chunk.fileName) ||
                    referenced.includes(chunk.fileName);
                if (!keep) delete bundle[key];
            }
        },
    };
}

export default defineConfig({
    plugins: [react(), pruneUnreferencedAssets()],
    build: { outDir: "../static", emptyOutDir: true, chunkSizeWarningLimit: 2048 },
    server: {
        proxy: {
            "/api": { target: API, changeOrigin: true },
            "/raw": { target: API, changeOrigin: true },
        },
    },
});
