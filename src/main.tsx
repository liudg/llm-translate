import React from "react";
import ReactDOM from "react-dom/client";
import { getCurrentWindow } from "@tauri-apps/api/window";
import Main from "./windows/main/Main";
import Mini from "./windows/mini/Mini";
import Wizard from "./windows/wizard/Wizard";
import { startSettingsSync } from "./stores/settings";
import "./index.css";

// One bundle for all windows; the window label picks the view.
const label = getCurrentWindow().label;
const View = label === "wizard" ? Wizard : label === "mini" ? Mini : Main;

startSettingsSync();

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <View />
  </React.StrictMode>,
);
