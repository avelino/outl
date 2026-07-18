/* @refresh reload */
import { render } from "solid-js/web";
import { attachConsole } from "@tauri-apps/plugin-log";
import App from "./App";
import "./styles.css";

// Forward Rust logs (the `Webview` target of tauri-plugin-log) into the browser
// console, so backend `log::` lines show up in Safari Web Inspector alongside
// the frontend's own `console.*`. Best-effort: outside a Tauri webview it no-ops.
void attachConsole().catch(() => {});

render(() => <App />, document.getElementById("root") as HTMLElement);
