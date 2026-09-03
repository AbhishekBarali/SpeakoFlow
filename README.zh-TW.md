<div align="center">

[English](README.md) · **繁體中文**

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final-v2/png/lockup-dark-h256.png" />
  <img src="Logo/final-v2/png/lockup-h256.png" alt="SpeakoFlow" width="340" />
</picture>

# SpeakoFlow：適用於 Windows、macOS 與 Linux 的免費開源語音輸入與 AI 助理

<p><strong>你的思考速度，比打字更快。</strong></p>

**免費、在本機執行的桌面語音助理。語音輸入、寫作與 AI 助理，都能用聲音完成。**

[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational)](#安裝)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)

<img src="assets/darko.gif" alt="SpeakoFlow 即時語音輸入示範" width="720" />

### 下載

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?logo=windows&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?logo=apple&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?logo=linux&logoColor=black&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)

[所有版本](https://github.com/AbhishekBarali/SpeakoFlow/releases) &nbsp;·&nbsp; [官方網站](https://www.speakoflow.com) &nbsp;·&nbsp; [使用文件](https://www.speakoflow.com/docs)

</div>

> **接收新版本通知：** 點選本頁上方的 **Watch → Custom → Releases**。

---

## 目錄

- [SpeakoFlow 是什麼？](#speakoflow-是什麼)
- [中文語音輸入](#中文語音輸入)
- [為什麼選擇 SpeakoFlow](#為什麼選擇-speakoflow)
- [功能](#功能)
- [預設快速鍵](#預設快速鍵)
- [安裝](#安裝)
- [從原始碼建置](#從原始碼建置)
- [技術架構](#技術架構)
- [隱私](#隱私)
- [疑難排解](#疑難排解)
- [開發計畫](#開發計畫)
- [參與貢獻](#參與貢獻)
- [授權](#授權)
- [致謝](#致謝)

## SpeakoFlow 是什麼？

SpeakoFlow 能在你工作的地方直接把語音變成文字。按下快速鍵並開始說話，內容就會輸入到目前使用的應用程式。以「Hey Flow」開頭，SpeakoFlow 可以把口述需求寫成完整回覆或電子郵件。你也可以開啟浮動助理面板，以語音對話並聽取回答。

語音轉文字在你的電腦上執行，因此錄音不會離開裝置。AI 助理可以使用你選擇的任何模型，包括完全離線的內建模型、自架的本機伺服器，或使用自有金鑰的雲端服務。哪些資料留在電腦上，由你決定。

我在獨自準備考試時做了這個工具。當時我付費使用的語音輸入軟體只能把語音打成文字，聽得見我說話，卻無法進一步幫忙。

## 中文語音輸入

SpeakoFlow 已內建繁體中文與簡體中文介面，也支援中文語音轉文字。語音辨識在本機執行，不需要上傳錄音或建立帳號。

你可以依需求選擇模型：

- **NVIDIA Nemotron Streaming 3.5：** 0.6B 參數的即時串流模型，支援中文在內的 28 種語言。說話時就能看到文字逐步出現。
- **SenseVoice：** 體積小、速度快，支援中文、粵語、英語、日語與韓語。
- **Breeze ASR：** 針對臺灣華語調校，也能辨識中英夾雜的口述內容。
- **Whisper：** Small、Medium、Turbo 與 Large 等多語言模型都能辨識中文，也可將其他語言翻譯成英文。

在語言選單中選擇「繁體中文」或「簡體中文」後，SpeakoFlow 會透過 OpenCC 將輸出統一轉成所選的繁體或簡體，不必手動轉換。

> **AI 清理功能的語言限制：** 中文語音輸入本身可正常使用，但內建的 SpeakoFlow Mini 清理模型目前只支援英語。中文使用者可以關閉 AI 清理，或改用自己信任且支援中文的本機或雲端模型。這項限制不影響中文語音辨識。

## 為什麼選擇 SpeakoFlow

大多數語音輸入工具只負責打字。Wispr Flow、Superwhisper 與 Handy 都能把語音轉成文字，但無法查看你正在處理的內容，再替你寫好回覆。

SpeakoFlow 同時提供語音輸入與螢幕感知助理。在這四款工具中，它也是唯一免費、開源，並同時支援三大桌面平台的選擇。

- **與 Wispr Flow 相比：** Wispr Flow 是閉源軟體，語音轉錄在雲端處理，沒有 Linux 版本，免費方案每週上限為 2,000 字，之後每月 15 美元。SpeakoFlow 採用 MIT 授權，轉錄在你的電腦上完成，而且沒有字數上限。完整比較請見 [SpeakoFlow 與 Wispr Flow](https://www.speakoflow.com/blog/speakoflow-vs-wispr-flow)。
- **與 Superwhisper 相比：** Superwhisper 是一款支援 macOS、Windows 與 iOS 的閉源軟體，Pro 方案每月 8.49 美元，買斷價為 249.99 美元。SpeakoFlow 免費、採用 MIT 授權，而且也支援 Linux。
- **以 Handy 為基礎：** SpeakoFlow 的語音輸入核心來自 [Handy](https://github.com/cjpais/Handy)。Handy 是成熟且好用的純語音輸入工具。SpeakoFlow 在這個基礎上加入語音指令寫作、本機翻譯、文字轉語音、個人記憶與螢幕感知助理。

如果你只需要語音輸入，Handy 是很好的選擇。如果你希望電腦也能回答問題，請繼續閱讀。你也可以參考[最佳免費開源 Wispr Flow 替代方案](https://www.speakoflow.com/blog/best-free-open-source-wispr-flow-alternatives)。

## 功能

### Generate with Flow：說「Hey Flow」，讓它替你寫好內容

以「Hey Flow」開始口述，SpeakoFlow 會依照你的需求寫作，而不是逐字轉錄。說明你想要的電子郵件、回覆或草稿，它會產生完整文字，並貼到游標所在的位置。觸發詞可以重新命名，也能在任何可輸入文字的應用程式中使用。這是一般語音輸入工具做不到的部分。

### 螢幕視覺：詢問畫面上的內容

你可以針對目前正在查看的內容提問，助理會參考畫面回答，例如終端機中的錯誤、瀏覽器裡的合約，或試算表中的圖表。搭配 Generate with Flow，它能根據螢幕內容寫回覆，不必只依賴口述。SpeakoFlow 只會在你提出要求時擷取畫面，畫面只會傳送到你選擇的模型服務，並且只在本機保留小型縮圖。

### 語音輸入：用聲音在任何應用程式打字

按下快速鍵並開始說話。你可以在說話時即時看到文字，也可以在說完後一次輸出。語音辨識透過 whisper.cpp、Nemotron、SenseVoice 等模型在 GPU 或 CPU 上完全離線執行。

### 助理面板：浮在工作畫面上的語音對話

使用快速鍵開啟永遠置頂的浮動對話面板。你可以用語音或文字提問、查看串流回答，並讓助理朗讀內容。不使用時，面板可以縮成一顆小膠囊。

### 翻譯：用任何語言說話，在本機取得整潔的英文

搭配 Whisper 模型，你可以使用其他語言說話，並在裝置上取得整理過的英文內容，全程不需要往返雲端。

### AI 清理：使用我們為口述內容訓練的模型

SpeakoFlow Mini 是我們自行訓練的模型，專門把口述內容整理成清楚文字。它會移除贅詞、修正文法與標點，並套用口頭編輯指令。當你在口述中說「new paragraph」或「scratch that」時，它會執行你的意思，而不是把指令原樣輸入。

模型下載大小為 795 MB，在你的電腦上執行，目前只支援英語。你可以再套用 Professional、Friendly、Concise 等寫作風格，或自訂指令。如果你已有信任的本機或雲端模型，也能改用自己的模型完成清理。

### 使用電腦上已有的模型

如果模型已經存在電腦中，SpeakoFlow 可以直接從原本的位置使用。你可以加入 `.gguf` 或 Whisper `.bin` 檔案，也可以連結一個資料夾，SpeakoFlow 會連同子資料夾一起掃描。檔案不會被複製或移動，移除項目也只會取消登錄。

若需要下載模型，SpeakoFlow 會同時抓取八個分段，並從中斷處繼續。相同連線下的實測速度從約 0.5 MB/s 提升到 19 MB/s。

### 網路搜尋、角色設定與個人記憶

助理可選擇性使用網路搜尋，取得最新資訊。角色設定可切換不同人物與聲音，每個角色也能設定回答長度。個人記憶儲存在裝置上，而且預設關閉。啟用後，你可以隨時查看、編輯或刪除內容。

所有選項都在「設定」中，每個快速鍵也都能重新指定。

各功能的完整文件：[Generate with Flow](https://www.speakoflow.com/docs/writing/generate-with-flow)、[螢幕視覺](https://www.speakoflow.com/docs/assistant/screen-vision)、[語音輸入](https://www.speakoflow.com/docs/dictation/basics)、[助理面板](https://www.speakoflow.com/docs/assistant/panel)、[語言與翻譯](https://www.speakoflow.com/docs/models/languages)、[AI 清理](https://www.speakoflow.com/docs/writing/ai-cleanup)、[網路搜尋](https://www.speakoflow.com/docs/assistant/web-search)、[角色設定](https://www.speakoflow.com/docs/personalize/profiles)與[個人記憶](https://www.speakoflow.com/docs/personalize/memory)。

## 預設快速鍵

| 操作 | Windows | macOS | Linux |
| --- | --- | --- | --- |
| 語音輸入 | `Left Ctrl + Left Super` | `Option + Space` | `Ctrl + Space` |
| 詢問助理 | `Left Ctrl + Left Alt` | `Option + Ctrl + Space` | `Ctrl + Alt + Space` |

按住快速鍵說話，放開後輸出文字。你也可以在設定中把「錄音方式」改為點按，第一次按下開始錄音，第二次按下停止，適合免手持操作。這項選擇會套用到所有錄音快速鍵，而且每組快速鍵都能重新指定。

三個平台的所有快速鍵與預設值請見[鍵盤快速鍵文件](https://www.speakoflow.com/docs/start/keyboard-shortcuts)。

## 安裝

請前往 [Releases](https://github.com/AbhishekBarali/SpeakoFlow/releases) 頁面下載適用於 Windows、macOS 或 Linux 的最新版本。首次啟動時，設定精靈會協助你選擇語音轉文字模型，也可以選擇下載本機助理模型。

### Windows

下載 `.exe` 安裝程式並執行。由於安裝程式尚未由已知發行者簽章，Windows 可能顯示 SmartScreen 警告。請選擇 **More info → Run anyway**。

### Linux

- **Arch Linux：** 從 AUR 安裝：
  ```bash
  yay -S speakoflow-bin
  # 或
  paru -S speakoflow-bin
  ```
- **Debian、Ubuntu 24.04 以上、Mint 22 以上、Pop!_OS、Tuxedo OS：** 下載 `.deb` 並安裝。這樣能正確登錄應用程式圖示與選單項目，AppImage 本身無法完成這些整合：
  ```bash
  sudo apt install ./SpeakoFlow_*_$(dpkg --print-architecture).deb
  ```
  `.deb` 使用 Ubuntu 24.04 建置，因此需要該時期的 glibc。較舊的發行版請改用 AppImage。
- **其他發行版，包括 Fedora 與 openSUSE：** 下載 AppImage，使用 `chmod +x` 加上執行權限後啟動。AppImage 不會自行整合桌面，因此不會自動出現在檔案管理員或應用程式選單。若需要整合，可使用 Gear Lever 或 AppImageLauncher。

AppImage 與 `.deb` 都提供 x86_64 與 ARM64 版本。目前沒有 `.rpm`，因為現有封裝方式無法正確包含語音引擎。與其提供安裝後無法轉錄的套件，我們選擇先不發布。

### macOS

下載 `.dmg`，把 **SpeakoFlow** 拖到「Applications」。接著前往 **System Settings → Privacy & Security**，允許 **Microphone** 與 **Accessibility** 權限，SpeakoFlow 才能聽見你的聲音並在其他應用程式中輸入文字。

應用程式尚未取得 Apple 簽章，因此 macOS 第一次啟動時會阻擋它。你需要使用一行終端機指令解除限制。下方有完整說明，也可以查看[安裝文件](https://www.speakoflow.com/docs/start/install#macos)。

<details>
<summary><b>為什麼 macOS 顯示「SpeakoFlow is damaged」，以及如何處理</b></summary>

<br />

SpeakoFlow 可在 macOS 上正常運作，但目前尚未取得 Apple 簽章。第一次啟動時，macOS 會顯示 **「SpeakoFlow is damaged and can't be opened」**。

**應用程式並未損壞。** 這是 macOS 對無法追溯至付費 Apple Developer 帳號的應用程式所顯示的訊息。簽章費用為每年 99 美元，本專案目前尚未負擔這項費用，因此出現阻擋屬於預期行為。

安裝步驟：

1. 依照機型下載安裝檔：Apple Silicon 使用 `SpeakoFlow_<version>_aarch64.dmg`，Intel Mac 使用 `SpeakoFlow_<version>_x64.dmg`。接著把 **SpeakoFlow** 拖到「Applications」。
2. 開啟 **Terminal**，可以按 `Cmd + Space` 後輸入 `Terminal`，貼上以下指令並按 Return：
   ```bash
   xattr -dr com.apple.quarantine /Applications/SpeakoFlow.app
   ```
3. 從 Launchpad、Spotlight 或「Applications」正常開啟 SpeakoFlow。

**每次安裝新版本只需執行一次。** 這行指令會移除 macOS 加在下載檔案上的「來自網際網路」標記。解除後，SpeakoFlow 就能正常啟動。因為未簽章版本無法自動更新，下載下一個版本時需要再執行一次，但不必每次啟動都執行。

macOS 15 之後移除了舊的右鍵 **Open** 略過方式，而且「damaged」訊息不會在 **System Settings → Privacy & Security** 顯示 **Open Anyway** 按鈕，因此只能透過終端機處理。正式的 Apple 簽章與公證已列入[開發計畫](#開發計畫)，完成後就不再需要這個步驟。

**Intel Mac 自 1.3.0 起支援。** Intel Mac 請下載 `SpeakoFlow_<version>_x64.dmg`，M1 以上機型請下載 `SpeakoFlow_<version>_aarch64.dmg`。Intel 版本只使用 CPU，因為 GPU 後端針對 Apple Silicon，因此轉錄速度較慢，但功能完整。

每個 Intel 版本都會在 CI 的實體 Intel 執行環境中檢查。測試會只保留應用程式本身的程式庫再啟動執行檔，無法啟動的套件不會進入發布頁面。你也可以[從原始碼建置](#從原始碼建置)，額外步驟請見 [BUILD.md](BUILD.md)。

> 本節早期版本曾寫道 GitHub 已停用 Intel 建置機器，因此無法產生或測試 Intel 版本。這項說法不正確。GitHub 於 2025 年 12 月停用舊的 `macos-13` runner，但以 `macos-15-intel` 取代，可使用至 2027 年 8 月。感謝 [@hellosimplerick](https://github.com/AbhishekBarali/SpeakoFlow/issues/19) 指出問題，也因此促成了 Intel 版本。

</details>

如需使用助理，請在設定中選擇服務來源：

- **內建離線模型：** 下載小型本機模型，完全在你的電腦上執行，不需要金鑰。
- **本機伺服器：** 將 SpeakoFlow 連接至 Ollama 或 LM Studio。
- **雲端：** 使用你自己的 API 金鑰，連接任何 OpenAI 相容服務。

## 從原始碼建置

需要先安裝 [Rust](https://rustup.rs/) 與 [Bun](https://bun.sh/)。

```bash
git clone https://github.com/AbhishekBarali/SpeakoFlow.git
cd SpeakoFlow
bun install
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
bun run tauri dev
```

在 Arch Linux 與其衍生發行版上，可使用以下指令建置並安裝目前的原始碼：

```bash
bun run install:arch
speak
```

這會把應用程式安裝到目前使用者的 `~/.local`，包括語音引擎程式庫、桌面項目與 `speak` 終端機指令。

各平台的設定方式請見 [BUILD.md](BUILD.md)。

## 技術架構

- **應用程式：** [Tauri 2](https://tauri.app)，搭配 Rust 後端與 React、TypeScript 前端。
- **語音轉文字：** whisper.cpp、Parakeet/Nemotron、SenseVoice 等本機引擎與模型，支援 GPU 加速，並使用 Silero VAD 偵測語音。
- **AI 助理：** 內建 llama.cpp 引擎，也可連接任何自行設定的 OpenAI 相容服務。
- **文字轉語音：** 本機使用 [Kokoro](https://github.com/hexgrad/kokoro)，也支援 OpenAI 相容服務、ElevenLabs 與 Azure。

## 隱私

語音在你的裝置上轉錄，不會上傳。助理只會連接你選擇的模型服務，也可以完全使用本機模型。SpeakoFlow 沒有遙測，也不需要帳號。網路搜尋與個人記憶等選用功能預設關閉，記憶內容儲存在你的裝置上，可隨時查看、編輯或刪除。

儲存內容與位置的完整說明請見[隱私文件](https://www.speakoflow.com/docs/reference/privacy)。

## 疑難排解

下方列出常見問題。若問題未包含在內，請查看[疑難排解文件](https://www.speakoflow.com/docs/reference/troubleshooting)或[建立 issue](https://github.com/AbhishekBarali/SpeakoFlow/issues)。

<details>
<summary><b>Linux：錄音浮層無法保持在其他視窗上方</b></summary>

<br />

錄音浮層必須顯示在所有視窗上方。Linux 只能透過兩種方式做到這件事：Sway、Hyprland 與 KDE Plasma 等環境使用的 `wlr-layer-shell` 協定，或傳統 X11 的「keep above」堆疊。

**原生 GNOME/Wayland 兩者都不支援。** Mutter 沒有實作 `wlr-layer-shell`，Wayland 也不允許應用程式自行提升到其他視窗上方，因此原生 GNOME/Wayland 無法讓浮層保持置頂。

SpeakoFlow 會自動處理這個問題。偵測到 GNOME on Wayland 時，它會透過 **XWayland** 執行，讓「keep above」正常運作。這是預設行為，不需設定。X11 工作階段與 KDE、wlroots Wayland 也能直接使用。

- 若仍要強制使用原生 Wayland，請以 `SPEAKOFLOW_ALLOW_WAYLAND=1` 啟動，浮層可能無法置頂。
- 若浮層在 layer-shell compositor 下運作異常，可使用 `SPEAKOFLOW_NO_GTK_LAYER_SHELL=1` 停用 layer shell。

</details>

<details>
<summary><b>Linux：快速鍵沒有反應，日誌重複顯示「Permission denied」</b></summary>

<br />

如果 Linux 上的語音輸入與助理快速鍵沒有反應，而且日誌反覆出現 `rdev grab error: ... PermissionDenied`，errno 13，代表應用程式無法讀取輸入裝置。這會影響直接讀取 `/dev/input/event*` 的 **handy-keys** 鍵盤引擎，使用者必須加入 `input` 群組。

有兩種處理方式，建議優先使用 Tauri：

- **切換引擎：** 在設定中把鍵盤引擎設為 **Tauri**。它使用 compositor 的全域快速鍵 API，不需要讀取原始輸入裝置。Tauri 已是 Linux 的預設引擎，只有曾手動切換至 handy-keys 的使用者會遇到這個問題。

- **授予 handy-keys 權限：** 如果確實需要 handy-keys，可以將使用者加入 `input` 群組，接著登出再登入：

  ```bash
  sudo usermod -aG input $USER
  ```

  `input` 群組可讀取系統層級的原始鍵盤事件。加入後，同一使用者帳號下執行的其他程式也可能讀到按鍵內容，包括密碼。請只在受信任的電腦上使用這個方式。

</details>

<details>
<summary><b>Linux：在觸控板縮放時應用程式當機</b></summary>

<br />

在部分 Linux 環境中，以觸控板縮放會造成視窗當機，日誌會出現 `Received invalid message: 'DrawingArea_CommitTransientZoom'`。這是 Tauri 與 wry 所使用的 Linux 網頁引擎 **WebKitGTK** 的問題，不是 SpeakoFlow 本身的錯誤。上游追蹤連結為 [tauri#13115](https://github.com/tauri-apps/tauri/issues/13115)與 [wry#544](https://github.com/tauri-apps/wry/issues/544)。

上游修正前，請避免在應用程式視窗內使用觸控板縮放。將系統的 `webkit2gtk-4.1` 套件更新至最新版也可能改善情況。

</details>

## 開發計畫

- Windows 與 macOS 程式碼簽章
- 更完整的模型目錄與更多一鍵下載的本機模型
- 更多社群翻譯
- 針對代理式程式開發調校的語音轉文字
- 提示詞協助：描述想做的內容，產生可直接使用的提示詞
- 語音指令：透過說話觸發操作並完成工作

## 參與貢獻

歡迎參與開發。請先閱讀 [CONTRIBUTING.md](CONTRIBUTING.md)。若要協助翻譯應用程式，請參考 [CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md)。

## 授權

本專案依 [MIT License](LICENSE) 發布。

## 致謝

SpeakoFlow 的語音輸入核心以 CJ Pais 開發的 [Handy](https://github.com/cjpais/Handy) 為基礎，並依 MIT 授權使用。感謝 CJ 將專案開源。助理、螢幕視覺、Generate with Flow、翻譯、文字轉語音與記憶功能由 SpeakoFlow 開發。

也感謝 [Tauri](https://tauri.app)、whisper.cpp、llama.cpp、Silero VAD 與 [Kokoro](https://github.com/hexgrad/kokoro)。

<div align="center">

作者：[Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
