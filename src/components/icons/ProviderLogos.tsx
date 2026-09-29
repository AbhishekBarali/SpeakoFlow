import React from "react";
import {
  AudioLines,
  AudioWaveform,
  Cat,
  Cpu,
  Earth,
  Fish,
  Mic,
  Plug,
  Search,
  SearchCode,
  Sparkles,
  Speech,
  Waves,
} from "lucide-react";
import { OpenAILogo } from "./BrandLogos";

/**
 * Brand marks for the AI providers a user picks in Settings (chat / cleanup
 * LLMs, cloud speech-to-text, text-to-speech, web search), so a provider row
 * reads as "Anthropic / Groq / Deepgram" at a glance instead of a line of
 * text. Every mark renders at `currentColor` (two opt into their own second
 * colour); the tile supplies the brand hue — see `PROVIDER_TILES`.
 *
 * Path data is copied verbatim from two open-source icon sets — never redrawn:
 *
 * - simple-icons (CC0 1.0, https://github.com/simple-icons/simple-icons):
 *   Anthropic, Google Gemini, Mistral AI, DeepSeek, OpenRouter, Perplexity,
 *   Moonshot AI, Kimi, Ollama, Apple, ElevenLabs, Deepgram, Brave.
 *   (OpenAI is reused from BrandLogos.tsx, also simple-icons.)
 * - LobeHub Icons, @lobehub/icons-static-svg 1.95.1
 *   (https://github.com/lobehub/lobe-icons), MIT License,
 *   Copyright (c) 2023 LobeHub:
 *   Groq, xAI (Grok), Cerebras, Together AI, Fireworks AI, Microsoft Azure,
 *   Microsoft (four-square mark), AWS, Z.ai, Tavily, Exa, Google Cloud.
 *
 * No open-source monochrome mark exists in either set (or svgl) for SerpApi,
 * Serper, TinyFish, Cartesia or Inworld. Those get distinct lucide glyphs (a
 * search glass, a search-with-code glass, a fish, a waveform, a globe) rather
 * than an invented logo, so no two of them render as identical tiles.
 */

interface LogoProps {
  size?: number;
  className?: string;
}

/** Shared `<svg>` shell. LobeHub marks rely on `fill-rule="evenodd"` at the
 *  root (their cut-outs disappear without it); simple-icons marks use the
 *  default nonzero rule. */
const Mark: React.FC<
  LogoProps & { evenOdd?: boolean; children: React.ReactNode }
> = ({ size = 18, className, evenOdd, children }) => (
  <svg
    role="img"
    aria-hidden
    viewBox="0 0 24 24"
    width={size}
    height={size}
    fill="currentColor"
    fillRule={evenOdd ? "evenodd" : undefined}
    className={className}
  >
    {children}
  </svg>
);

export { OpenAILogo };

export const AnthropicLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M17.3041 3.541h-3.6718l6.696 16.918H24Zm-10.6082 0L0 20.459h3.7442l1.3693-3.5527h7.0052l1.3693 3.5528h3.7442L10.5363 3.5409Zm-.3712 10.2232 2.2914-5.9456 2.2914 5.9456Z" />
  </Mark>
);

export const GeminiLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M11.04 19.32Q12 21.51 12 24q0-2.49.93-4.68.96-2.19 2.58-3.81t3.81-2.55Q21.51 12 24 12q-2.49 0-4.68-.93a12.3 12.3 0 0 1-3.81-2.58 12.3 12.3 0 0 1-2.58-3.81Q12 2.49 12 0q0 2.49-.96 4.68-.93 2.19-2.55 3.81a12.3 12.3 0 0 1-3.81 2.58Q2.49 12 0 12q2.49 0 4.68.96 2.19.93 3.81 2.55t2.55 3.81" />
  </Mark>
);

export const GroqLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path d="M12.036 2c-3.853-.035-7 3-7.036 6.781-.035 3.782 3.055 6.872 6.908 6.907h2.42v-2.566h-2.292c-2.407.028-4.38-1.866-4.408-4.23-.029-2.362 1.901-4.298 4.308-4.326h.1c2.407 0 4.358 1.915 4.365 4.278v6.305c0 2.342-1.944 4.25-4.323 4.279a4.375 4.375 0 01-3.033-1.252l-1.851 1.818A7 7 0 0012.029 22h.092c3.803-.056 6.858-3.083 6.879-6.816v-6.5C18.907 4.963 15.817 2 12.036 2z" />
  </Mark>
);

export const MistralLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M17.143 3.429v3.428h-3.429v3.429h-3.428V6.857H6.857V3.43H3.43v13.714H0v3.428h10.286v-3.428H6.857v-3.429h3.429v3.429h3.429v-3.429h3.428v3.429h-3.428v3.428H24v-3.428h-3.43V3.429z" />
  </Mark>
);

