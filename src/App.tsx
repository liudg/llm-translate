import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";

type EffectInfo = { osBuild: number; effect: string };

export default function App() {
  const [info, setInfo] = useState<EffectInfo | null>(null);

  useEffect(() => {
    invoke<EffectInfo>("window_effect_info").then(setInfo).catch(console.error);
  }, []);

  return (
    <main className="flex h-full flex-col gap-4 p-6 text-neutral-900 dark:text-neutral-100">
      <h1 className="text-2xl font-semibold">LLM Translate</h1>
      <p className="text-sm opacity-70">M0 骨架：窗口特效验证</p>
      <div className="rounded-xl bg-white/50 p-4 text-sm shadow-sm backdrop-blur dark:bg-white/5">
        {info ? (
          <>
            <div>系统 build：{info.osBuild}</div>
            <div>窗口特效：{info.effect}</div>
          </>
        ) : (
          "读取中…"
        )}
      </div>
    </main>
  );
}
