<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";

  let dropMessage = $state("Drop a ZIP file here to start.");
  let statusMessage = $state("");
  let isDragOver = $state(false);
  let currentStep = $state<"Unzipping" | "ConvertingImages" | "Recompressing" | "Completed" | "Error" | "">("");
  let outputPath = $state("");
  let isProcessing = $state(false);

  const unlisten_functions: Array<() => void> = [];

  interface ProgressPayload {
    step: "Unzipping" | "ConvertingImages" | "Recompressing" | "Completed" | "Error";
    status: string;
    output_path?: string;
  }

  onMount(async () => {
    unlisten_functions.push(
      await listen<void>("tauri://drag-enter", () => {
        if (!isProcessing) isDragOver = true;
      })
    );

    unlisten_functions.push(
      await listen<void>("tauri://drag-leave", () => {
        isDragOver = false;
      })
    );

    unlisten_functions.push(
      await listen<string[]>("tauri://drag-drop", (event) => {
        isDragOver = false;
        if (isProcessing) return;
        const files = event.payload.paths;
        if (files.length === 1 && files[0].toLowerCase().endsWith('.zip')) {
          handle_unzip(files[0]);
        } else {
          statusMessage = "Please drop a single ZIP file.";
        }
      })
    );

    unlisten_functions.push(
      await listen<ProgressPayload>("progress_update", (event) => {
        currentStep = event.payload.step;
        statusMessage = event.payload.status;
        if (event.payload.output_path) {
          outputPath = event.payload.output_path;
        }
        if (event.payload.step === "Completed" || event.payload.step === "Error") {
          isProcessing = false;
        }
      })
    );
  });

  onDestroy(() => {
    unlisten_functions.forEach(unlisten => unlisten());
  });

  async function handle_unzip(filePath: string) {
    isProcessing = true;
    statusMessage = `Processing ${filePath}...`;
    currentStep = "";
    outputPath = "";
    invoke("unzip_file", { zipPath: filePath }).catch((error) => {
      // This catches errors from the Rust command itself (e.g., command not found)
      console.error("Invoke error:", error);
      statusMessage = `Error: ${error}`;
      currentStep = "Error";
      isProcessing = false;
    });
  }
</script>

<div class="p-4 h-full w-full">
<div
  class="flex flex-col justify-center items-center h-full box-border
    outline outline-3 outline-dashed outline-gray-400 outline-offset-[-3px]
    transition-colors duration-300 ease-in-out p-8
    dark:outline-gray-500"
  class:outline-blue-500={isDragOver && !isProcessing}
  class:dark:outline-cyan-500={isDragOver && !isProcessing}
>
  <div class="w-full h-full text-center flex flex-col">
    <h1 class="text-2xl mb-6">GNB-ZipImageCompressor</h1>

    <div class="flex items-center justify-center mb-6 gap-2">
      <label for="output-path" class="text-lg">出力フォルダ：</label>
      <input
        id="output-path"
        type="text"
        readonly
        value={outputPath}
        class="flex-grow border border-gray-300 rounded p-2 bg-white text-gray-800
          dark:border-gray-600 dark:bg-gray-700 dark:text-gray-200"
      />
    </div>

    <div class="flex flex-col text-left gap-2 mb-6">
      <div class="">
        <input
          type="checkbox"
          id="unzip"
          checked={currentStep === 'Unzipping' || currentStep === 'ConvertingImages' || currentStep === 'Recompressing' || currentStep === 'Completed'}
          disabled
          class="w-5 h-5 accent-blue-500 dark:accent-cyan-500"
        />
        <label for="unzip">解凍</label>
      </div>
      <div class="">
        <input
          type="checkbox"
          id="convert"
          checked={currentStep === 'ConvertingImages' || currentStep === 'Recompressing' || currentStep === 'Completed'}
          disabled
          class="w-5 h-5 accent-blue-500 dark:accent-cyan-500"
        />
        <label for="convert">変換</label>
      </div>
      <div class="">
        <input
          type="checkbox"
          id="compress"
          checked={currentStep === 'Recompressing' || currentStep === 'Completed'}
          disabled
          class="w-5 h-5 accent-blue-500 dark:accent-cyan-500"
        />
        <label for="compress">圧縮</label>
      </div>
    </div>

    <div
      class="grow border-2 border-dashed rounded-md p-4 min-h-[60px] mb-6
        flex justify-center items-center"
    >
      <p class="m-0 break-all">{statusMessage}</p>
    </div>

    {#if !isProcessing}
      <p class="text-lg">
        {dropMessage}
      </p>
    {/if}
  </div>
</div>
</div>