export const DeepSeekLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M23.748 4.651c-.254-.124-.364.113-.512.233-.051.04-.094.09-.137.137-.372.397-.806.657-1.373.626-.829-.046-1.537.214-2.163.848-.133-.782-.575-1.248-1.247-1.548-.352-.155-.708-.311-.955-.65-.172-.24-.219-.509-.305-.774-.055-.16-.11-.323-.293-.35-.2-.031-.278.136-.356.276-.313.572-.434 1.202-.422 1.84.027 1.436.633 2.58 1.838 3.393.137.094.172.187.129.323-.082.28-.18.553-.266.833-.055.179-.137.218-.328.14a5.5 5.5 0 0 1-1.737-1.179c-.857-.828-1.631-1.743-2.597-2.46a12 12 0 0 0-.689-.47c-.985-.957.13-1.743.387-1.836.27-.098.094-.433-.778-.428-.872.003-1.67.295-2.687.685a3 3 0 0 1-.465.136 9.6 9.6 0 0 0-2.883-.101c-1.885.21-3.39 1.1-4.497 2.622C.082 8.776-.231 10.854.152 13.02c.403 2.284 1.568 4.175 3.36 5.653 1.857 1.533 3.997 2.284 6.438 2.14 1.482-.085 3.132-.284 4.994-1.86.47.234.962.328 1.78.398.629.058 1.235-.031 1.705-.129.735-.155.684-.836.418-.961-2.155-1.004-1.682-.595-2.112-.926 1.095-1.295 2.768-3.598 3.284-6.733.05-.346.115-.834.108-1.114-.004-.171.035-.238.23-.257a4.2 4.2 0 0 0 1.545-.475c1.397-.763 1.96-2.016 2.093-3.517.02-.23-.004-.467-.247-.588M11.58 18.168c-2.088-1.642-3.101-2.183-3.52-2.16-.39.024-.32.472-.234.763.09.288.207.487.371.74.114.167.192.416-.113.603-.673.416-1.842-.14-1.897-.168-1.361-.801-2.5-1.86-3.301-3.306-.775-1.393-1.225-2.888-1.299-4.482-.02-.385.094-.522.477-.592a4.7 4.7 0 0 1 1.53-.038c2.131.311 3.946 1.264 5.467 2.774.868.86 1.525 1.887 2.202 2.89.72 1.066 1.494 2.082 2.48 2.915.348.291.626.513.892.677-.802.09-2.14.109-3.055-.615zm1.001-6.44a.306.306 0 0 1 .415-.287.3.3 0 0 1 .113.074.3.3 0 0 1 .086.214c0 .17-.136.307-.308.307a.303.303 0 0 1-.306-.307m3.11 1.596c-.2.081-.4.151-.591.16a1.25 1.25 0 0 1-.798-.254c-.274-.23-.47-.358-.551-.758a1.7 1.7 0 0 1 .015-.588c.07-.327-.007-.537-.238-.727-.188-.156-.426-.199-.689-.199a.6.6 0 0 1-.254-.078.253.253 0 0 1-.114-.358 1 1 0 0 1 .192-.21c.356-.202.767-.136 1.146.016.352.144.618.408 1.001.782.392.451.462.576.685.915.176.264.336.536.446.848.066.194-.02.353-.25.45" />
  </Mark>
);

export const XAILogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path d="M6.469 8.776L16.512 23h-4.464L2.005 8.776H6.47zm-.004 7.9l2.233 3.164L6.467 23H2l4.465-6.324zM22 2.582V23h-3.659V7.764L22 2.582zM22 1l-9.952 14.095-2.233-3.163L17.533 1H22z" />
  </Mark>
);

export const OpenRouterLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M16.778 1.844v1.919q-.569-.026-1.138-.032-.708-.008-1.415.037c-1.93.126-4.023.728-6.149 2.237-2.911 2.066-2.731 1.95-4.14 2.75-.396.223-1.342.574-2.185.798-.841.225-1.753.333-1.751.333v4.229s.768.108 1.61.333c.842.224 1.789.575 2.185.799 1.41.798 1.228.683 4.14 2.75 2.126 1.509 4.22 2.11 6.148 2.236.88.058 1.716.041 2.555.005v1.918l7.222-4.168-7.222-4.17v2.176c-.86.038-1.611.065-2.278.021-1.364-.09-2.417-.357-3.979-1.465-2.244-1.593-2.866-2.027-3.68-2.508.889-.518 1.449-.906 3.822-2.59 1.56-1.109 2.614-1.377 3.978-1.466.667-.044 1.418-.017 2.278.02v2.176L24 6.014Z" />
  </Mark>
);

export const CerebrasLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path
      clipRule="evenodd"
      d="M14.121 2.701a9.299 9.299 0 000 18.598V22.7c-5.91 0-10.7-4.791-10.7-10.701S8.21 1.299 14.12 1.299V2.7zm4.752 3.677A7.353 7.353 0 109.42 17.643l-.901 1.074a8.754 8.754 0 01-1.08-12.334 8.755 8.755 0 0112.335-1.08l-.901 1.075zm-2.255.844a5.407 5.407 0 00-5.048 9.563l-.656 1.24a6.81 6.81 0 016.358-12.043l-.654 1.24zM14.12 8.539a3.46 3.46 0 100 6.922v1.402a4.863 4.863 0 010-9.726v1.402z"
    />
    <path d="M15.407 10.836a2.24 2.24 0 00-.51-.409 1.084 1.084 0 00-.544-.152c-.255 0-.483.047-.684.14a1.58 1.58 0 00-.84.912c-.074.203-.11.416-.11.631 0 .218.036.43.11.631a1.594 1.594 0 00.84.913c.2.093.43.14.684.14.216 0 .417-.046.602-.135.188-.09.35-.225.475-.392l.928 1.006c-.14.14-.3.261-.482.363a3.367 3.367 0 01-1.083.38c-.17.026-.317.04-.44.04a3.315 3.315 0 01-1.182-.21 2.825 2.825 0 01-.961-.597 2.816 2.816 0 01-.644-.929 2.987 2.987 0 01-.238-1.21c0-.444.08-.847.238-1.21.15-.35.368-.666.643-.929.278-.261.605-.464.962-.596a3.315 3.315 0 011.182-.21c.355 0 .712.068 1.072.204.361.138.685.36.944.649l-.962.97z" />
  </Mark>
);

export const TogetherLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path d="M23.197 4.503A6 6 0 0015 2.307a5.973 5.973 0 00-2.995 4.933l5.996.008v.515h-5.996c.039.937.298 1.87.8 2.74a6 6 0 1010.39-6z" />
    <path d="M.805 4.5A6 6 0 003 12.697a5.972 5.972 0 005.77.127L5.779 7.627l.446-.257 2.997 5.192A6 6 0 10.804 4.5z" />
    <path d="M12 23.894a6 6 0 005.999-6c0-2.13-1.1-3.996-2.775-5.06l-3.005 5.189-.444-.258 2.997-5.192A6 6 0 1012 23.894z" />
  </Mark>
);

export const FireworksLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path
      clipRule="evenodd"
      d="M14.8 5l-2.801 6.795L9.195 5H7.397l3.072 7.428a1.64 1.64 0 003.038.002L16.598 5H14.8zm1.196 10.352l5.124-5.244-.699-1.669-5.596 5.739a1.664 1.664 0 00-.343 1.807 1.642 1.642 0 001.516 1.012L16 17l8-.02-.699-1.669-7.303.041h-.002zM2.88 10.104l.699-1.669 5.596 5.739c.468.479.603 1.189.343 1.807a1.643 1.643 0 01-1.516 1.012l-8-.018-.002.002.699-1.669 7.303.042-5.122-5.246z"
    />
  </Mark>
);

