import { defineConfig } from "vite";
import { fileURLToPath } from "node:url";
// @ts-ignore -- vite-plugin-fable ships no type declarations
import fable from "vite-plugin-fable";

// The Fable daemon needs an absolute path to the F# project.
const fsproj = fileURLToPath(new URL("./src/App.fsproj", import.meta.url));

// Tauri 2's CLI exports TAURI_ENV_* (v1 used bare TAURI_*). Reading the old
// names here would silently yield undefined rather than error, so these must
// stay in sync with the envPrefix below.
const platform = process.env.TAURI_ENV_PLATFORM;
const isDebug = !!process.env.TAURI_ENV_DEBUG;

// https://vitejs.dev/config/
export default defineConfig(async () => ({
  plugins: [fable({ fsproj })],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  // prevent vite from obscuring rust errors
  clearScreen: false,
  // tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // .NET writes locked files under obj/bin during design-time builds,
      // which crashes vite's watcher on Windows (EBUSY)
      ignored: ["**/src-tauri/**", "**/src/obj/**", "**/src/bin/**", "**/fable_modules/**"],
    },
  },
  // to make use of `TAURI_ENV_DEBUG` and other env variables
  // https://v2.tauri.app/reference/environment-variables/
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // Tauri 2 targets Edge WebView2 on Windows and WebKit elsewhere
    target: platform === "windows" ? "chrome105" : "safari13",
    // don't minify for debug builds
    minify: isDebug ? false : "esbuild",
    // produce sourcemaps for debug builds
    sourcemap: isDebug,
  },
}));
