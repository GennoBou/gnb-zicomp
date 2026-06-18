<script lang="ts">
  import { onMount, onDestroy } from "svelte";
  import { listen } from "@tauri-apps/api/event";
  import { invoke } from "@tauri-apps/api/core";

  let dropMessage = $state("Drop ZIP file(s) here to start.");
  let logMessages = $state<{ type: "ERR" | "INF" | "DIR" | "LOG" | "SUCCESS", message: string, ratio?: number }[]>([]);
  let isDragOver = $state(false);
  let currentStep = $state<"Unzipping" | "ConvertingImages" | "Recompressing" | "Completed" | "Error" | "AllCompleted" | "">("");
  let isProcessing = $state(false);

  const unlisten_functions: Array<() => void> = [];

  interface ProgressPayload {
    step: "Unzipping" | "ConvertingImages" | "Recompressing" | "Completed" | "Error" | "AllCompleted" | "StartingFile";
    status: string;
    output_path?: string;
    original_size?: number;
    new_size?: number;
  }

  async function initializeLogs(initialOutputPath: string = "") {
    logMessages = [{ type: "DIR", message: `出力フォルダ：${initialOutputPath}` }];
  }

  onMount(async () => {
    let initialOutputPath = "";
    try {
      initialOutputPath = await invoke("get_output_directory");
    } catch (e) {
      console.error("Failed to get initial output directory:", e);
      initialOutputPath = "取得できませんでした"; // エラー時の表示
    }
    initializeLogs(initialOutputPath);

    // Listen for file drop from command line arguments
    unlisten_functions.push(
      await listen<string[]>("file-drop-from-args", (event) => {
        const filePaths = event.payload;
        if (filePaths && filePaths.length > 0) {
          const zipFiles = filePaths.filter(f => f.toLowerCase().endsWith('.zip'));
          if (zipFiles.length > 0) {
            setTimeout(() => {
              handle_unzip(zipFiles);
            }, 500);
          }
        }
      })
    );

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
        const zipFiles = files.filter(f => f.toLowerCase().endsWith('.zip'));
        if (zipFiles.length > 0) {
          handle_unzip(zipFiles);
        } else {
          initializeLogs(); // エラー時もログを初期化
          logMessages.push({ type: "LOG", message: "Please drop at least one ZIP file." });
        }
      })
    );

    unlisten_functions.push(
      await listen<ProgressPayload>("progress_update", (event) => {
        const payload = event.payload;
        if (payload.step !== "StartingFile") {
          currentStep = payload.step;
        }
        
        // ログの簡略化：特定のステップの細かなステータスは表示せず、
        // 完了、エラー、または出力パス情報のみをログに追加する
        if (payload.step === "Completed") {
          let ratio: number | undefined = undefined;
          if (payload.original_size && payload.new_size && payload.original_size > 0) {
            ratio = ((payload.original_size - payload.new_size) / payload.original_size) * 100;
          }
          logMessages.push({ type: "SUCCESS", message: payload.status, ratio });
        } else if (payload.step === "Error") {
          logMessages.push({ type: "ERR", message: payload.status });
        } else if (payload.step === "AllCompleted") {
          logMessages.push({ type: "SUCCESS", message: payload.status });
          isProcessing = false;
        } else if (payload.step === "StartingFile") {
          logMessages.push({ type: "LOG", message: payload.status });
        } else if (payload.output_path) {
          if (logMessages.length > 0 && logMessages[0].type === "DIR") {
            logMessages[0].message = `出力フォルダ：${payload.output_path}`;
          } else {
            logMessages.unshift({ type: "DIR", message: `出力フォルダ：${payload.output_path}` });
          }
        }
      })
    );
  });

  onDestroy(() => {
    unlisten_functions.forEach(unlisten => unlisten());
  });

  async function handle_unzip(filePaths: string[]) {
    isProcessing = true;
    let currentOutputPath = "";
    try {
      currentOutputPath = await invoke("get_output_directory");
    } catch (e) {
      console.error("Failed to get current output directory:", e);
      currentOutputPath = "取得できませんでした";
    }
    initializeLogs(currentOutputPath);
    currentStep = "";
    invoke("unzip_file", { zipPaths: filePaths }).catch((error) => {
      console.error("Invoke error:", error);
      logMessages.push({ type: "ERR", message: `Error starting process: ${error}` });
      currentStep = "Error";
      isProcessing = false;
    });
  }
</script>

