/**
 * The typed boundary between the React frontend and the Rust backend.
 *
 * Every Tauri command is wrapped exactly once, here. Components call these functions and
 * never `invoke` directly, so the surface stays enumerable and mockable in tests.
 *
 * The frontend holds no scheduling rules and no persistence logic: it asks the backend
 * whether something is legal, and renders the answer.
 */
import { invoke } from "@tauri-apps/api/core";
import { isAppInfo, type AppInfo } from "@evara/domain";

/** Reads build and version facts from the backend. */
export async function appInfo(): Promise<AppInfo> {
  const value = await invoke("app_info");
  if (!isAppInfo(value)) {
    throw new Error(`app_info returned an unexpected payload: ${JSON.stringify(value)}`);
  }
  return value;
}
