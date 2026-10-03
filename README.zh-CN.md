<div align="center">

[English](README.md) · **简体中文** · [繁體中文](README.zh-TW.md)

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="Logo/final-v2/png/lockup-dark-h256.png" />
  <img src="Logo/final-v2/png/lockup-h256.png" alt="SpeakoFlow" width="320" />
</picture>

# 适用于 Windows、macOS 和 Linux 的免费语音输入与 AI 助手

说话，它就替你打字，任何应用都行。提问，它就回答。开启对话，和它把问题聊透。<br />
它还能帮你整理会议纪要。免费、开源，默认在本地运行。

[![Latest release](https://img.shields.io/github/v/release/AbhishekBarali/SpeakoFlow?label=release&color=0A7A70)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-2ea44f.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/Windows%20%7C%20macOS%20%7C%20Linux-informational)](#安装)
[![Built with Tauri](https://img.shields.io/badge/built%20with-Tauri%202-24C8DB?logo=tauri&logoColor=white)](https://tauri.app)

<img src="assets/readme/demo.webp" width="760" alt="SpeakoFlow 演示：口述一封邮件并自动输入，让助手翻译选中的文字并用答案替换，然后与助手进行语音对话" />

[![Download for Windows](https://img.shields.io/badge/Download-Windows-0078D4?logo=windows&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for macOS](https://img.shields.io/badge/Download-macOS-000000?logo=apple&logoColor=white&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)
[![Download for Linux](https://img.shields.io/badge/Download-Linux-FCC624?logo=linux&logoColor=black&style=for-the-badge)](https://github.com/AbhishekBarali/SpeakoFlow/releases/latest)

[官方网站](https://www.speakoflow.com) &nbsp;·&nbsp; [使用文档](https://www.speakoflow.com/docs) &nbsp;·&nbsp; [所有版本](https://github.com/AbhishekBarali/SpeakoFlow/releases)

</div>

## 它能做什么

你思考的速度比打字快。SpeakoFlow 让你在任何应用里用一个快捷键，改用声音来工作：

<table>
<tr>
<td width="33%" valign="top">

**语音输入**

<kbd>Left Ctrl</kbd> + <kbd>Left Win</kbd>

按住按键，说话，松开。你说的话会输入到光标所在的位置，任何能输入文字的应用都可以。

</td>
<td width="33%" valign="top">

**提问**

<kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd>

可以先选中一段文字，也可以不选，然后直接说出你的要求。翻译它、回复它、解释它都行。答案可以复制、插入，或者替换掉选中的文字。

</td>
<td width="33%" valign="top">

**对话**

<kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd> + <kbd>C</kbd>

与助手进行免手动的语音对话。把问题说出来慢慢理清，它会出声回答。再按一次这组按键即可结束。

</td>
</tr>
</table>

<sub>以上为 Windows 默认快捷键。macOS 和 Linux 的按键见[键盘快捷键](#键盘快捷键)，每一组都可以修改。</sub>

除非你选择云端服务，语音都在你自己的电脑上转写。助手使用你选择的模型：可以离线运行的内置模型、你自己的 Ollama 或 LM Studio 服务器，或者使用你自己 API 密钥的云端服务商。

我是在独自备考时做出 SpeakoFlow 的。当时我付费使用的语音输入软件听得见我说话，却帮不上我的忙，于是我做了一个两件事都能做的工具。

## 中文用户须知

SpeakoFlow 内置简体中文和繁体中文界面，也支持中文语音转文字，识别全程在本地完成。可以按需选择模型：

- **Nemotron：** 支持包括中文在内的 28 种语言，并能自动识别语言。
- **SenseVoice：** 体积小、速度快，支持普通话、粤语、英语、日语和韩语。
- **Breeze ASR：** 针对台湾华语调校，也能识别中英夹杂的口述内容。
- **Whisper：** 多语言模型，可以识别中文，也能把其他语言翻译成英文。

在语言菜单中选择“简体中文”或“繁体中文”后，SpeakoFlow 会用 OpenCC 把输出统一转换为对应的字形，无需手动转换。

> **AI 清理的语言限制：** 内置的 SpeakoFlow Mini 清理模型目前只支持英语。中文用户可以不开启 AI 清理，或改用支持中文的其他本地或云端模型。这不影响中文语音识别本身。

## 功能

### 语音输入

按住按键，说话，松开。文字会出现在光标所在的位置，录音浮窗还可以在你说话时实时显示文字。转写在你的显卡或处理器上用本地模型完成：英语默认使用 Parakeet，Nemotron 支持 28 种语言并能自动识别，Whisper 支持 99 种语言，模型目录中共有 65 个语音模型。如果你更想用云端，ElevenLabs 和 Deepgram 可以边说边出字，OpenAI、Groq、Mistral、Azure AI Speech 和 OpenRouter 也都能用。

- **撤销。** 不小心取消了录音？浮窗会在几秒内提供“撤销”，历史记录也会保留这段录音，之后仍可恢复。转写失败时则会提供“重试”。
- **按你的方式拼写。** 把人名和专业术语加入词典，或者设置文本替换规则。在 Windows 上，它还能学习你修改过的词。
- **翻译成英文。** Whisper、Canary、Granite Speech 和 Voxtral 模型，以及云端的 OpenAI 或 Groq，可以把其他语言的语音直接转成英文文本。
- **按住或按一下。** 可以按住按键说话，也可以切换为按一下模式：按一次开始，再按一次停止。

### 提问

<div align="center">
<img src="assets/readme/ask.webp" width="720" alt="快速提问的三个示例：把选中的消息翻译成西班牙语并替换，写一条回复并插入到光标处，以及解释选中的句子" />
</div>

选中一些文字，或者什么都不选。按住 <kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd>，说出你想要的：“把这段翻译成西班牙语”“写一条礼貌的回复，说我周四去不了”“解释一下这个”。答案会以卡片形式实时显示在当前应用上方，你可以复制、插入到光标处，或者用它替换选中的文字。

如果你允许，它还能做更多：

- **屏幕视觉。** 问问终端里的报错，或者表格里的图表是什么意思。这项功能默认关闭；开启后，也由模型针对每个问题判断是否需要查看屏幕。没用上的截图不会离开你的电脑。
- **网络搜索**，支持 Serper、Brave、Tavily、Exa、SerpAPI 或 TinyFish。
- **提醒。** “二十分钟后提醒我发发票。”提醒在重启后依然有效，弹出时也不会抢走你的键盘焦点。
- **角色与记忆。** 给它设置不同的角色，每个角色有自己的回答长度，还可以让它记住你的工作习惯。记忆默认关闭，只保存在你的电脑上，你可以随时编辑或清除。

### 对话

按下 <kbd>Left Ctrl</kbd> + <kbd>Left Alt</kbd> + <kbd>C</kbd> 然后开口说话。助手会出声回答，你也可以在它说话时随时插话。Esc 会停止当前回答但不结束对话，再按一次这组按键则结束对话。中途需要输入点什么？照常使用语音输入就行，对话会暂停等你，结束后再继续。

回答可以由你电脑上的语音朗读（Kokoro、Kitten、Pocket TTS 或 Supertonic），也可以使用 OpenAI、ElevenLabs、Deepgram、Cartesia、Google、Azure 等云端语音。

### 会议 <sup>beta</sup>

<table>
<tr>
<td width="50%"><img src="assets/readme/screens/meetings-live.webp" alt="正在录制的会议，实时字幕区分了你说的话和对方说的话" /></td>
<td width="50%"><img src="assets/readme/screens/meeting-notes.webp" alt="会后生成的纪要：摘要、要点、议题、决定，以及带负责人的后续事项" /></td>
</tr>
</table>

开会前点击“开始录制”。SpeakoFlow 会把你的麦克风和电脑播放的声音分成两路录制，并在大家说话时实时转写，所以它始终知道哪些话是你说的。不会有机器人加入会议，任何会议软件都能用。在 Windows 上，它检测到通话时还会主动询问是否录制。

通话结束后，它会按你选择的模板（通用、站会、一对一、面试或待办事项）写出纪要，包括摘要、决定和带负责人的后续事项。其他人的声音会标注为“发言人 1”“发言人 2”等。之后你可以就这场会议提问，也可以选择“讨论此会议”，在对话中把它聊一遍。

在 macOS 上，要录制通话中对方的声音，需要 BlackHole 之类的虚拟音频设备。详见[故障排除](#故障排除)。

### AI 清理

SpeakoFlow Mini 是我们为一项任务专门训练的小模型：把你说的话整理成干净的文字。它会去掉口头禅和语气词，修正语法和标点，并执行口头修改指令，所以当你说“scratch that”或“actually, eleven”时，它会照你的意思改，而不是把这些话原样打出来。模型下载大小为 795 MB，在你的电脑上运行，目前只支持英语。也可以改用其他任何本地或云端模型来做这件事，在 Apple 芯片的 Mac 上还可以使用 Apple Intelligence。

AI 清理默认关闭。开启后，它可以有自己的快捷键（语音输入的按键加 Shift），也可以在每次语音输入时自动运行。在此基础上，你还可以叠加一种写作风格：专业、友好、简洁、正式、随意，或者你自己写的风格。

### 其他功能

- **统计。** 已口述的字数、你的语速、相比每分钟 40 词的打字速度节省的时间，以及六个月的活跃记录和连续天数。
- **历史记录。** 你的语音输入、提问和对话都在这里。可以回放录音、重新转写、恢复被取消的录音，或者把一次聊天接着变成语音对话。旧录音可以按数量、天数或月数自动删除。
- **使用已有的模型。** 添加一个 `.gguf` 或 Whisper `.bin` 文件，或者关联一个文件夹，里面的所有模型都会出现在应用里。文件不会被复制或移动。需要下载的模型会分八段同时下载，中断后从断点继续。
- **用 Flow 生成。** 以 “Hey Flow” 开头开始口述，描述你想写的内容，粘贴出来的会是写好的文字，而不是你的原话。默认关闭，在“设置 → 语音输入”中开启。
- **20 种界面语言。**

每项功能在[使用文档](https://www.speakoflow.com/docs)中都有单独的页面。

## 看看界面

<table>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/home.webp" alt="主页：所有快捷键及“按住说话/按一下切换”开关、负责各项任务的模型，以及最近的语音输入" /><br /><sub><b>主页。</b> 你的快捷键、负责各项任务的模型，以及最近口述的内容。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/assistant.webp" alt="助手页面：提问和对话的快捷键、助手使用的模型、它的声音，以及屏幕视觉和网络搜索的开关" /><br /><sub><b>助手。</b> 它的模型、它的声音，以及允许它做什么。</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/cleanup.webp" alt="AI 清理页面：它的快捷键、作为清理模型的 SpeakoFlow Mini，以及带前后对比示例的写作风格" /><br /><sub><b>AI 清理。</b> 随口说，得到整洁的文字，风格由你挑选。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/insights.webp" alt="统计页面：口述字数、每分钟词数、节省的时间、语音输入次数，以及六个月的活跃图" /><br /><sub><b>统计。</b> 你口述了多少，省下了多少打字时间。</sub></td>
</tr>
<tr>
<td width="50%" valign="top"><img src="assets/readme/screens/models.webp" alt="在本机运行的语音转文字模型，Parakeet 正在使用，另有更多模型可供下载" /><br /><sub><b>模型。</b> 每项任务都可以在本机或云端运行。</sub></td>
<td width="50%" valign="top"><img src="assets/readme/screens/models-voice.webp" alt="语音设置：已选择并在本机就绪的 Kokoro，旁边是其他本地语音和十几种云端语音" /><br /><sub><b>语音。</b> 四种本地语音和十几种云端语音。</sub></td>
</tr>
</table>

## 键盘快捷键

| 操作                        | Windows                        | macOS           | Linux                  |
| --------------------------- | ------------------------------ | --------------- | ---------------------- |
| 语音输入                    | `Left Ctrl + Left Win`         | `Fn` (🌐)       | `Ctrl + Space`         |
| 向助手提问                  | `Left Ctrl + Left Alt`         | `Fn + Ctrl`     | `Ctrl + Alt + Space`   |
| 开始或结束对话              | `Left Ctrl + Left Alt + C`     | `Fn + Ctrl + C` | `Ctrl + Alt + C`       |
| 语音输入并清理 <sup>1</sup> | `Left Ctrl + Left Win + Shift` | `Fn + Shift`    | `Ctrl + Shift + Space` |
| 取消                        | `Esc`                          | `Esc`           | 暂不支持               |

<sup>1</sup> 仅在 AI 清理已开启并设置了独立快捷键时可用。

各平台的规律相同：语音输入的按键加 Shift 就是语音输入并清理，提问的按键加 C 就是开始对话。录音快捷键默认需要按住使用；在主页把“按住说话”切换为“按一下切换”后，按一次开始，再按一次停止。

Esc 只在有任务进行时（例如正在录音，或正在朗读回答）才会取消，其余时间其他应用照常使用 Esc。要修改快捷键，点击它的按键即可。取消和对话这两个快捷键也可以在那里关闭。

在 Mac 上，请把“系统设置 → 键盘 → 按下 🌐 键时”设为“不执行任何操作”，否则按地球键时还会打开表情选择器。之前使用旧版 Option + Space 默认快捷键的 Mac，更新后会保留原来的设置。

脚本和窗口管理器可以通过 `--toggle-transcription` 等[命令行参数](https://www.speakoflow.com/docs/settings/cli)控制 SpeakoFlow。

## 模型与服务商

每项任务都可以在你的电脑上运行，也可以交给你选择的服务商。云端服务商使用你自己的 API 密钥，密钥保存在系统钥匙串中。

| 任务             | 在你的电脑上                                                                                                                                   | 在云端（使用你的密钥）                                                                                                                                                                           |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| 语音转文字       | Parakeet、Nemotron、Canary、Cohere Transcribe、Whisper、Moonshine、Voxtral、Qwen3-ASR、GigaAM、Granite Speech、SenseVoice 等（目录中共 65 个） | ElevenLabs、Deepgram、OpenAI、Groq、Mistral (Voxtral)、Azure AI Speech、OpenRouter，或任何兼容 OpenAI 的服务器                                                                                   |
| 助手与清理       | 内置引擎（llama.cpp，完全离线）、Ollama、LM Studio、用于清理的 SpeakoFlow Mini、Apple 芯片上用于清理的 Apple Intelligence                      | OpenAI、Anthropic、Google Gemini、Azure OpenAI、AWS Bedrock、OpenRouter、Groq、Cerebras、xAI、DeepSeek、Mistral、Moonshot、Together AI、Fireworks AI、Perplexity、Z.AI，或任何兼容 OpenAI 的接口 |
| 语音回答         | Kokoro、Kitten、Pocket TTS、Supertonic                                                                                                         | OpenAI、ElevenLabs、OpenRouter、Deepgram、Cartesia、Google Cloud、Azure AI Speech、Groq、xAI、Mistral、Inworld，或你自己的服务器                                                                 |
| 网络搜索（可选） |                                                                                                                                                | Serper、Brave、Tavily、Exa、SerpAPI、TinyFish                                                                                                                                                    |

## 隐私

默认情况下，你的语音在你的电脑上转写，不会上传。没有遥测，没有数据分析，也不需要账号。

只有在你自己设置了以下功能时，数据才会离开你的电脑：

- **云端语音服务**：如果你选择它来代替本地模型。它会收到你的录音。
- **助手的服务商**：如果它不是本地服务商。它会收到你的问题、你选中的文字、开启屏幕视觉且模型请求时的截图，以及你生成会议纪要或就会议提问时的会议文字记录。
- **网络搜索**：如果你开启了它。搜索服务商会收到搜索词。
- **反馈**：如果你在应用内发送反馈。发送的内容与对话框中显示的完全一致。

API 密钥保存在系统钥匙串中。记忆在你开启之前一直是关闭的，并且只保存在你的电脑上，你可以查看、编辑或清除。更多细节见[隐私页面](https://www.speakoflow.com/docs/reference/privacy)。

## 安装

从 [Releases](https://github.com/AbhishekBarali/SpeakoFlow/releases) 页面下载最新版本。首次启动时你需要选择一个语音模型，模型下载期间会有一段简短的导览介绍快捷键。

### Windows

运行 `.exe` 安装程序。由于安装程序还没有由知名发行者签名，Windows 可能会显示 SmartScreen 提示；请选择 **More info → Run anyway**（“更多信息 → 仍要运行”）。

### macOS

下载适合你 Mac 的 `.dmg`（Apple 芯片选 `aarch64`，Intel 选 `x64`），把 **SpeakoFlow** 拖进“应用程序”。应用还没有经过 Apple 签名，所以 macOS 会提示它“已损坏，无法打开”。它并没有损坏。在“终端”中运行下面这条命令解除一次限制，之后就能正常打开：

```bash
xattr -dr com.apple.quarantine /Applications/SpeakoFlow.app
```

随后 SpeakoFlow 会请求**麦克风**和**辅助功能**权限，这样它才能听到你说话，并在其他应用中输入文字。

<details>
<summary>关于 macOS 安装的更多说明</summary>

<br />

对于无法追溯到付费 Apple 开发者账号的应用，macOS 都会显示“已损坏”的提示。签名每年需要 99 美元，本项目目前还没有这笔预算。macOS 15 及以后的版本取消了以前右键 → **打开** 的绕过方式，而且这条提示是唯一一种在“系统设置”中不提供“仍要打开”按钮的情况，所以只能通过终端处理。这条命令会去掉该应用副本上“从互联网下载”的标记。

每下载一次需要运行一次。在应用内安装的更新不带这个标记，所以不需要再运行。如果你手动下载了新的 `.dmg`，则需要对那个副本再运行一次。

更新后，macOS 有时会在“辅助功能”“麦克风”或“屏幕录制”中继续显示 SpeakoFlow 已获授权，但实际上不再生效。如果某个权限页面一直在等待，请点击其中的“重置权限”按钮，然后在“系统设置”中重新开启 SpeakoFlow。

Intel 版本需要 macOS 14 Sonoma 或更高版本。它只使用处理器运行，所以转写速度比 Apple 芯片慢，但所有功能都可用。每个 Intel 版本发布前，CI 都会在真实的 Intel Mac 上启动测试。

</details>

### Linux

- **Arch Linux。** 从 AUR 安装 `speakoflow-bin`，例如 `yay -S speakoflow-bin`。
- **Debian 13+、Ubuntu 24.04+、Mint 22+、Pop!\_OS。** 安装 `.deb`，它还会添加应用图标和菜单项：
  ```bash
  sudo apt install ./SpeakoFlow_*_amd64.deb
  ```
- **其他发行版，包括 Fedora 和 openSUSE。** 使用 AppImage：用 `chmod +x` 赋予执行权限后运行。Gear Lever 或 AppImageLauncher 之类的工具可以把它添加到应用菜单。

两种安装包都基于 Ubuntu 24.04 构建，提供 x86_64 和 ARM64 版本，因此需要 glibc 2.39 或更新版本。这意味着 Ubuntu 22.04、Debian 12、Mint 21 以及 RHEL 9 及其衍生版无法使用。目前还没有 `.rpm`，因为现有打包方式无法正确包含语音引擎，而一个装得上却无法转写的安装包比没有更糟。

### 更新

SpeakoFlow 会在后台检查新版本，并在“设置 → 关于”中安装更新，安装前会用项目的签名密钥验证每个版本。AUR 安装包则通过你的包管理器更新。如果想在 GitHub 上收到新版本通知，请点击本页顶部的 **Watch → Custom → Releases**。

## 从源码构建

你需要 [Rust](https://rustup.rs/) 和 [Bun](https://bun.sh/)。

```bash
git clone https://github.com/AbhishekBarali/SpeakoFlow.git
cd SpeakoFlow
bun install
mkdir -p src-tauri/resources/models
curl -o src-tauri/resources/models/silero_vad_v4.onnx https://blob.handy.computer/silero_vad_v4.onnx
bun run tauri dev
```

在基于 Arch 的发行版上，`bun run install:arch` 会构建当前源码，并把它安装到 `~/.local`，同时添加桌面项和 `speak` 命令。各平台的环境配置见 [BUILD.md](BUILD.md)。

应用基于 [Tauri 2](https://tauri.app)，后端为 Rust，前端为 React 和 TypeScript。语音识别运行在 transcribe.cpp、whisper.cpp 和 ONNX Runtime 上，并使用 Silero VAD；助手和清理运行在内置的 llama.cpp 引擎或任何兼容 OpenAI 的 API 上；本地语音在应用窗口中使用 [Kokoro](https://github.com/hexgrad/kokoro)，在处理器上使用 [sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)；会议中的发言人标注使用 WeSpeaker 声纹向量。

## 对比

|          | SpeakoFlow | Wispr Flow                                    | Superwhisper                                   | Handy      |
| -------- | ---------- | --------------------------------------------- | ---------------------------------------------- | ---------- |
| 价格     | 免费       | 桌面端每周 2,000 词以内免费，之后每月 15 美元 | 有免费版，Pro 每月 8.49 美元或买断 249.99 美元 | 免费       |
| 源代码   | 开源 (MIT) | 闭源                                          | 闭源                                           | 开源 (MIT) |
| Linux    | 支持       | 不支持                                        | 不支持                                         | 支持       |
| 离线转写 | 支持       | 不支持                                        | 支持                                           | 支持       |
| AI 助手  | 有         | 有                                            | 无                                             | 无         |

<sub>价格和平台信息来自各产品官网，查阅于 2026 年 10 月。</sub>

SpeakoFlow 的语音输入核心来自 [Handy](https://github.com/cjpais/Handy)；如果你只需要语音输入，Handy 是个不错的选择。更多细节：[SpeakoFlow 与 Wispr Flow 对比](https://www.speakoflow.com/blog/speakoflow-vs-wispr-flow)，以及[免费开源的 Wispr Flow 替代品](https://www.speakoflow.com/blog/best-free-open-source-wispr-flow-alternatives)。

## 故障排除

下面是常见问题。其他问题请查看[故障排除文档](https://www.speakoflow.com/docs/reference/troubleshooting)或[提交 issue](https://github.com/AbhishekBarali/SpeakoFlow/issues)。

<details>
<summary><b>macOS：会议只录到了我这一方的声音</b></summary>

<br />

macOS 没有为应用提供直接录制电脑播放声音的方式。Windows 有 WASAPI loopback，Linux 有 PulseAudio 或 PipeWire 的监听源，而 Mac 需要在中间加一个虚拟音频设备。

安装 [BlackHole](https://github.com/ExistentialAudio/BlackHole) 之类的回环驱动，在“音频 MIDI 设置”中创建一个同时输出到扬声器和 BlackHole 的“多输出设备”，并把它设为输出设备。这样 SpeakoFlow 就能录到通话另一方的声音。你的麦克风无论如何都会被录制。

</details>

<details>
<summary><b>Linux：录音浮窗无法保持在其他应用上方</b></summary>

<br />

在 Linux 上，窗口只能通过 `wlr-layer-shell` 协议（Sway、Hyprland 等 wlroots 合成器，以及 KDE Plasma）或 X11 的“保持在上方”堆叠来浮于其他窗口之上。原生 GNOME Wayland 两者都不支持，所以 SpeakoFlow 检测到它时会改用 XWayland 运行，浮窗就能正常置顶。这不需要任何设置，X11 以及 KDE 或 wlroots 的 Wayland 也都能直接使用。

- 如果仍要强制使用原生 Wayland，请用 `SPEAKOFLOW_ALLOW_WAYLAND=1` 启动。浮窗可能无法保持置顶。
- 如果浮窗在 layer-shell 合成器下表现异常，请用 `SPEAKOFLOW_NO_GTK_LAYER_SHELL=1` 启动。

</details>

<details>
<summary><b>Linux：快捷键没有反应，日志反复出现“Permission denied”</b></summary>

<br />

如果日志中满是 `rdev grab error: ... PermissionDenied`，说明应用无法读取你的输入设备。这只影响 **handy-keys** 键盘引擎，它需要读取 `/dev/input/event*`，并要求你的用户在 `input` 组中。Linux 上默认使用的是 Tauri 引擎，所以只有手动切换过引擎才会遇到这个问题。

- 把自己加入该组，然后注销并重新登录：
  ```bash
  sudo usermod -aG input $USER
  ```
- 或者在“设置 → 高级”中把键盘引擎切回 **Tauri**。它不需要任何权限，但通过 X11 注册快捷键，所以在原生 Wayland 下，只有当某个 X11 窗口获得焦点时才能响应。

在 Wayland 上，最可靠的做法是使用桌面环境自己的快捷键。在 GNOME 或 KDE 设置中添加一个自定义快捷键，或在 Sway、Hyprland 中添加一行 `bind`，让它运行 `speakoflow --toggle-transcription`（如果是 AppImage，就写它的路径再加上同样的参数）。`--toggle-post-process`、`--toggle-assistant` 和 `--cancel` 的用法相同。

</details>

<details>
<summary><b>Linux：在触控板上双指缩放时应用崩溃</b></summary>

<br />

如果崩溃时日志中出现 `Received invalid message: 'DrawingArea_CommitTransientZoom'`，这是 WebKitGTK 的一个缺陷，影响许多基于它构建的应用，上游追踪见 [tauri#13115](https://github.com/tauri-apps/tauri/issues/13115) 和 [wry#544](https://github.com/tauri-apps/wry/issues/544)。在上游修复之前，请避免在窗口内双指缩放。把 `webkit2gtk-4.1` 更新到最新版本可能会有帮助。

</details>

## 路线图

- Windows 和 macOS 代码签名
- 更多一键安装的本地模型
- 更多社区翻译
- 针对 AI 智能体编程优化的语音输入
- 提示词助手：描述你想构建的东西，得到一份可靠的提示词
- 能替你执行操作的语音指令

## 参与贡献

欢迎参与贡献。[CONTRIBUTING.md](CONTRIBUTING.md) 介绍了如何上手，[CONTRIBUTING_TRANSLATIONS.md](CONTRIBUTING_TRANSLATIONS.md) 说明了如何翻译应用。

发现了问题或有新想法？可以在应用内使用“发送反馈”（“设置”旁边的 **?**），或者[提交 issue](https://github.com/AbhishekBarali/SpeakoFlow/issues)。

## 许可证与致谢

SpeakoFlow 以 [MIT 许可证](LICENSE)发布。

语音输入核心来自 CJ Pais 开发的 [Handy](https://github.com/cjpais/Handy)，依据 MIT 许可证使用。感谢 CJ 将它开源。助手、对话、会议、屏幕视觉、用 Flow 生成、翻译、语音回答和记忆功能由 SpeakoFlow 自行开发。

同时感谢 [Tauri](https://tauri.app)、whisper.cpp、llama.cpp、ONNX Runtime、[sherpa-onnx](https://github.com/k2-fsa/sherpa-onnx)、Silero VAD、WeSpeaker、[Kokoro](https://github.com/hexgrad/kokoro) 以及 Kyutai 的 Pocket TTS。

<div align="center">

作者：[Abhishek Barali](https://github.com/AbhishekBarali) · [speakoflow.com](https://www.speakoflow.com)

</div>
