import "@surrealdb/ui/styles.css";
import "@surrealdb/ui/fonts.css";
import "@mantine/core/styles.css";
import "./app.css";

import { MantineProvider } from "@mantine/core";
import { ConfirmationProvider, MANTINE_THEME } from "@surrealdb/ui";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";

createRoot(document.getElementById("root") as HTMLElement).render(
    <StrictMode>
        {/* Dark only: the kit is dark-first and the browser always was. */}
        <MantineProvider theme={MANTINE_THEME} forceColorScheme="dark">
            <ConfirmationProvider>
                <App />
            </ConfirmationProvider>
        </MantineProvider>
    </StrictMode>,
);
