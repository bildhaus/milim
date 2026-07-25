import { readFileSync } from "node:fs";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

const favicon = new URL("../desktop/src-tauri/icons/icon.png", import.meta.url);

export default defineConfig({
  plugins: [
    react(),
    {
      name: "milim-favicon",
      configureServer(server) {
        server.middlewares.use("/assets/icon.png", (_request, response) => {
          response.setHeader("Content-Type", "image/png");
          response.end(readFileSync(favicon));
        });
      },
      generateBundle() {
        this.emitFile({
          type: "asset",
          fileName: "assets/icon.png",
          source: readFileSync(favicon),
        });
      },
    },
  ],
});