export const PerplexityLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M22.3977 7.0896h-2.3106V.0676l-7.5094 6.3542V.1577h-1.1554v6.1966L4.4904 0v7.0896H1.6023v10.3976h2.8882V24l6.932-6.3591v6.2005h1.1554v-6.0469l6.9318 6.1807v-6.4879h2.8882V7.0896zm-3.4657-4.531v4.531h-5.355l5.355-4.531zm-13.2862.0676 4.8691 4.4634H5.6458V2.6262zM2.7576 16.332V8.245h7.8476l-6.1149 6.1147v1.9723H2.7576zm2.8882 5.0404v-3.8852h.0001v-2.6488l5.7763-5.7764v7.0111l-5.7764 5.2993zm12.7086.0248-5.7766-5.1509V9.0618l5.7766 5.7766v6.5588zm2.8882-5.0652h-1.733v-1.9723L13.3948 8.245h7.8478v8.087z" />
  </Mark>
);

export const AzureLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path
      d="M18.397 15.296H7.4a.51.51 0 00-.347.882l7.066 6.595c.206.192.477.298.758.298h6.226l-2.706-7.775z"
      fillOpacity=".75"
    />
    <path
      d="M8.295.857c-.477 0-.9.304-1.053.756L.495 21.605a1.11 1.11 0 001.052 1.466h5.43c.477 0 .9-.304 1.053-.755l1.341-3.975-2.318-2.163a.51.51 0 01.347-.882h3L15.271.857H8.295z"
      fillOpacity=".5"
    />
    <path d="M17.193 1.613a1.11 1.11 0 00-1.052-.756h-7.81.035c.477 0 .9.304 1.052.756l6.748 19.992a1.11 1.11 0 01-1.052 1.466h-.12 7.895a1.11 1.11 0 001.052-1.466L17.193 1.613z" />
  </Mark>
);

/** Microsoft's four-square mark — used for Azure AI Speech. `multicolor`
 *  draws it in its own four colours instead of `currentColor`. */
export const MicrosoftLogo: React.FC<LogoProps & { multicolor?: boolean }> = ({
  multicolor,
  ...p
}) => (
  <Mark {...p} evenOdd>
    <path
      d="M11.49 2H2v9.492h9.492V2h-.002z"
      fill={multicolor ? "#F25022" : undefined}
    />
    <path
      d="M22 2h-9.492v9.492H22V2z"
      fill={multicolor ? "#7FBA00" : undefined}
    />
    <path
      d="M11.49 12.508H2V22h9.492v-9.492h-.002z"
      fill={multicolor ? "#00A4EF" : undefined}
    />
    <path
      d="M22 12.508h-9.492V22H22v-9.492z"
      fill={multicolor ? "#FFB900" : undefined}
    />
  </Mark>
);

/** AWS wordmark and smile. `smile` colours the arrow on its own, the way the
 *  real mark draws it (white type, orange smile). */
export const AWSLogo: React.FC<LogoProps & { smile?: string }> = ({
  smile,
  ...p
}) => (
  <Mark {...p} evenOdd>
    <path d="M6.763 11.212c0 .296.032.535.088.71.064.176.144.368.256.576.04.063.056.127.056.183 0 .08-.048.16-.152.24l-.503.335a.383.383 0 01-.208.072c-.08 0-.16-.04-.239-.112a2.47 2.47 0 01-.287-.375 6.18 6.18 0 01-.248-.471c-.622.734-1.405 1.101-2.347 1.101-.67 0-1.205-.191-1.596-.574-.39-.384-.59-.894-.59-1.533 0-.678.24-1.23.726-1.644.487-.415 1.133-.623 1.955-.623.272 0 .551.024.846.064.296.04.6.104.918.176v-.583c0-.607-.127-1.03-.375-1.277-.255-.248-.686-.367-1.3-.367-.28 0-.568.031-.863.103-.295.072-.583.16-.862.272-.09.04-.184.075-.28.104a.488.488 0 01-.127.023c-.112 0-.168-.08-.168-.247v-.391c0-.128.016-.224.056-.28a.597.597 0 01.224-.167 4.577 4.577 0 011.005-.36 4.84 4.84 0 011.246-.151c.95 0 1.644.216 2.091.647.44.43.662 1.085.662 1.963v2.586h.016zm-3.24 1.214c.263 0 .534-.048.822-.144a1.78 1.78 0 00.758-.51 1.27 1.27 0 00.272-.512c.047-.191.08-.423.08-.694v-.335a6.66 6.66 0 00-.735-.136 6.02 6.02 0 00-.75-.048c-.535 0-.926.104-1.19.32-.263.215-.39.518-.39.917 0 .375.095.655.295.846.191.2.47.296.838.296zm6.41.862c-.144 0-.24-.024-.304-.08-.064-.048-.12-.16-.168-.311L7.586 6.726a1.398 1.398 0 01-.072-.32c0-.128.064-.2.191-.2h.783c.151 0 .255.025.31.08.065.048.113.16.16.312l1.342 5.284 1.245-5.284c.04-.16.088-.264.151-.312a.549.549 0 01.32-.08h.638c.152 0 .256.025.32.08.063.048.12.16.151.312l1.261 5.348 1.381-5.348c.048-.16.104-.264.16-.312a.52.52 0 01.311-.08h.743c.127 0 .2.065.2.2 0 .04-.009.08-.017.128a1.137 1.137 0 01-.056.2l-1.923 6.17c-.048.16-.104.263-.168.311a.51.51 0 01-.303.08h-.687c-.15 0-.255-.024-.32-.08-.063-.056-.119-.16-.15-.32L12.32 7.747l-1.23 5.14c-.04.16-.087.264-.15.32-.065.056-.177.08-.32.08l-.686.001zm10.256.215c-.415 0-.83-.048-1.229-.143-.399-.096-.71-.2-.918-.32-.128-.071-.215-.151-.247-.223a.563.563 0 01-.048-.224v-.407c0-.167.064-.247.183-.247.048 0 .096.008.144.024.048.016.12.048.2.08.271.12.566.215.878.279.32.064.63.096.95.096.502 0 .894-.088 1.165-.264a.86.86 0 00.415-.758.777.777 0 00-.215-.559c-.144-.151-.416-.287-.807-.415l-1.157-.36c-.583-.183-1.014-.454-1.277-.813a1.902 1.902 0 01-.4-1.158c0-.335.073-.63.216-.886.144-.255.335-.479.575-.654.24-.184.51-.32.83-.415.32-.096.655-.136 1.006-.136.175 0 .36.008.535.032.183.024.35.056.518.088.16.04.312.08.455.127.144.048.256.096.336.144a.69.69 0 01.24.2.43.43 0 01.071.263v.375c0 .168-.064.256-.184.256a.83.83 0 01-.303-.096 3.652 3.652 0 00-1.532-.311c-.455 0-.815.071-1.062.223-.248.152-.375.383-.375.71 0 .224.08.416.24.567.16.152.454.304.877.44l1.134.358c.574.184.99.44 1.237.767.247.327.367.702.367 1.117 0 .343-.072.655-.207.926a2.157 2.157 0 01-.583.703c-.248.2-.543.343-.886.447-.36.111-.734.167-1.142.167z" />
    <path
      fill={smile}
      d="M.378 15.475c3.384 1.963 7.56 3.153 11.877 3.153 2.914 0 6.114-.607 9.06-1.852.44-.2.814.287.383.607-2.626 1.94-6.442 2.969-9.722 2.969-4.598 0-8.74-1.7-11.87-4.526-.247-.223-.024-.527.272-.351zm23.531-.2c.287.36-.08 2.826-1.485 4.007-.215.184-.423.088-.327-.151l.175-.439c.343-.88.802-2.198.52-2.555-.336-.43-2.22-.207-3.074-.103-.255.032-.295-.192-.063-.36 1.5-1.053 3.967-.75 4.254-.399z"
    />
  </Mark>
);