<div class="h-screen w-screen bg-slate-50 dark:bg-slate-950 p-4 transition-colors duration-500 overflow-hidden flex flex-col gap-4">
  <!-- Header Area: 2 Columns -->
  <header class="grid grid-cols-1 md:grid-cols-2 gap-4 h-auto shrink-0">
    <!-- Left: Title & Steps -->
    <div class="bg-white dark:bg-slate-900 p-6 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-800 flex flex-col justify-between">
      <div>
        <h1 class="text-2xl font-bold text-slate-800 dark:text-slate-100 mb-1">
          GNB ZipImageCompressor
        </h1>
        <p class="text-sm text-slate-500 dark:text-slate-400 mb-6">ZIP内の画像をWebPに変換して再圧縮</p>
      </div>

      <div class="flex flex-row items-center gap-4">
        {#each [
          { label: '解凍', id: 'unzip', active: currentStep === 'Unzipping' || currentStep === 'ConvertingImages' || currentStep === 'Recompressing' || currentStep === 'Completed' },
          { label: '変換', id: 'convert', active: currentStep === 'ConvertingImages' || currentStep === 'Recompressing' || currentStep === 'Completed' },
          { label: '圧縮', id: 'compress', active: currentStep === 'Recompressing' || currentStep === 'Completed' }
        ] as step}
          <div class="flex items-center gap-2">
            <div class="relative flex items-center justify-center">
              <div class="w-5 h-5 rounded-full border-2 transition-all duration-300
                {step.active ? 'bg-blue-500 border-blue-500 dark:bg-blue-400 dark:border-blue-400' : 'border-slate-300 dark:border-slate-600'}">
                {#if step.active}
                  <svg class="w-3.5 h-3.5 text-white" fill="none" viewBox="0 0 24 24" stroke="currentColor" stroke-width="4">
                    <path stroke-linecap="round" stroke-linejoin="round" d="M5 13l4 4L19 7" />
                  </svg>
                {/if}
              </div>
            </div>
            <span class="text-xs font-medium {step.active ? 'text-blue-600 dark:text-blue-400' : 'text-slate-400'}">
              {step.label}
            </span>
          </div>
        {/each}
      </div>
    </div>

    <!-- Right: Drag & Drop Area (Compact) -->
    <div class="h-full min-h-[140px]">
      <div
        class="flex flex-col justify-center items-center rounded-2xl border-2 border-dashed
          transition-all duration-300 ease-in-out p-4
          {isDragOver && !isProcessing 
            ? 'fixed inset-4 z-50 border-blue-500 bg-blue-50/80 backdrop-blur-sm dark:border-blue-400 dark:bg-blue-900/40 shadow-2xl' 
            : 'relative h-full border-slate-300 dark:border-slate-700 bg-white dark:bg-slate-900 shadow-sm'}"
      >
        {#if !isProcessing}
          <div class="flex flex-col items-center {isDragOver ? 'animate-bounce text-blue-500 scale-125' : 'text-slate-400'} transition-transform duration-300">
            <svg class="w-8 h-8 mb-2" fill="none" viewBox="0 0 24 24" stroke="currentColor">
              <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M15 13l-3-3m0 0l-3 3m3-3v12" />
            </svg>
            <p class="text-sm font-medium text-center {isDragOver ? 'text-blue-600 dark:text-blue-400' : 'text-slate-500 dark:text-slate-400'}">
              {isDragOver ? 'Drop to Start!' : dropMessage}
            </p>
          </div>
        {:else}
          <div class="flex flex-col items-center text-blue-500">
            <div class="w-6 h-6 border-3 border-blue-500 border-t-transparent rounded-full animate-spin mb-2"></div>
            <p class="text-xs font-medium animate-pulse">Processing...</p>
          </div>
        {/if}
      </div>
    </div>
  </header>

  <!-- Bottom Area: Log Console -->
  <main class="grow flex flex-col bg-white dark:bg-slate-900 rounded-2xl shadow-sm border border-slate-200 dark:border-slate-800 overflow-hidden">
    <div class="px-4 py-2 border-b border-slate-100 dark:border-slate-800 bg-slate-50/50 dark:bg-slate-950/50 flex justify-between items-center">
      <span class="text-xs font-bold text-slate-400 uppercase tracking-wider">Log Console</span>
      <div class="flex gap-1">
        <div class="w-2 h-2 rounded-full bg-slate-200 dark:bg-slate-700"></div>
        <div class="w-2 h-2 rounded-full bg-slate-200 dark:bg-slate-700"></div>
        <div class="w-2 h-2 rounded-full bg-slate-200 dark:bg-slate-700"></div>
      </div>
    </div>
    <div
      class="grow p-4 overflow-y-auto scrollbar-thin scrollbar-thumb-slate-200 dark:scrollbar-thumb-slate-800"
    >
      {#each logMessages as log}
        <div class="flex gap-3 py-1 animate-in fade-in slide-in-from-left-1 duration-200 font-mono text-xs items-center">
          {#if log.type === "ERR"}
            <span class="text-red-500 font-bold shrink-0">ERR</span>
            <p class="m-0 break-all text-red-600 dark:text-red-400">{log.message}</p>
          {:else if log.type === "SUCCESS"}
            <span class="text-green-500 font-bold shrink-0">INF</span>
            <p class="m-0 break-all text-green-600 dark:text-green-400">{log.message}</p>
            {#if log.ratio !== undefined}
              <span class="ml-auto px-2 py-0.5 rounded-full text-[10px] font-bold {log.ratio >= 0 ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400' : 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'}">
                {log.ratio >= 0 ? '+' : '-'}{Math.abs(log.ratio).toFixed(1)}% {log.ratio >= 0 ? 'smaller' : 'larger'}
              </span>
            {/if}
          {:else if log.type === "DIR"}
            <span class="text-blue-500 font-bold shrink-0">DIR</span>
            <p class="m-0 break-all text-blue-600 dark:text-blue-400">{log.message}</p>
          {:else}
            <span class="text-slate-300 dark:text-slate-600 font-bold shrink-0">LOG</span>
            <p class="m-0 break-all text-slate-500 dark:text-slate-400">{log.message}</p>
          {/if}
        </div>
      {/each}
    </div>
  </main>
</div>