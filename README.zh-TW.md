<div align="center">

[English](README.md) · [简体中文](README.zh-CN.md) · **繁體中文**

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final-v2/png/lockup-dark-h256.png" />
  <img src="Logo/final-v2/png/lockup-h256.png" alt="SpeakoFlow" width="320" />
</picture>

# 適用於 Windows、macOS 與 Linux 的免費語音輸入與 AI 助理

說話，它就替你打字，任何應用程式都行。提問，它就回答。開啟對話，和它把問題聊清楚。<br />
它也能幫你整理會議紀錄。免費、開源，預設在本機執行。

[![Latest release](https://img.shields.io/github/v/release/AbhishekBarali/SpeakoFlow?label=release&color=0A7A70)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational)](#安裝)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)

<img src="assets/readme/demo.webp" width="760" alt="SpeakoFlow 示範：口述一封電子郵件並自動輸入，請助理翻譯選取的文字並以答案取代，接著與助理進行語音對話" />

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?logo=windows&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?logo=apple&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?logo=linux&logoColor=black&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)

[官方網站](https://www.speakoflow.com) &nbsp;·&nbsp; [使用文件](https://www.speakoflow.com/docs) &nbsp;·&nbsp; [所有版本](https://github.com/AbhishekBarali/SpeakoFlow/releases)

</div>

## 它能做什麼

你思考的速度比打字更快。SpeakoFlow 讓你在任何應用程式中，只要一組快速鍵，就能改用聲音工作：

<table>
<tr>
<td width="33%" valign="top">

**語音輸入**

<kbd>Left Ctrl</kbd> + <kbd>Left Win</kbd>

按住按鍵，說話，放開。你說的內容會輸入到游標所在的位置，任何可輸入文字的應用程式都可以。

</td>
<td width="33%" valign="top">

**提問**

<kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd>

可以先選取一段文字，也可以不選，接著直接說出你的需求。翻譯、回覆或解釋都行。答案可以複製、插入，或取代選取的文字。

</td>
<td width="33%" valign="top">

**對話**

<kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd> + <kbd>C</kbd>

與助理進行免手持的語音對話。把問題說出來慢慢釐清，它會出聲回答。再按一次這組按鍵即可結束。

</td>
</tr>
</table>

<sub>以上為 Windows 預設快速鍵。macOS 與 Linux 的按鍵請見[鍵盤快速鍵](#鍵盤快速鍵)，每一組都可以重新指定。</sub>

除非你選擇雲端服務，語音都會在你自己的電腦上轉錄。助理使用你選擇的模型：可離線執行的內建模型、你自己的 Ollama 或 LM Studio 伺服器，或使用自有 API 金鑰的雲端服務。

我在獨自準備考試時做了 SpeakoFlow。當時我付費使用的語音輸入軟體聽得見我說話，卻幫不上忙，所以我做了一個兩件事都能做的工具。

## 中文使用者須知

SpeakoFlow 內建繁體中文與簡體中文介面，也支援中文語音轉文字，辨識全程在本機完成。你可以依需求選擇模型：

- **Nemotron：** 支援包括中文在內的 28 種語言，並能自動偵測語言。
- **SenseVoice：** 體積小、速度快，支援國語、粵語、英語、日語與韓語。
- **Breeze ASR：** 針對臺灣華語調校，也能辨識中英夾雜的口述內容。
- **Whisper：** 多語言模型，可以辨識中文，也能把其他語言翻譯成英文。

在語言選單中選擇「繁體中文」或「簡體中文」後，SpeakoFlow 會透過 OpenCC 將輸出統一轉成所選的字形，不必手動轉換。

> **AI 整理的語言限制：** 內建的 SpeakoFlow Mini 整理模型目前只支援英語。中文使用者可以不啟用 AI 整理，或改用支援中文的其他本機或雲端模型。這項限制不影響中文語音辨識。

## 功能

### 語音輸入

按住按鍵，說話，放開。文字會出現在游標所在的位置，錄音浮層也能在你說話時即時顯示文字。轉錄透過本機模型在顯示卡或處理器上完成：英語預設使用 Parakeet，Nemotron 支援 28 種語言並能自動偵測，Whisper 支援 99 種語言，模型目錄中共有 65 個語音模型。如果你偏好雲端，ElevenLabs 與 Deepgram 可以邊說邊出字，OpenAI、Groq、Mistral、Azure AI Speech 與 OpenRouter 也都能使用。

- **復原。** 不小心取消了錄音？浮層會在幾秒內提供「復原」，歷史紀錄也會保留這段錄音，之後仍可復原。轉錄失敗時則會提供「重試」。
- **照你的方式拼寫。** 把人名與專業術語加入字典，或設定文字取代規則。在 Windows 上，它也能學習你修正過的字詞。
- **翻譯成英文。** Whisper、Canary、Granite Speech 與 Voxtral 模型，以及雲端的 OpenAI 或 Groq，可以把其他語言的語音直接轉成英文文字。
- **按住或按一下。** 可以按住按鍵說話，也可以切換成按一下模式：按一次開始，再按一次停止。

### 提問

<div align="center">
<img src="assets/readme/ask.webp" width="720" alt="快速提問的三個範例：把選取的訊息翻譯成西班牙文並取代、撰寫回覆並插入到游標處，以及解釋選取的句子" />
</div>

選取一些文字，或什麼都不選。按住 <kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd>，說出你想要的：「把這段翻譯成西班牙文」「寫一則禮貌的回覆，說我星期四沒辦法參加」「解釋一下這個」。答案會以卡片形式即時顯示在目前的應用程式上方，你可以複製、插入到游標處，或用它取代選取的文字。

如果你允許，它還能做更多：

- **螢幕視覺。** 詢問終端機裡的錯誤，或試算表中的圖表代表什麼。這項功能預設關閉；啟用後，也由模型針對每個問題判斷是否需要查看螢幕。沒有用到的螢幕截圖不會離開你的電腦。
- **網路搜尋**，支援 Serper、Brave、Tavily、Exa、SerpAPI 或 TinyFish。
- **提醒。** 「二十分鐘後提醒我寄發票。」提醒在重新啟動後依然有效，彈出時也不會搶走你的鍵盤焦點。
- **角色與記憶。** 為它設定不同的角色，每個角色有各自的回答長度，也可以讓它記住你的工作習慣。記憶預設關閉，只儲存在你的電腦上，你可以隨時編輯或清除。

### 對話

按下 <kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd> + <kbd>C</kbd> 後開始說話。助理會出聲回答，你也可以在它說話時隨時插話。Esc 會停止目前的回答，但不會結束對話；再按一次這組按鍵則會結束對話。中途需要輸入一些文字？照常使用語音輸入即可，對話會暫停等你，結束後再繼續。

回答可以由你電腦上的語音朗讀（Kokoro、Kitten、Pocket TTS 或 Supertonic），也可以使用 OpenAI、ElevenLabs、Deepgram、Cartesia、Google、Azure 等雲端語音。

### 會議 <sup>beta</sup>

<table>
<tr>
<td width="50%"><img src="assets/readme/screens/meetings-live.webp" alt="正在錄製的會議，即時逐字稿區分了你說的話與對方說的話" /></td>
<td width="50%"><img src="assets/readme/screens/meeting-notes.webp" alt="會議結束後產生的紀錄：摘要、重點、主題、決議，以及附負責人的後續事項" /></td>
</tr>
</table>

開會前按下「開始錄製」。SpeakoFlow 會把你的麥克風與電腦播放的聲音分成兩路錄製，並在大家說話時即時轉錄，所以它始終知道哪些話是你說的。不會有機器人加入會議，任何會議軟體都能使用。在 Windows 上，它偵測到通話時也會主動詢問是否錄製。

通話結束後，它會依照你選擇的範本（一般、站立會議、一對一、面試或待辦事項）寫出會議紀錄，包含摘要、決議與附負責人的後續事項。其他人的聲音會標示為「說話者 1」「說話者 2」等。之後你可以針對這場會議提問，也可以選擇「討論這場會議」，在對話中把它討論一遍。

在 macOS 上，若要錄製通話中對方的聲音，需要 BlackHole 之類的虛擬音訊裝置。詳見[疑難排解](#疑難排解)。

### AI 整理

SpeakoFlow Mini 是我們為單一任務訓練的小型模型：把你說的話整理成乾淨的文字。它會移除贅詞，修正文法與標點，並執行口頭編輯指令，所以當你說「scratch that」或「actually, eleven」時，它會照你的意思修改，而不是把這些話原樣打出來。模型下載大小為 795 MB，在你的電腦上執行，目前只支援英語。你也可以改用其他任何本機或雲端模型來完成這件事，在 Apple 晶片的 Mac 上還能使用 Apple Intelligence。

AI 整理預設關閉。啟用後，它可以有自己的快速鍵（語音輸入的按鍵加上 Shift），也可以在每次語音輸入時自動執行。在此之上，你還能套用一種寫作風格：專業、友善、精簡、正式、輕鬆，或你自己撰寫的風格。

### 其他功能

- **使用分析。** 已口述的字數、你的語速、與每分鐘 40 個字的打字速度相比節省的時間，以及六個月的活動紀錄與連續天數。
- **歷史紀錄。** 你的語音輸入、提問與對話都在這裡。可以重播錄音、重新轉錄、復原已取消的錄音，或把一段聊天接著轉成語音對話。舊錄音可以依數量、天數或月數自動刪除。
- **使用電腦上已有的模型。** 加入 `.gguf` 或 Whisper `.bin` 檔案，或連結一個資料夾，裡面的所有模型都會出現在應用程式中。檔案不會被複製或移動。需要下載的模型會同時抓取八個分段，並從中斷處繼續。
- **使用 Flow 生成。** 以「Hey Flow」開頭開始口述，描述你想寫的內容，貼上的會是寫好的文字，而不是你的原話。預設關閉，可在「設定 → 語音輸入」中啟用。
- **20 種介面語言。**

每項功能在[使用文件](https://www.speakoflow.com/docs)中都有專屬頁面。

## 介面一覽

<table>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/home.webp" alt="首頁：所有快速鍵與「按住說話／按一下切換」開關、負責各項工作的模型，以及最近的語音輸入" /><br /><sub><b>首頁。</b> 你的快速鍵、負責各項工作的模型，以及最近口述的內容。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/assistant.webp" alt="助理頁面：提問與對話的快速鍵、助理使用的模型、它的聲音，以及螢幕視覺與網路搜尋的開關" /><br /><sub><b>助理。</b> 它的模型、它的聲音，以及允許它做什麼。</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/cleanup.webp" alt="AI 整理頁面：它的快速鍵、作為整理模型的 SpeakoFlow Mini，以及附前後對照範例的寫作風格" /><br /><sub><b>AI 整理。</b> 隨口說，得到整齊的文字，風格由你挑選。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/insights.webp" alt="使用分析頁面：口述字數、每分鐘字數、節省的時間、語音輸入次數，以及六個月的活動圖" /><br /><sub><b>使用分析。</b> 你口述了多少，省下了多少打字時間。</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/models.webp" alt="在這台電腦上執行的語音轉文字模型，Parakeet 正在使用中，另有更多模型可供下載" /><br /><sub><b>模型。</b> 每項工作都可以在本機或雲端執行。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/models-voice.webp" alt="語音設定：已選擇並在本機就緒的 Kokoro，旁邊是其他本機語音與十幾種雲端語音" /><br /><sub><b>語音。</b> 四種本機語音與十幾種雲端語音。</sub></td>
</tr>
</table>

## 鍵盤快速鍵

| 操作                        | Windows                        | macOS           | Linux                  |
| --------------------------- | ------------------------------ | --------------- | ---------------------- |
| 語音輸入                    | `Left Ctrl + Left Win`         | `Fn` (🌐)       | `Ctrl + Space`         |
| 詢問助理                    | `Left Ctrl + Left Alt`         | `Fn + Ctrl`     | `Ctrl + Alt + Space`   |
| 開始或結束對話              | `Left Ctrl + Left Alt + C`     | `Fn + Ctrl + C` | `Ctrl + Alt + C`       |
| 語音輸入並整理 <sup>1</sup> | `Left Ctrl + Left Win + Shift` | `Fn + Shift`    | `Ctrl + Shift + Space` |
| 取消                        | `Esc`                          | `Esc`           | 尚未支援               |

<sup>1</sup> 僅在 AI 整理已啟用並設有專屬快速鍵時可用。

各平台的規則相同：語音輸入的按鍵加上 Shift 就是語音輸入並整理，提問的按鍵加上 C 就是開始對話。錄音快速鍵預設需要按住使用；在首頁把「按住說話」切換成「按一下切換」後，按一次開始，再按一次停止。

Esc 只在有工作進行時（例如正在錄音，或正在朗讀回答）才會取消，其他時候其他應用程式仍可照常使用 Esc。若要更改快速鍵，點選它的按鍵即可。取消與對話這兩組快速鍵也可以在那裡關閉。

在 Mac 上，請把「系統設定 → 鍵盤 → 按下 🌐 鍵來」設為「不執行任何動作」，否則按下地球鍵時還會開啟表情符號選擇器。原本使用舊版 Option + Space 預設快速鍵的 Mac，更新後會保留原來的設定。

指令碼與視窗管理員可以透過 `--toggle-transcription` 等[命令列參數](https://www.speakoflow.com/docs/settings/cli)控制 SpeakoFlow。

## 模型與服務供應商

每項工作都可以在你的電腦上執行，也可以交給你選擇的服務供應商。雲端服務使用你自己的 API 金鑰，金鑰儲存在系統鑰匙圈中。

| 工作             | 在你的電腦上                                                                                                                                   | 在雲端（使用你的金鑰）                                                                                                                                                                         |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 語音轉文字       | Parakeet、Nemotron、Canary、Cohere Transcribe、Whisper、Moonshine、Voxtral、Qwen3-ASR、GigaAM、Granite Speech、SenseVoice 等（目錄中共 65 個） | ElevenLabs、Deepgram、OpenAI、Groq、Mistral (Voxtral)、Azure AI Speech、OpenRouter，或任何 OpenAI 相容伺服器                                                                                   |
| 助理與整理       | 內建引擎（llama.cpp，完全離線）、Ollama、LM Studio、用於整理的 SpeakoFlow Mini、Apple 晶片上用於整理的 Apple Intelligence                      | OpenAI、Anthropic、Google Gemini、Azure OpenAI、AWS Bedrock、OpenRouter、Groq、Cerebras、xAI、DeepSeek、Mistral、Moonshot、Together AI、Fireworks AI、Perplexity、Z.AI，或任何 OpenAI 相容端點 |
| 語音回答         | Kokoro、Kitten、Pocket TTS、Supertonic                                                                                                         | OpenAI、ElevenLabs、OpenRouter、Deepgram、Cartesia、Google Cloud、Azure AI Speech、Groq、xAI、Mistral、Inworld，或你自己的伺服器                                                               |
| 網路搜尋（選用） |                                                                                                                                                | Serper、Brave、Tavily、Exa、SerpAPI、TinyFish                                                                                                                                                  |

## 隱私

預設情況下，你的語音會在你的電腦上轉錄，不會上傳。沒有遙測、沒有數據分析，也不需要帳號。

只有在你自行設定以下功能時，資料才會離開你的電腦：

- **雲端語音服務**：如果你選擇它來取代本機模型。它會收到你的錄音。
- **助理的服務供應商**：如果它不是本機服務。它會收到你的問題、你選取的文字、啟用螢幕視覺且模型要求時的螢幕截圖，以及你產生會議紀錄或針對會議提問時的會議逐字稿。
- **網路搜尋**：如果你啟用了它。搜尋服務會收到搜尋字詞。
- **意見回饋**：如果你從應用程式內傳送意見回饋。傳送的內容與對話框中顯示的完全相同。

API 金鑰儲存在系統鑰匙圈中。記憶在你啟用之前都是關閉的，而且只儲存在你的電腦上，你可以查看、編輯或清除。更多細節請見[隱私頁面](https://www.speakoflow.com/docs/reference/privacy)。

## 安裝

請從 [Releases](https://github.com/AbhishekBarali/SpeakoFlow/releases) 頁面下載最新版本。首次啟動時你需要選擇一個語音模型，模型下載期間會有一段簡短的導覽介紹快速鍵。

### Windows

執行 `.exe` 安裝程式。由於安裝程式尚未由已知發行者簽章，Windows 可能會顯示 SmartScreen 提示；請選擇 **More info → Run anyway**（「其他資訊 → 仍要執行」）。

### macOS

下載適合你 Mac 的 `.dmg`（Apple 晶片選 `aarch64`，Intel 選 `x64`），把 **SpeakoFlow** 拖到「應用程式」。這個應用程式尚未取得 Apple 簽章，所以 macOS 會顯示它「已損毀，無法打開」。它並沒有損毀。請在「終端機」執行下面這行指令解除一次限制，之後就能正常開啟：

```bash
xattr -dr com.apple.quarantine /Applications/SpeakoFlow.app
```

接著 SpeakoFlow 會要求**麥克風**與**輔助使用**權限，這樣它才能聽見你說話，並在其他應用程式中輸入文字。

<details>
<summary>關於 macOS 安裝的更多說明</summary>

<br />

對於無法追溯到付費 Apple Developer 帳號的應用程式，macOS 都會顯示「已損毀」的訊息。簽章費用為每年 99 美元，本專案目前還沒有這筆預算。macOS 15 之後移除了舊的右鍵 → **打開** 略過方式，而且這則訊息是唯一一種在「系統設定」中不提供「強制打開」按鈕的情況，所以只能透過終端機處理。這行指令會移除該應用程式副本上「從網際網路下載」的標記。

每次下載後執行一次即可。在應用程式內安裝的更新不會帶有這個標記，因此不必再執行。如果你手動下載了新的 `.dmg`，就需要對那個副本再執行一次。

更新後，macOS 有時會在「輔助使用」「麥克風」或「螢幕錄製」中繼續顯示 SpeakoFlow 已獲授權，但實際上不再生效。如果某個權限畫面一直在等待，請按下其中的「重設權限」按鈕，然後在「系統設定」中重新開啟 SpeakoFlow。

Intel 版本需要 macOS 14 Sonoma 或更新版本。它只使用處理器執行，所以轉錄速度比 Apple 晶片慢，但所有功能都能使用。每個 Intel 版本發布前，CI 都會在實體 Intel Mac 上啟動測試。

</details>

### Linux

- **Arch Linux。** 從 AUR 安裝 `speakoflow-bin`，例如 `yay -S speakoflow-bin`。
- **Debian 13+、Ubuntu 24.04+、Mint 22+、Pop!\_OS。** 安裝 `.deb`，它也會加入應用程式圖示與選單項目：
  ```bash
  sudo apt install ./SpeakoFlow_*_amd64.deb
  ```
- **其他發行版，包括 Fedora 與 openSUSE。** 使用 AppImage：以 `chmod +x` 加上執行權限後啟動。Gear Lever 或 AppImageLauncher 等工具可以把它加入應用程式選單。

兩種套件都以 Ubuntu 24.04 建置，提供 x86_64 與 ARM64 版本，因此需要 glibc 2.39 或更新版本。這表示 Ubuntu 22.04、Debian 12、Mint 21，以及 RHEL 9 與其衍生版本無法使用。目前沒有 `.rpm`，因為現有封裝方式無法正確包含語音引擎，而一個能安裝卻無法轉錄的套件比沒有更糟。

### 更新

SpeakoFlow 會在背景檢查新版本，並在「設定 → 關於」中安裝更新，安裝前會以專案的簽章金鑰驗證每個版本。AUR 套件則透過你的套件管理員更新。如果想在 GitHub 上收到新版本通知，請點選本頁上方的 **Watch → Custom → Releases**。

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

在 Arch 系列的發行版上，`bun run install:arch` 會建置目前的原始碼，並安裝到 `~/.local`，同時加入桌面項目與 `speak` 指令。各平台的設定方式請見 [BUILD.md](BUILD.md)。

應用程式以 [Tauri 2](https://tauri.app) 打造，後端為 Rust，前端為 React 與 TypeScript。語音辨識使用 transcribe.cpp、whisper.cpp 與 ONNX Runtime，並搭配 Silero VAD；助理與整理使用內建的 llama.cpp 引擎或任何 OpenAI 相容 API；本機語音在應用程式視窗中使用 [Kokoro](https://github.com/hexgrad/kokoro)，在處理器上使用 [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)；會議中的說話者標示使用 WeSpeaker 聲紋向量。

## 比較

|          | SpeakoFlow | Wispr Flow                                    | Superwhisper                                   | Handy      |
| -------- | ---------- | --------------------------------------------- | ---------------------------------------------- | ---------- |
| 價格     | 免費       | 桌面版每週 2,000 字以內免費，之後每月 15 美元 | 有免費版，Pro 每月 8.49 美元或買斷 249.99 美元 | 免費       |
| 原始碼   | 開源 (MIT) | 閉源                                          | 閉源                                           | 開源 (MIT) |
| Linux    | 支援       | 不支援                                        | 不支援                                         | 支援       |
| 離線轉錄 | 支援       | 不支援                                        | 支援                                           | 支援       |
| AI 助理  | 有         | 有                                            | 無                                             | 無         |

<sub>價格與平台資訊取自各產品官網，查閱於 2026 年 10 月。</sub>

SpeakoFlow 的語音輸入核心來自 [Handy](https://github.com/cjpais/Handy)；如果你只需要語音輸入，Handy 是很好的選擇。更多細節：[SpeakoFlow 與 Wispr Flow 比較](https://www.speakoflow.com/blog/speakoflow-vs-wispr-flow)，以及[免費開源的 Wispr Flow 替代方案](https://www.speakoflow.com/blog/best-free-open-source-wispr-flow-alternatives)。

## 疑難排解

下方列出常見問題。其他問題請查看[疑難排解文件](https://www.speakoflow.com/docs/reference/troubleshooting)或[建立 issue](https://github.com/AbhishekBarali/SpeakoFlow/issues)。

<details>
<summary><b>macOS：會議只錄到我這一方的聲音</b></summary>

<br />

macOS 沒有提供應用程式直接錄製電腦播放聲音的方式。Windows 有 WASAPI loopback，Linux 有 PulseAudio 或 PipeWire 的監聽來源，而 Mac 需要在中間加入虛擬音訊裝置。

安裝 [BlackHole](https://github.com/ExistentialAudio/BlackHole) 之類的回送驅動程式，在「音訊 MIDI 設定」中建立同時輸出到喇叭與 BlackHole 的「多重輸出裝置」，並把它設為輸出裝置。這樣 SpeakoFlow 就能錄到通話另一方的聲音。你的麥克風無論如何都會被錄製。

</details>

<details>
<summary><b>Linux：錄音浮層無法保持在其他視窗上方</b></summary>

<br />

在 Linux 上，視窗只能透過 `wlr-layer-shell` 協定（Sway、Hyprland 等 wlroots compositor，以及 KDE Plasma）或 X11 的「keep above」堆疊來浮在其他視窗上方。原生 GNOME Wayland 兩者都不支援，所以 SpeakoFlow 偵測到它時會改以 XWayland 執行，浮層就能正常置頂。這不需要任何設定，X11 以及 KDE 或 wlroots 的 Wayland 也都能直接使用。

- 若仍要強制使用原生 Wayland，請以 `SPEAKOFLOW_ALLOW_WAYLAND=1` 啟動。浮層可能無法保持置頂。
- 若浮層在 layer-shell compositor 下運作異常，請以 `SPEAKOFLOW_NO_GTK_LAYER_SHELL=1` 啟動。

</details>

<details>
<summary><b>Linux：快速鍵沒有反應，日誌重複顯示「Permission denied」</b></summary>

<br />

如果日誌中充滿 `rdev grab error: ... PermissionDenied`，代表應用程式無法讀取你的輸入裝置。這只會影響 **SpeakoFlow Keys** 鍵盤引擎：它要讀取 `/dev/input/event*`（需要你的使用者屬於 `input` 群組），還要透過 `/dev/uinput` 重新送出按鍵（在包括 Ubuntu 在內的許多發行版上預設只有 root 可寫入，所以只加入該群組還不夠）。Linux 預設使用的是 Tauri 引擎，所以只有手動切換過引擎才會遇到這個問題。首頁的「快捷鍵」卡片會提示這種情況，並列出要執行的指令。

- 授予這兩項權限，然後登出再登入：
  ```bash
  sudo usermod -aG input "$USER"
  echo 'KERNEL=="uinput", GROUP="input", MODE="0660"' | sudo tee /etc/udev/rules.d/70-speakoflow-uinput.rules
  sudo udevadm control --reload && sudo udevadm trigger /dev/uinput
  ```
- 或在「設定 → 進階」中把鍵盤引擎切回 **Tauri**。它不需要任何權限，但透過 X11 註冊快速鍵，所以在原生 Wayland 下，只有當某個 X11 視窗取得焦點時才能回應。

在 Wayland 上，最可靠的做法是使用桌面環境本身的快速鍵。在 Wayland 工作階段中，「快捷鍵」卡片會顯示「設定」按鈕，列出每個操作對應的指令，可直接複製。在 GNOME 或 KDE 設定中新增自訂快速鍵，或在 Sway、Hyprland 中加入一行 `bind`，讓它執行 `speakoflow --toggle-transcription`（若使用 AppImage，就寫它的路徑再加上相同參數）。`--toggle-post-process`、`--toggle-assistant`、`--toggle-call`（開始或結束對話）與 `--cancel` 的用法相同。這樣設定的快速鍵按一次開始、再按一次停止，和「按一下切換」一樣。

</details>

<details>
<summary><b>Linux：在觸控板上縮放時應用程式當機</b></summary>

<br />

如果當機時日誌出現 `Received invalid message: 'DrawingArea_CommitTransientZoom'`，這是 WebKitGTK 的錯誤，會影響許多以它打造的應用程式，上游追蹤連結為 [tauri#13115](https://github.com/tauri-apps/tauri/issues/13115) 與 [wry#544](https://github.com/tauri-apps/wry/issues/544)。上游修正前，請避免在視窗內使用觸控板縮放。將 `webkit2gtk-4.1` 更新到最新版本可能有幫助。

</details>

## 開發計畫

- Windows 與 macOS 程式碼簽章
- 更多一鍵安裝的本機模型
- 更多社群翻譯
- 針對代理式程式開發調校的語音輸入
- 提示詞協助：描述你想做的東西，取得一份可靠的提示詞
- 能替你執行操作的語音指令

## 參與貢獻

歡迎參與貢獻。[CONTRIBUTING.md](CONTRIBUTING.md) 說明如何開始，[CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) 說明如何翻譯應用程式。

發現問題或有新點子？可以在應用程式內使用「傳送意見回饋」（「設定」旁邊的 **?**），或[建立 issue](https://github.com/AbhishekBarali/SpeakoFlow/issues)。

## 授權與致謝

SpeakoFlow 依 [MIT License](LICENSE) 發布。

語音輸入核心來自 CJ Pais 開發的 [Handy](https://github.com/cjpais/Handy)，依 MIT 授權使用。感謝 CJ 將它開源。助理、對話、會議、螢幕視覺、使用 Flow 生成、翻譯、語音回答與記憶功能由 SpeakoFlow 自行開發。

也感謝 [Tauri](https://tauri.app)、whisper.cpp、llama.cpp、ONNX Runtime、[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)、Silero VAD、WeSpeaker、[Kokoro](https://github.com/hexgrad/kokoro) 以及 Kyutai 的 Pocket TTS。

<div align="center">

作者：[Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