export const MoonshotLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="m1.053 16.91 9.538 2.55a21 20.981 0 0 0 .06 2.031l5.956 1.592a12 11.99 0 0 1-15.554-6.172m-1.02-5.79 11.352 3.035a21 20.981 0 0 0-.469 2.01l10.817 2.89a12 11.99 0 0 1-1.845 2.004L.658 15.918a12 11.99 0 0 1-.625-4.796m1.593-5.146L13.573 9.17a21 20.981 0 0 0-1.01 1.874l11.297 3.02a21 20.981 0 0 1-.67 2.362l-11.55-3.087L.125 10.26a12 11.99 0 0 1 1.499-4.285ZM6.067 1.58l11.285 3.016a21 20.981 0 0 0-1.688 1.719l7.824 2.091a21 20.981 0 0 1 .513 2.664L2.107 5.218a12 11.99 0 0 1 3.96-3.638M21.68 4.866 7.222 1.003A12 11.99 0 0 1 21.68 4.866" />
  </Mark>
);

export const KimiLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M21.765.351C22.998.351 24 1.353 24 2.586S22.998 4.82 21.765 4.82h-1.974c-.15 0-.26-.12-.26-.26V2.586A2.237 2.237 0 0 1 21.765.35M9.41 13.388l8.447-8.377c.16-.16.07-.471-.14-.471h-4.55s-.1.02-.14.06l-9.099 9.029c-.14.14-.35.02-.35-.21V4.81c0-.15-.1-.27-.221-.27H.22c-.12 0-.22.12-.22.27v18.57c0 .15.1.27.22.27h3.137c.12 0 .22-.12.22-.27v-3.79c0-.08.03-.16.08-.21l2.826-2.796c.07-.07.16-.08.241-.03l7.546 5.551a8.9 8.9 0 0 0 4.018 1.493c.12.01.23-.11.23-.27V19.76c0-.14-.08-.25-.19-.26a5.8 5.8 0 0 1-2.355-.942l-6.533-4.73c-.14-.09-.15-.32-.03-.441" />
  </Mark>
);

export const ZAILogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path d="M12.105 2L9.927 4.953H.653L2.83 2h9.276zM23.254 19.048L21.078 22h-9.242l2.174-2.952h9.244zM24 2L9.264 22H0L14.736 2H24z" />
  </Mark>
);

