<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open } from "@tauri-apps/plugin-dialog";
  import { showError } from "../common/ErrorToast.svelte";
  import { errMsg } from "../../lib/errors";
  import { t } from "../../lib/t";

  export interface AudioTrack {
    index: number;
    codec: string;
    lang: string | null;
    channels: number;
    default: boolean;
  }

  export interface SubtitleStream {
    index: number;
    codec: string;
    lang: string | null;
    default: boolean;
  }

  export interface VideoInspection {
    path: string;
    file_name: string;
    duration_secs: number;
    audio_tracks: AudioTrack[];
    subtitle_streams: SubtitleStream[];
  }

  const VIDEO_EXT = ["mp4", "mkv", "avi", "mov", "webm", "m4v", "ts", "flv", "wmv", "mpg", "mpeg"];

  let { onInspect }: { onInspect: (inspection: VideoInspection) => void } = $props();

  let dragOver = $state(false);
  let busy = $state(false);
  let error = $state<string | null>(null);
  let unlisten: (() => void) | null = null;

  onMount(() => {
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (event.payload.type === "over") {
          dragOver = true;
          return;
        }
        if (event.payload.type === "leave") {
          dragOver = false;
          return;
        }
        if (event.payload.type === "drop") {
          dragOver = false;
          void handlePath(event.payload.paths[0]);
        }
      })
      .then((u) => (unlisten = u));
    return () => unlisten?.();
  });

  function isVideo(path: string): boolean {
    const ext = path.split(".").pop()?.toLowerCase() ?? "";
    return VIDEO_EXT.includes(ext);
  }

  async function handlePath(path?: string): Promise<void> {
    if (!path) return;
    if (!isVideo(path)) {
      error = t("import.errFormat");
      return;
    }
    busy = true;
    error = null;
    try {
      const inspection = await invoke<VideoInspection>("inspect_video", { path });
      onInspect(inspection);
    } catch (e) {
      error = errMsg(e, "import.errInspect");
      showError(e);
    } finally {
      busy = false;
    }
  }

  async function pickFile(): Promise<void> {
    const selected = await open({
      multiple: false,
      filters: [{ name: t("import.fileFilter"), extensions: [...VIDEO_EXT] }],
    });
    if (typeof selected === "string") await handlePath(selected);
  }
</script>

<div
  class="dropzone"
  class:dragover={dragOver}
  role="button"
  tabindex="0"
  onkeydown={(e) => {
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      void pickFile();
    }
  }}
  onclick={() => void pickFile()}
>
  <p class="hint">
    {#if busy}
      {t("import.inspecting")}
    {:else}
      {t("import.dropHint")}
    {/if}
  </p>
  {#if error}
    <p class="error" role="alert">{error}</p>
  {/if}
</div>

<style>
  .dropzone {
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    padding: 40px;
    border: 2px dashed var(--border);
    border-radius: var(--radius);
    color: var(--text-muted);
    cursor: pointer;
    text-align: center;
    transition:
      border-color 0.15s,
      background 0.15s;
  }

  .dropzone:hover,
  .dropzone:focus-visible {
    border-color: var(--accent);
  }

  .dropzone.dragover {
    border-color: var(--accent);
    background: color-mix(in srgb, var(--accent) 8%, transparent);
  }

  .hint {
    margin: 0;
  }

  .error {
    color: var(--danger);
    margin: 0;
  }
</style>
