import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Main from "./windows/main/Main";
import Mini from "./windows/mini/Mini";
import Popup from "./windows/popup/Popup";
import Shot from "./windows/shot/Shot";
import Wizard from "./windows/wizard/Wizard";
import { startSettingsSync } from "./stores/settings";
import "./index.css";

// One bundle for all windows; the window label picks the view.
const label = getCurrentWindow().label;
const VIEWS = { wizard: Wizard, mini: Mini, popup: Popup, shot: Shot } as const;
const View = VIEWS[label as keyof typeof VIEWS] ?? Main;

startSettingsSync();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <View />
  </React.StrictMode>,
);