export const OllamaLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M16.361 10.26a.894.894 0 0 0-.558.47l-.072.148.001.207c0 .193.004.217.059.353.076.193.152.312.291.448.24.238.51.3.872.205a.86.86 0 0 0 .517-.436.752.752 0 0 0 .08-.498c-.064-.453-.33-.782-.724-.897a1.06 1.06 0 0 0-.466 0zm-9.203.005c-.305.096-.533.32-.65.639a1.187 1.187 0 0 0-.06.52c.057.309.31.59.598.667.362.095.632.033.872-.205.14-.136.215-.255.291-.448.055-.136.059-.16.059-.353l.001-.207-.072-.148a.894.894 0 0 0-.565-.472 1.02 1.02 0 0 0-.474.007Zm4.184 2c-.131.071-.223.25-.195.383.031.143.157.288.353.407.105.063.112.072.117.136.004.038-.01.146-.029.243-.02.094-.036.194-.036.222.002.074.07.195.143.253.064.052.076.054.255.059.164.005.198.001.264-.03.169-.082.212-.234.15-.525-.052-.243-.042-.28.087-.355.137-.08.281-.219.324-.314a.365.365 0 0 0-.175-.48.394.394 0 0 0-.181-.033c-.126 0-.207.03-.355.124l-.085.053-.053-.032c-.219-.13-.259-.145-.391-.143a.396.396 0 0 0-.193.032zm.39-2.195c-.373.036-.475.05-.654.086-.291.06-.68.195-.951.328-.94.46-1.589 1.226-1.787 2.114-.04.176-.045.234-.045.53 0 .294.005.357.043.524.264 1.16 1.332 2.017 2.714 2.173.3.033 1.596.033 1.896 0 1.11-.125 2.064-.727 2.493-1.571.114-.226.169-.372.22-.602.039-.167.044-.23.044-.523 0-.297-.005-.355-.045-.531-.288-1.29-1.539-2.304-3.072-2.497a6.873 6.873 0 0 0-.855-.031zm.645.937a3.283 3.283 0 0 1 1.44.514c.223.148.537.458.671.662.166.251.26.508.303.82.02.143.01.251-.043.482-.08.345-.332.705-.672.957a3.115 3.115 0 0 1-.689.348c-.382.122-.632.144-1.525.138-.582-.006-.686-.01-.853-.042-.57-.107-1.022-.334-1.35-.68-.264-.28-.385-.535-.45-.946-.03-.192.025-.509.137-.776.136-.326.488-.73.836-.963.403-.269.934-.46 1.422-.512.187-.02.586-.02.773-.002zm-5.503-11a1.653 1.653 0 0 0-.683.298C5.617.74 5.173 1.666 4.985 2.819c-.07.436-.119 1.04-.119 1.503 0 .544.064 1.24.155 1.721.02.107.031.202.023.208a8.12 8.12 0 0 1-.187.152 5.324 5.324 0 0 0-.949 1.02 5.49 5.49 0 0 0-.94 2.339 6.625 6.625 0 0 0-.023 1.357c.091.78.325 1.438.727 2.04l.13.195-.037.064c-.269.452-.498 1.105-.605 1.732-.084.496-.095.629-.095 1.294 0 .67.009.803.088 1.266.095.555.288 1.143.503 1.534.071.128.243.393.264.407.007.003-.014.067-.046.141a7.405 7.405 0 0 0-.548 1.873c-.062.417-.071.552-.071.991 0 .56.031.832.148 1.279L3.42 24h1.478l-.05-.091c-.297-.552-.325-1.575-.068-2.597.117-.472.25-.819.498-1.296l.148-.29v-.177c0-.165-.003-.184-.057-.293a.915.915 0 0 0-.194-.25 1.74 1.74 0 0 1-.385-.543c-.424-.92-.506-2.286-.208-3.451.124-.486.329-.918.544-1.154a.787.787 0 0 0 .223-.531c0-.195-.07-.355-.224-.522a3.136 3.136 0 0 1-.817-1.729c-.14-.96.114-2.005.69-2.834.563-.814 1.353-1.336 2.237-1.475.199-.033.57-.028.776.01.226.04.367.028.512-.041.179-.085.268-.19.374-.431.093-.215.165-.333.36-.576.234-.29.46-.489.822-.729.413-.27.884-.467 1.352-.561.17-.035.25-.04.569-.04.319 0 .398.005.569.04a4.07 4.07 0 0 1 1.914.997c.117.109.398.457.488.602.034.057.095.177.132.267.105.241.195.346.374.43.14.068.286.082.503.045.343-.058.607-.053.943.016 1.144.23 2.14 1.173 2.581 2.437.385 1.108.276 2.267-.296 3.153-.097.15-.193.27-.333.419-.301.322-.301.722-.001 1.053.493.539.801 1.866.708 3.036-.062.772-.26 1.463-.533 1.854a2.096 2.096 0 0 1-.224.258.916.916 0 0 0-.194.25c-.054.109-.057.128-.057.293v.178l.148.29c.248.476.38.823.498 1.295.253 1.008.231 2.01-.059 2.581a.845.845 0 0 0-.044.098c0 .006.329.009.732.009h.73l.02-.074.036-.134c.019-.076.057-.3.088-.516.029-.217.029-1.016 0-1.258-.11-.875-.295-1.57-.597-2.226-.032-.074-.053-.138-.046-.141.008-.005.057-.074.108-.152.376-.569.607-1.284.724-2.228.031-.26.031-1.378 0-1.628-.083-.645-.182-1.082-.348-1.525a6.083 6.083 0 0 0-.329-.7l-.038-.064.131-.194c.402-.604.636-1.262.727-2.04a6.625 6.625 0 0 0-.024-1.358 5.512 5.512 0 0 0-.939-2.339 5.325 5.325 0 0 0-.95-1.02 8.097 8.097 0 0 1-.186-.152.692.692 0 0 1 .023-.208c.208-1.087.201-2.443-.017-3.503-.19-.924-.535-1.658-.98-2.082-.354-.338-.716-.482-1.15-.455-.996.059-1.8 1.205-2.116 3.01a6.805 6.805 0 0 0-.097.726c0 .036-.007.066-.015.066a.96.96 0 0 1-.149-.078A4.857 4.857 0 0 0 12 3.03c-.832 0-1.687.243-2.456.698a.958.958 0 0 1-.148.078c-.008 0-.015-.03-.015-.066a6.71 6.71 0 0 0-.097-.725C8.997 1.392 8.337.319 7.46.048a2.096 2.096 0 0 0-.585-.041Zm.293 1.402c.248.197.523.759.682 1.388.03.113.06.244.069.292.007.047.026.152.041.233.067.365.098.76.102 1.24l.002.475-.12.175-.118.178h-.278c-.324 0-.646.041-.954.124l-.238.06c-.033.007-.038-.003-.057-.144a8.438 8.438 0 0 1 .016-2.323c.124-.788.413-1.501.696-1.711.067-.05.079-.049.157.013zm9.825-.012c.17.126.358.46.498.888.28.854.36 2.028.212 3.145-.019.14-.024.151-.057.144l-.238-.06a3.693 3.693 0 0 0-.954-.124h-.278l-.119-.178-.119-.175.002-.474c.004-.669.066-1.19.214-1.772.157-.623.434-1.185.68-1.382.078-.062.09-.063.159-.012z" />
  </Mark>
);

export const AppleLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M12.152 6.896c-.948 0-2.415-1.078-3.96-1.04-2.04.027-3.91 1.183-4.961 3.014-2.117 3.675-.546 9.103 1.519 12.09 1.013 1.454 2.208 3.09 3.792 3.039 1.52-.065 2.09-.987 3.935-.987 1.831 0 2.35.987 3.96.948 1.637-.026 2.676-1.48 3.676-2.948 1.156-1.688 1.636-3.325 1.662-3.415-.039-.013-3.182-1.221-3.22-4.857-.026-3.04 2.48-4.494 2.597-4.559-1.429-2.09-3.623-2.324-4.39-2.376-2-.156-3.675 1.09-4.61 1.09zM15.53 3.83c.843-1.012 1.4-2.427 1.245-3.83-1.207.052-2.662.805-3.532 1.818-.78.896-1.454 2.338-1.273 3.714 1.338.104 2.715-.688 3.559-1.701" />
  </Mark>
);

export const ElevenLabsLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M4.6035 0v24h4.9317V0zm9.8613 0v24h4.9317V0z" />
  </Mark>
);

