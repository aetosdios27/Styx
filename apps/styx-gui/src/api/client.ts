import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { AppEvent, AppSnapshot, CommandResponse, emptySnapshot, InfoHashHex } from "./types";

function hasTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

function requireTauriRuntime(): void {
  if (!hasTauriRuntime()) {
    throw new Error("Styx desktop IPC is available only inside the Tauri app window.");
  }
}

export async function getSnapshot(): Promise<AppSnapshot> {
  if (!hasTauriRuntime()) {
    return {
      ...emptySnapshot(),
      logs: [
        {
          level: "warn",
          message: "Browser preview is not connected to the Styx desktop backend. Launch with `bun run app:dev`."
        }
      ]
    };
  }

  return invoke<AppSnapshot>("get_snapshot");
}

export async function addTorrent(source: string, destination: string | null): Promise<CommandResponse> {
  requireTauriRuntime();
  return invoke<CommandResponse>("add_torrent", { source, destination });
}

export async function removeTorrent(infoHash: InfoHashHex): Promise<CommandResponse> {
  requireTauriRuntime();
  return invoke<CommandResponse>("remove_torrent", { infoHash });
}

export async function pauseTorrent(infoHash: InfoHashHex): Promise<CommandResponse> {
  requireTauriRuntime();
  return invoke<CommandResponse>("pause_torrent", { infoHash });
}

export async function resumeTorrent(infoHash: InfoHashHex): Promise<CommandResponse> {
  requireTauriRuntime();
  return invoke<CommandResponse>("resume_torrent", { infoHash });
}

export async function subscribeToEvents(onEvent: (event: AppEvent) => void): Promise<() => void> {
  if (!hasTauriRuntime()) {
    return () => {};
  }

  const unlisten = await listen<AppEvent>("styx://event", (event) => onEvent(event.payload));
  return unlisten;
}