export const DeepgramLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M11.203 24H1.517a.364.364 0 0 1-.258-.62l6.239-6.275a.366.366 0 0 1 .259-.108h3.52c2.723 0 5.025-2.127 5.107-4.845a5.004 5.004 0 0 0-4.999-5.148H7.613v4.646c0 .2-.164.364-.365.364H.968a.365.365 0 0 1-.363-.364V.364C.605.164.768 0 .969 0h10.416c6.684 0 12.111 5.485 12.01 12.187C23.293 18.77 17.794 24 11.202 24z" />
  </Mark>
);

export const BraveLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p}>
    <path d="M15.68 0l2.096 2.38s1.84-.512 2.709.358c.868.87 1.584 1.638 1.584 1.638l-.562 1.381.715 2.047s-2.104 7.98-2.35 8.955c-.486 1.919-.818 2.66-2.198 3.633-1.38.972-3.884 2.66-4.293 2.916-.409.256-.92.692-1.38.692-.46 0-.97-.436-1.38-.692a185.796 185.796 0 01-4.293-2.916c-1.38-.973-1.712-1.714-2.197-3.633-.247-.975-2.351-8.955-2.351-8.955l.715-2.047-.562-1.381s.716-.768 1.585-1.638c.868-.87 2.708-.358 2.708-.358L8.321 0h7.36zm-3.679 14.936c-.14 0-1.038.317-1.758.69-.72.373-1.242.637-1.409.742-.167.104-.065.301.087.409.152.107 2.194 1.69 2.393 1.866.198.175.489.464.687.464.198 0 .49-.29.688-.464.198-.175 2.24-1.759 2.392-1.866.152-.108.254-.305.087-.41-.167-.104-.689-.368-1.41-.741-.72-.373-1.617-.69-1.757-.69zm0-11.278s-.409.001-1.022.206-1.278.46-1.584.46c-.307 0-2.581-.434-2.581-.434S4.119 7.152 4.119 7.849c0 .697.339.881.68 1.243l2.02 2.149c.192.203.59.511.356 1.066-.235.555-.58 1.26-.196 1.977.384.716 1.042 1.194 1.464 1.115.421-.08 1.412-.598 1.776-.834.364-.237 1.518-1.19 1.518-1.554 0-.365-1.193-1.02-1.413-1.168-.22-.15-1.226-.725-1.247-.95-.02-.227-.012-.293.284-.851.297-.559.831-1.304.742-1.8-.089-.495-.95-.753-1.565-.986-.615-.232-1.799-.671-1.947-.74-.148-.068-.11-.133.339-.175.448-.043 1.719-.212 2.292-.052.573.16 1.552.403 1.632.532.079.13.149.134.067.579-.081.445-.5 2.581-.541 2.96-.04.38-.12.63.288.724.409.094 1.097.256 1.333.256s.924-.162 1.333-.256c.408-.093.329-.344.288-.723-.04-.38-.46-2.516-.541-2.961-.082-.445-.012-.45.067-.579.08-.129 1.059-.372 1.632-.532.573-.16 1.845.009 2.292.052.449.042.487.107.339.175-.148.069-1.332.508-1.947.74-.615.233-1.476.49-1.565.986-.09.496.445 1.241.742 1.8.297.558.304.624.284.85-.02.226-1.026.802-1.247.95-.22.15-1.413.804-1.413 1.169 0 .364 1.154 1.317 1.518 1.554.364.236 1.355.755 1.776.834.422.079 1.08-.4 1.464-1.115.384-.716.039-1.422-.195-1.977-.235-.555.163-.863.355-1.066l2.02-2.149c.341-.362.68-.546.68-1.243 0-.697-2.695-3.96-2.695-3.96s-2.274.436-2.58.436c-.307 0-.972-.256-1.585-.461-.613-.205-1.022-.206-1.022-.206z" />
  </Mark>
);

export const TavilyLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path d="M8.033 14.273a1.612 1.612 0 011.139.47l.04.042.044.043a1.61 1.61 0 010 2.277l-3.073 3.073.816.816c.6.6.303 1.627-.525 1.814l-5.159 1.165a1.07 1.07 0 01-.897-.2l-.102-.09a1.07 1.07 0 01-.289-1l1.164-5.158A1.079 1.079 0 013.006 17l.816.817 3.074-3.074a1.612 1.612 0 011.137-.47zM17.042 13.246c0-.85.935-1.366 1.653-.912l4.47 2.824c.336.212.503.562.503.911 0 .35-.167.7-.501.913l-4.472 2.824a1.079 1.079 0 01-1.654-.912v-1.155h-7.027c.37-.4.605-.902.677-1.438l.022-.232a2.65 2.65 0 00-.492-1.669h6.821v-1.154zM8.188 0c.35 0 .7.168.913.503l2.823 4.47a1.079 1.079 0 01-.911 1.655H9.857v6.692h-1.67a2.633 2.633 0 00-1.668.48V6.629H5.365c-.849 0-1.366-.936-.912-1.654L7.276.503A1.072 1.072 0 018.188 0z" />
  </Mark>
);

export const ExaLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path
      clipRule="evenodd"
      d="M3 0h19v1.791L13.892 12 22 22.209V24H3V0zm9.62 10.348l6.589-8.557H6.03l6.59 8.557zM5.138 3.935v7.17h5.52l-5.52-7.17zm5.52 8.96h-5.52v7.17l5.52-7.17zM6.03 22.21l6.59-8.557 6.589 8.557H6.03z"
    />
  </Mark>
);

export const GoogleCloudLogo: React.FC<LogoProps> = (p) => (
  <Mark {...p} evenOdd>
    <path
      clipRule="evenodd"
      d="M4.914 5.18c3.32-3.762 9.095-4.247 12.91-1.13l.362.312.252.232A9.4 9.4 0 0121.02 8.93 6.778 6.778 0 0124 14.597c-.028 3.744-3.087 6.726-6.83 6.697H6.739a6.746 6.746 0 01-3.869-1.222l-.224-.162-.302-.247a6.778 6.778 0 01.54-10.673l-.004.001a9.644 9.644 0 012.034-3.812zm10.345 2.11c-2.143-1.734-5.282-1.46-7.138.578l-.026.025a6.77 6.77 0 014.045 2.523l-3.023 3.023a2.606 2.606 0 10-2.379 3.682h10.43c1.44 0 2.607-1.137 2.607-2.576a2.607 2.607 0 00-2.606-2.607v-.52a5.205 5.205 0 00-1.685-3.933l-.225-.195z"
    />
  </Mark>
);

// ---------------------------------------------------------------------------

export type ProviderKind = "llm" | "stt" | "tts" | "search";

/** What a provider row renders: the mark plus its tinted tile. */
export interface ProviderBrand {
  icon: React.ReactNode;
  /** Classes for the leading icon tile (background wash + glyph color). */
  tileClass: string;
}

/**
 * Every provider wears its own colour, the way its app icon does: a solid
 * brand tile with the mark knocked out in white (or in the brand's second
 * colour, where the real mark has one). A column of identical grey chips made
 * a list of providers read as a list of blanks; the colour is what lets you
 * find Groq or Deepgram without reading.
 *
 * Hexes are the brands' own, from the same two sources as the marks:
 * simple-icons `data/simple-icons.json` (Mistral #FA520F, NVIDIA #76B900,
 * Deepgram #13EF93, Brave #FB542B, Claude #D97757, Perplexity #1FB8CD) and
 * LobeHub's published `COLOR_PRIMARY` / colour marks (Groq #F55036, Cerebras
 * #F15A29, DeepSeek #4D6BFE, Fireworks #5019C5, Exa #1F40ED, Qwen #6336E7 →
 * #6F69F7, Azure #0078D4, AWS #222F3E + #FF9900, Gemini #3186FF, Together
 * #EF2CC1 → #FC4C02, Tavily #468BFF, Kimi #1783FF, OpenRouter's slate).
 *
 * Brands whose mark is black (OpenAI, xAI, Ollama, ElevenLabs, Apple, Z.ai,
 * Moonshot) take the ink tile, which inverts with the theme — black on a light
 * page, white on a dark one — which is exactly how those brands present.
 */
const SOLID =
  "shadow-[inset_0_1px_0_rgb(255_255_255/0.16),inset_0_0_0_1px_rgb(0_0_0/0.06),0_1px_2px_rgb(0_0_0/0.12)] dark:shadow-[inset_0_1px_0_rgb(255_255_255/0.14),inset_0_0_0_1px_rgb(255_255_255/0.1)]";
const PROVIDER_TILES = {
  mono: `bg-ink text-surface ${SOLID}`,
  anthropic: `bg-[#D97757] text-white ${SOLID}`,
  gemini: `bg-linear-to-br from-[#3186FF] to-[#8E75B2] text-white ${SOLID}`,
  groq: `bg-[#F55036] text-white ${SOLID}`,
  mistral: `bg-[#FA520F] text-white ${SOLID}`,
  deepseek: `bg-[#4D6BFE] text-white ${SOLID}`,
  openrouter: `bg-[#5B6B82] text-white ${SOLID}`,
  cerebras: `bg-[#F15A29] text-white ${SOLID}`,
  together: `bg-linear-to-br from-[#EF2CC1] to-[#FC4C02] text-white ${SOLID}`,
  fireworks: `bg-[#5019C5] text-white ${SOLID}`,
  perplexity: `bg-[#0E2A2E] text-[#22B8CD] ${SOLID}`,
  azure: `bg-linear-to-br from-[#0078D4] to-[#114A8B] text-white ${SOLID}`,
  microsoft:
    "bg-white text-ink ring-1 ring-inset ring-hairline-strong dark:ring-transparent",
  aws: `bg-[#222F3E] text-white ${SOLID}`,
  deepgram: `bg-[#101416] text-[#13EF93] ${SOLID}`,
  googlecloud: `bg-[#4285F4] text-white ${SOLID}`,
  brave: `bg-[#FB542B] text-white ${SOLID}`,
  tavily: `bg-[#468BFF] text-white ${SOLID}`,
  exa: `bg-[#1F40ED] text-white ${SOLID}`,
  kimi: `bg-[#1783FF] text-white ${SOLID}`,
  /** Ours (the built-in engine, the local voice): the app's own teal. */
  ours: `bg-accent-fill text-white ${SOLID}`,
  /** Things the user points at themselves (custom endpoints, search APIs
   *  with no mark of their own): a quiet tinted chip, clearly not a brand. */
  generic: "bg-accent/10 text-accent ring-1 ring-inset ring-accent/15",
  fallback: "bg-surface-strong text-muted",
} as const;

type TileKey = keyof typeof PROVIDER_TILES;
type Glyph = (size: number) => React.ReactNode;

interface Entry {
  glyph: Glyph;
  tile: TileKey;
}

const logo =
  (C: React.FC<LogoProps>): Glyph =>
  (size) => <C size={size} />;
const lucide =
  (C: typeof Sparkles): Glyph =>
  (size) => <C size={size} strokeWidth={2} aria-hidden />;

const E = {
  openai: { glyph: logo(OpenAILogo), tile: "mono" },
  anthropic: { glyph: logo(AnthropicLogo), tile: "anthropic" },
  gemini: { glyph: logo(GeminiLogo), tile: "gemini" },
  groq: { glyph: logo(GroqLogo), tile: "groq" },
  mistral: { glyph: logo(MistralLogo), tile: "mistral" },
  deepseek: { glyph: logo(DeepSeekLogo), tile: "deepseek" },
  xai: { glyph: logo(XAILogo), tile: "mono" },
  openrouter: { glyph: logo(OpenRouterLogo), tile: "openrouter" },
  cerebras: { glyph: logo(CerebrasLogo), tile: "cerebras" },
  together: { glyph: logo(TogetherLogo), tile: "together" },
  fireworks: { glyph: logo(FireworksLogo), tile: "fireworks" },
  perplexity: { glyph: logo(PerplexityLogo), tile: "perplexity" },
  azure: { glyph: logo(AzureLogo), tile: "azure" },
  microsoft: {
    glyph: (size) => <MicrosoftLogo size={size} multicolor />,
    tile: "microsoft",
  },
  aws: {
    glyph: (size) => <AWSLogo size={size} smile="#FF9900" />,
    tile: "aws",
  },
  moonshot: { glyph: logo(MoonshotLogo), tile: "mono" },
  kimi: { glyph: logo(KimiLogo), tile: "kimi" },
  zai: { glyph: logo(ZAILogo), tile: "mono" },
  ollama: { glyph: logo(OllamaLogo), tile: "mono" },
  apple: { glyph: logo(AppleLogo), tile: "mono" },
  elevenlabs: { glyph: logo(ElevenLabsLogo), tile: "mono" },
  deepgram: { glyph: logo(DeepgramLogo), tile: "deepgram" },
  googlecloud: { glyph: logo(GoogleCloudLogo), tile: "googlecloud" },
  cartesia: { glyph: lucide(AudioWaveform), tile: "mono" },
  inworld: { glyph: lucide(Earth), tile: "mono" },
  brave: { glyph: logo(BraveLogo), tile: "brave" },
  tavily: { glyph: logo(TavilyLogo), tile: "tavily" },
  exa: { glyph: logo(ExaLogo), tile: "exa" },
  builtin: { glyph: lucide(Cpu), tile: "ours" },
  custom: { glyph: lucide(Plug), tile: "generic" },
  customMic: { glyph: lucide(Mic), tile: "generic" },
  kokoro: { glyph: lucide(AudioLines), tile: "ours" },
  kitten: { glyph: lucide(Cat), tile: "ours" },
  pocket: { glyph: lucide(Speech), tile: "ours" },
  supertonic: { glyph: lucide(Waves), tile: "ours" },
  search: { glyph: lucide(Search), tile: "generic" },
  searchApi: { glyph: lucide(SearchCode), tile: "generic" },
  fish: { glyph: lucide(Fish), tile: "generic" },
  fallback: { glyph: lucide(Sparkles), tile: "fallback" },
} as const satisfies Record<string, Entry>;

/** Exact provider ids as the backend stores them (settings.rs registries,
 *  `assistant_tts_engine`, and the web-search backend switch). */
const EXACT: Record<string, Entry> = {
  // LLM providers (post_process_providers)
  openai: E.openai,
  zai: E.zai,
  openrouter: E.openrouter,
  anthropic: E.anthropic,
  groq: E.groq,
  cerebras: E.cerebras,
  gemini: E.gemini,
  xai: E.xai,
  deepseek: E.deepseek,
  mistral: E.mistral,
  moonshot: E.moonshot,
  together: E.together,
  fireworks: E.fireworks,
  perplexity: E.perplexity,
  azure_openai: E.azure,
  apple_intelligence: E.apple,
  bedrock_mantle: E.aws,
  builtin: E.builtin,
  local: E.ollama,
  custom: E.custom,
  // Cloud STT extras
  elevenlabs: E.elevenlabs,
  deepgram: E.deepgram,
  // TTS engines
  kokoro: E.kokoro,
  kitten: E.kitten,
  pocket: E.pocket,
  supertonic: E.supertonic,
  azure: E.azure,
  cartesia: E.cartesia,
  inworld: E.inworld,
  // Web search backends
  brave: E.brave,
  tavily: E.tavily,
  exa: E.exa,
  serper: E.search,
  serpapi: E.searchApi,
  tinyfish: E.fish,
};

/** Per-kind overrides for ids shared across registries. */
const BY_KIND: Partial<Record<ProviderKind, Record<string, Entry>>> = {
  stt: { custom: E.customMic },
  tts: { azure: E.microsoft, google: E.googlecloud },
};

/** Ordered: more specific brands first (`azure_openai` must not read as
 *  OpenAI, `openrouter` must not read as OpenAI either). */
const LOOSE: ReadonlyArray<[RegExp, Entry]> = [
  [/azure/, E.azure],
  [/bedrock|aws|amazon/, E.aws],
  [/openrouter/, E.openrouter],
  [/gemini|google/, E.gemini],
  [/anthropic|claude/, E.anthropic],
  [/deepseek/, E.deepseek],
  [/mistral/, E.mistral],
  [/groq/, E.groq],
  [/grok|xai|x\.ai/, E.xai],
  [/cerebras/, E.cerebras],
  [/together/, E.together],
  [/fireworks/, E.fireworks],
  [/perplexity|sonar/, E.perplexity],
  [/kimi/, E.kimi],
  [/moonshot/, E.moonshot],
  [/zhipu|glm|z\.ai|bigmodel/, E.zai],
  [/ollama/, E.ollama],
  [/apple/, E.apple],
  [/elevenlabs|eleven_labs/, E.elevenlabs],
  [/deepgram/, E.deepgram],
  [/brave/, E.brave],
  [/tavily/, E.tavily],
  [/openai|whisper|gpt/, E.openai],
  [/microsoft/, E.microsoft],
];

/**
 * Resolve the logo + tile tint for a provider id. Exact id first (with `kind`
 * disambiguating ids that mean different things in different registries), then
 * a loose substring match so user-named or future ids still pick up a brand,
 * then a neutral Sparkles tile.
 */
export const getProviderBrand = (
  id: string,
  kind?: ProviderKind,
  size = 18,
): ProviderBrand => {
  const key = id.trim().toLowerCase();
  const entry =
    (kind ? BY_KIND[kind]?.[key] : undefined) ??
    EXACT[key] ??
    LOOSE.find(([re]) => re.test(key))?.[1] ??
    E.fallback;
  return { icon: entry.glyph(size), tileClass: PROVIDER_TILES[entry.tile] };
};

const TILE_SIZES = {
  sm: { box: "h-6 w-6 rounded-md", glyph: 14 },
  md: { box: "h-8 w-8 rounded-lg", glyph: 17 },
  lg: { box: "h-10 w-10 rounded-xl", glyph: 20 },
} as const;

/** A provider's mark centered in its tinted rounded tile. */
export const ProviderTile: React.FC<{
  id: string;
  kind?: ProviderKind;
  size?: "sm" | "md" | "lg";
  className?: string;
}> = ({ id, kind, size = "md", className }) => {
  const dims = TILE_SIZES[size];
  const brand = getProviderBrand(id, kind, dims.glyph);
  return (
    <span
      aria-hidden
      className={`grid shrink-0 place-items-center ${dims.box} ${brand.tileClass} ${className ?? ""}`}
    >
      {brand.icon}
    </span>
  );
};
