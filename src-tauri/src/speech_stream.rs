//! Incremental sentence chunking, so the assistant can start speaking while the
//! model is still writing.
//!
//! The panel already streams tokens to the screen as they arrive, but speech
//! used to wait for the finished reply: generate everything, *then* synthesize,
//! *then* play. That stacks three waits and leaves several seconds of silence
//! before the first word. Feeding completed sentences to the voice engine as
//! they appear removes the first two waits — the user hears sentence one while
//! sentence two is still being written.
//!
//! Cutting a token stream into speakable pieces is the whole difficulty. Two
//! ways to get it wrong:
//!
//! - **Cutting where there is no sentence end.** "Handy 2.5 is out" must not
//!   become "Handy two" / "five is out", and `1. First item` must not break
//!   after the list marker. Guards below cover decimals, ordinals/list markers,
//!   initials ("J. R. R."), and common abbreviations ("e.g.", "Dr.").
//! - **Cutting inside Markdown.** A fenced code block, a `[label](url)` link or
//!   an inline-code span can straddle a chunk boundary; cutting inside one
//!   leaves the sanitizer unable to recognise it, so raw punctuation or a URL
//!   gets read aloud. Cuts are therefore suppressed inside those constructs.
//!
//! The other half is pacing. The *first* chunk is cut as early as is defensible
//! because it decides how long the user waits in silence; later chunks are
//! allowed to grow so the engine gets whole thoughts and the delivery keeps its
//! natural rhythm. Since speaking a sentence takes longer than generating the
//! next one, the pipeline is ahead of playback after the opening and the extra
//! size costs nothing.
//!
//! Chunks come out already cleaned by [`crate::tts::sanitize_for_speech_chunk`],
//! and pieces that clean away to nothing (a lone code block, a divider) are
//! dropped rather than sent to the engine as empty requests.

/// Sizing rules for [`SpeechChunker`]. Character counts, not bytes, so
/// non-Latin scripts get the same pacing.
#[derive(Debug, Clone, Copy)]
pub struct ChunkPolicy {
    /// Shortest acceptable opening chunk. Small on purpose: time-to-first-word
    /// is the entire point, and a brief opener ("Sure, one moment.") is better
    /// UX than silence.
    pub first_min_chars: usize,
    /// Once the opening chunk is this long with no sentence end in sight, fall
    /// back to a clause boundary (comma/semicolon/colon) instead of waiting for
    /// a full stop. Only ever applies to the first chunk of a reply.
    pub first_clause_chars: usize,
    /// Shortest acceptable follow-up chunk. Larger than the opener so the engine
    /// receives complete thoughts, usually two or three sentences.
    pub min_chars: usize,
    /// Hard ceiling. Past this a cut is forced at the last clause boundary, or
    /// failing that the last space, so a wall of unpunctuated prose still
    /// speaks instead of buffering forever.
    pub max_chars: usize,
}

impl Default for ChunkPolicy {
    fn default() -> Self {
        Self {
            first_min_chars: 24,
            first_clause_chars: 90,
            min_chars: 140,
            max_chars: 400,
        }
    }
}

/// Abbreviations whose trailing period is not a sentence end. Compared
/// lowercased with interior dots removed, so "e.g." matches as "eg" and
/// "U.S." as "us".
const ABBREVIATIONS: &[&str] = &[
    "mr", "mrs", "ms", "dr", "prof", "sr", "jr", "st", "vs", "etc", "eg", "ie", "approx", "fig",
    "no", "al", "inc", "ltd", "co", "corp", "dept", "est", "min", "max", "am", "pm", "us", "uk",
    "eu", "ca", "cf", "ed", "eds", "vol", "pp", "phd", "resp", "aka", "dept", "gov", "sept", "jan",
    "feb", "mar", "apr", "jun", "jul", "aug", "oct", "nov", "dec",
];

/// Characters that legitimately trail a sentence end and belong to the chunk:
/// closing quotes and brackets.
const CLOSERS: &[char] = &['"', '\'', ')', ']', '}', '”', '’', '»', '›'];

/// Western sentence terminators.
const ENDERS: &[char] = &['.', '!', '?', '…'];

/// CJK / fullwidth terminators. These are not followed by a space, so they end a
/// sentence on their own.
const CJK_ENDERS: &[char] = &['。', '！', '？', '．'];

/// Clause separators, used as a fallback cut point.
const CLAUSE_MARKS: &[char] = &[',', ';', ':', '，', '；', '：', '、'];

/// Cuts a stream of model tokens into speakable chunks.
///
/// Feed tokens with [`push`](Self::push) and flush the tail with
/// [`finish`](Self::finish). Both return already-sanitized text ready to hand to
/// a voice engine; an empty vector simply means nothing is speakable yet.
pub struct SpeechChunker {
    policy: ChunkPolicy,
    /// Raw model text not yet emitted.
    buf: String,
    /// Whether anything has been spoken yet this turn, which selects the opening
    /// pacing rules. Only chunks that survived sanitizing count, so a reply that
    /// opens with a code block still gets a fast first *spoken* chunk.
    spoke: bool,
    /// Whether `buf` begins at the start of a line.
    ///
    /// Carried across cuts because a code fence is only a fence at a line start.
    /// Assuming every remainder began a new line let a mid-sentence "```" — as in
    /// prose *about* Markdown — open a phantom fence that never closed, which
    /// suppressed all further cuts and made `finish` discard the rest of the
    /// reply unspoken.
    at_line_start: bool,
}

impl Default for SpeechChunker {
    fn default() -> Self {
        Self::new()
    }
}

impl SpeechChunker {
    pub fn new() -> Self {
        Self::with_policy(ChunkPolicy::default())
    }

    pub fn with_policy(policy: ChunkPolicy) -> Self {
        Self {
            policy,
            buf: String::new(),
            spoke: false,
            at_line_start: true,
        }
    }

    /// Drop all buffered text without speaking it, and rearm the opening pacing.
    ///
    /// Used when a tool-calling round is discarded: the text that round produced
    /// never reaches the transcript, so it must not reach the speaker either.
    pub fn reset(&mut self) {
        self.buf.clear();
        self.spoke = false;
        self.at_line_start = true;
    }

    /// Accept newly streamed text and return every chunk that is now speakable.
    ///
    /// Usually empty — most tokens land mid-sentence — so callers should treat
    /// an empty result as normal.
    pub fn push(&mut self, text: &str) -> Vec<String> {
        self.buf.push_str(text);
        let mut out = Vec::new();
        // One token can complete more than one chunk (a slow consumer, or a
        // provider that delivers a whole paragraph in a single SSE frame).
        // `next_cut` guarantees a non-zero offset, so this always progresses.
        while let Some(cut) = self.next_cut() {
            let raw: String = self.buf.drain(..cut).collect();
            self.at_line_start = raw.ends_with('\n');
            if let Some(clean) = clean_chunk(&raw) {
                self.spoke = true;
                out.push(clean);
            }
        }
        out
    }

    /// Flush the trailing text at the end of a turn.
    ///
    /// Any still-unterminated code fence is dropped: it never received a closing
    /// fence, so the sanitizer cannot recognise it as code and would otherwise
    /// read the block aloud.
    pub fn finish(&mut self) -> Vec<String> {
        let scan = scan(&self.buf, &self.policy, self.spoke, self.at_line_start);
        if let Some(start) = scan.open_fence_start {
            self.buf.truncate(start);
        }
        let raw = std::mem::take(&mut self.buf);
        self.at_line_start = true;
        match clean_chunk(&raw) {
            Some(clean) => {
                self.spoke = true;
                vec![clean]
            }
            None => Vec::new(),
        }
    }

    /// Byte offset to cut the buffer at, or `None` while more text is needed.
    fn next_cut(&self) -> Option<usize> {
        let scan = scan(&self.buf, &self.policy, self.spoke, self.at_line_start);

        // Preferred: the earliest real sentence end at or past the minimum.
        if let Some(end) = scan.sentence_at_min {
            return Some(end);
        }
        // Opening chunk only: a long first sentence is cut at a clause boundary
        // rather than making the user wait for the full stop.
        if !self.spoke && scan.total_chars >= self.policy.first_clause_chars {
            if let Some(end) = scan.first_clause_at_min {
                return Some(end);
            }
        }
        // Runaway paragraph with no usable punctuation: force a cut. The loose
        // space is the last resort, so unbalanced markdown degrades to a slightly
        // awkward cut instead of holding the whole reply back.
        if scan.total_chars > self.policy.max_chars {
            return scan
                .last_clause_before_max
                .or(scan.last_space_before_max)
                .or(scan.last_loose_space_before_max)
                // A cut must consume something. A zero-length cut would make
                // `push`'s drain loop spin without progress, hanging the turn
                // and leaving the assistant permanently busy.
                .filter(|cut| *cut > 0);
        }
        None
    }
}

/// Everything a single left-to-right pass over the buffer needs to report.
#[derive(Debug, Default)]
struct Scan {
    total_chars: usize,
    /// Earliest sentence end at or past the minimum chunk length.
    sentence_at_min: Option<usize>,
    /// Earliest clause boundary at or past the minimum chunk length.
    first_clause_at_min: Option<usize>,
    /// Latest clause boundary still within the hard ceiling.
    last_clause_before_max: Option<usize>,
    /// Latest word gap still within the hard ceiling.
    last_space_before_max: Option<usize>,
    /// Latest word gap that is merely inside an inline construct rather than a
    /// code fence. Used only as a last resort by the hard-ceiling path, so a
    /// pathological run of markdown can never buffer the reply forever.
    last_loose_space_before_max: Option<usize>,
    /// Byte offset of an opening code fence that never closed.
    open_fence_start: Option<usize>,
}

/// Single pass over `buf`, tracking Markdown structure so cuts are only offered
/// at positions where the text can be safely sanitized in isolation.
fn scan(buf: &str, policy: &ChunkPolicy, spoke: bool, starts_at_line_start: bool) -> Scan {
    let min_chars = if spoke {
        policy.min_chars
    } else {
        policy.first_min_chars
    };
    let mut s = Scan::default();

    // Which delimiter opened the current fence. A fence must be closed by its
    // own delimiter — the sanitizer's regex pairs them like-for-like, so
    // treating "~~~" as closing a "```" block would offer a cut past text the
    // sanitizer will not recognise as code, and the code would be read aloud.
    let mut fence_marker: Option<&'static str> = None;
    let mut fence_start: Option<usize> = None;
    let mut inline_code = false;
    let mut in_bracket = false;
    let mut link_depth = 0usize;
    let mut at_line_start = starts_at_line_start;

    let mut i = 0usize;
    let mut char_no = 0usize; // characters consumed before position `i`

    while i < buf.len() {
        let c = match buf[i..].chars().next() {
            Some(c) => c,
            None => break,
        };
        let clen = c.len_utf8();
        let fence_open = fence_marker.is_some();

        // A ``` / ~~~ marker opens or closes a fenced block. Only recognised at
        // the start of a line (with up to three spaces of indent), matching how
        // Markdown — and the sanitizer's own regex — sees it.
        if at_line_start {
            let rest = &buf[i..];
            let indent = rest.len() - rest.trim_start_matches([' ', '\t']).len();
            if indent <= 3 {
                let after = &rest[indent..];
                let marker = if after.starts_with("```") {
                    Some("```")
                } else if after.starts_with("~~~") {
                    Some("~~~")
                } else {
                    None
                };
                if let Some(marker) = marker {
                    match fence_marker {
                        // Only its own delimiter closes a fence.
                        Some(open) if open == marker => {
                            fence_marker = None;
                            fence_start = None;
                        }
                        Some(_) => {}
                        None => {
                            fence_marker = Some(marker);
                            fence_start = Some(i);
                        }
                    }
                    // Skip the rest of the marker line, including its info
                    // string, so nothing on it is mistaken for prose.
                    let line_end = buf[i..].find('\n').map(|n| i + n + 1).unwrap_or(buf.len());
                    char_no += buf[i..line_end].chars().count();
                    i = line_end;
                    at_line_start = true;
                    continue;
                }
            }
        }

        at_line_start = c == '\n';

        // Inline constructs cannot span a line break in any reply we care about,
        // so a newline clears them. Without this the tracking is *sticky*: one
        // unpaired backtick or `[` would mark the rest of the reply as unsafe to
        // cut, and the whole turn would buffer up and be spoken in a single
        // chunk at the end — silently losing the entire latency win.
        if at_line_start {
            inline_code = false;
            in_bracket = false;
            link_depth = 0;
        } else if !fence_open {
            match c {
                '`' if inline_code => inline_code = false,
                // An opener only counts when it is actually closed on this line.
                // Markdown treats an unmatched marker as literal text, and so
                // must we: assuming it opened a span would protect — and
                // therefore refuse to cut — everything after it.
                '`' => inline_code = closes_on_line(buf, i + clen, '`'),
                '[' if !inline_code => in_bracket = closes_on_line(buf, i + clen, ']'),
                ']' if !inline_code => {
                    in_bracket = false;
                    // `](` starts the URL half of a Markdown link. The `(` is
                    // consumed here so it cannot also be counted as nesting.
                    if buf[i + clen..].starts_with('(') {
                        link_depth = 1;
                        char_no += 2;
                        i += clen + 1;
                        continue;
                    }
                }
                // Depth-counted so a URL containing parentheses — Wikipedia's
                // `Foo_(bar)` style — is not treated as closed too early.
                '(' if link_depth > 0 => link_depth += 1,
                ')' if link_depth > 0 => link_depth -= 1,
                _ => {}
            }
        }

        let in_link_url = link_depth > 0;
        let protected = fence_open || inline_code || in_bracket || in_link_url;
        let next_char_no = char_no + 1;

        // Outside a fence, a word gap is always *some* kind of cut point. Code
        // inside a fence is never cut: it is dropped by the sanitizer instead.
        if !fence_open && c.is_whitespace() && next_char_no <= policy.max_chars {
            s.last_loose_space_before_max = Some(i + clen);
        }

        if !protected {
            if c.is_whitespace() {
                if next_char_no <= policy.max_chars {
                    // Past the gap, not at it: a cut offset must always advance,
                    // and the trailing space is trimmed by the sanitizer anyway.
                    // Recording `i` here allowed a zero-length cut on a buffer
                    // starting with whitespace, which made `push` loop forever.
                    s.last_space_before_max = Some(i + clen);
                }
            } else if CLAUSE_MARKS.contains(&c) {
                let end = i + clen;
                if next_char_no >= min_chars && s.first_clause_at_min.is_none() {
                    s.first_clause_at_min = Some(end);
                }
                if next_char_no <= policy.max_chars {
                    s.last_clause_before_max = Some(end);
                }
            } else if ENDERS.contains(&c) || CJK_ENDERS.contains(&c) {
                if let Some((end, end_chars)) = sentence_end(buf, i, c) {
                    if end_chars >= min_chars && s.sentence_at_min.is_none() {
                        s.sentence_at_min = Some(end);
                    }
                    // A sentence end is also a fine forced-cut position.
                    if end_chars <= policy.max_chars {
                        s.last_clause_before_max = Some(end);
                    }
                }
            }
        }

        i += clen;
        char_no = next_char_no;
    }

    s.total_chars = char_no;
    s.open_fence_start = if fence_marker.is_some() {
        fence_start
    } else {
        None
    };
    s
}

/// Whether `closer` appears in `buf` from `from` before the end of the line.
///
/// Used to tell a real inline construct from a stray character. Mid-stream the
/// closer may simply not have arrived yet; the lenient answer is the safe one,
/// because inline code and link labels are spoken as their text anyway — only a
/// fenced block must never be cut into, and fences are tracked separately.
fn closes_on_line(buf: &str, from: usize, closer: char) -> bool {
    buf[from..]
        .chars()
        .take_while(|c| *c != '\n')
        .any(|c| c == closer)
}

/// Decide whether the terminator at byte offset `i` really ends a sentence.
///
/// Returns the byte offset just past the sentence (trailing quotes and brackets
/// included) together with its character count, or `None` when the mark is a
/// decimal point, a list marker, an initial, or part of an abbreviation.
fn sentence_end(buf: &str, i: usize, c: char) -> Option<(usize, usize)> {
    // Absorb repeated terminators ("...", "?!") and any closing punctuation, so
    // the chunk keeps them instead of orphaning them onto the next one.
    let mut end = i + c.len_utf8();
    while let Some(next) = buf[end..].chars().next() {
        if ENDERS.contains(&next) || CJK_ENDERS.contains(&next) || CLOSERS.contains(&next) {
            end += next.len_utf8();
        } else {
            break;
        }
    }

    let following = buf[end..].chars().next();

    // CJK terminators are not space-separated, so they stand on their own.
    let cjk = CJK_ENDERS.contains(&c);
    if !cjk {
        // Mid-stream, a terminator at the very end of the buffer is ambiguous:
        // "2." may still become "2.5". Wait for the next character.
        match following {
            Some(n) if n.is_whitespace() => {}
            _ => return None,
        }
    }

    if c == '.' {
        let token = preceding_token(buf, i);
        if token.is_empty() {
            return None;
        }
        // "1." / "42." — an ordered-list marker or a bare number.
        if token.chars().all(|ch| ch.is_ascii_digit()) {
            return None;
        }
        // "J. R. R." — a single letter is an initial, not a sentence.
        let mut chars = token.chars();
        if let (Some(only), None) = (chars.next(), chars.next()) {
            if only.is_alphabetic() {
                return None;
            }
        }
        let normalized: String = token
            .chars()
            .filter(|ch| *ch != '.')
            .flat_map(|ch| ch.to_lowercase())
            .collect();
        if ABBREVIATIONS.contains(&normalized.as_str()) {
            return None;
        }
    }

    // A lowercase word after the terminator means the thought is still running:
    // an abbreviation we do not list ("approx. twelve"), or a mid-sentence pause
    // ("Well... it depends"). Prose starts new sentences with a capital, a digit
    // or a quote, so treating this as a continuation costs at most a slightly
    // longer chunk, while cutting here would be audibly wrong.
    //
    // This needs the next word to exist, so a terminator followed only by
    // trailing whitespace defers until more text arrives — one token of delay.
    // The final sentence of a reply has no next word and is flushed by
    // [`SpeechChunker::finish`] instead. Skipped for CJK, which has no letter
    // case and unambiguous terminators.
    if !cjk {
        let next_word = buf[end..].split_whitespace().next()?;
        if next_word
            .chars()
            .next()
            .map(|ch| ch.is_lowercase() && ch.is_alphabetic())
            .unwrap_or(false)
        {
            return None;
        }
    }

    Some((end, buf[..end].chars().count()))
}

/// The run of non-whitespace characters immediately before byte offset `i`.
fn preceding_token(buf: &str, i: usize) -> &str {
    let head = &buf[..i];
    match head.rfind(char::is_whitespace) {
        Some(pos) => {
            let start = pos
                + head[pos..]
                    .chars()
                    .next()
                    .map(|c| c.len_utf8())
                    .unwrap_or(1);
            &head[start..]
        }
        None => head,
    }
}

/// Sanitize one chunk, returning `None` when nothing speakable is left.
fn clean_chunk(raw: &str) -> Option<String> {
    let clean = crate::tts::sanitize_for_speech_chunk(raw);
    if clean.trim().is_empty() {
        None
    } else {
        Some(clean)
    }
}

// ---------------------------------------------------------------------------
// Pipeline: chunker in, audio out
// ---------------------------------------------------------------------------

/// Where finished chunks are sent for synthesis.
enum Delivery {
    /// The local Kokoro model runs in the panel webview (kokoro-js/WebGPU), so
    /// chunks are forwarded as events and the webview streams them into a
    /// splitter that stays open for the whole reply.
    Local { app: tauri::AppHandle },
    /// Remote HTTP engines are synthesized here, one chunk at a time, by a task
    /// that outlives the turn if playback is still catching up.
    Remote {
        tx: tokio::sync::mpsc::UnboundedSender<String>,
    },
    /// The reply is closed; further chunks are ignored.
    Closed,
}

/// Turns a stream of model tokens into speech that starts before the model has
/// finished writing.
///
/// Create one per reply, feed it every token, and call
/// [`finish`](Self::finish) when the reply ends. Chunk pacing, Markdown
/// filtering and cancellation are handled internally; callers only forward text.
pub struct SpeechPipeline {
    chunker: SpeechChunker,
    delivery: Delivery,
    /// Cancellation epoch captured when the reply started. Everything downstream
    /// re-checks it, so a Stop silences queued and in-flight audio alike.
    epoch: u64,
    /// Whether any chunk was actually handed to an engine, so the caller can
    /// tell "speaking" from "there was nothing to say" (a code-only reply).
    spoke: bool,
}

impl SpeechPipeline {
    /// Begin streamed speech for one reply.
    ///
    /// `epoch` must be captured by the caller *before* generation starts, so a
    /// Stop pressed during generation still supersedes this reply.
    pub fn start(
        app: &tauri::AppHandle,
        settings: &crate::settings::AppSettings,
        epoch: u64,
    ) -> Self {
        let delivery = if settings.assistant_tts_engine == "kokoro" {
            use tauri::Emitter;
            // Tells the webview to open a splitter for this reply. The hook
            // ignores it when speech is disabled. The epoch rides along so every
            // chunk the webview sends back can be attributed to this reply, and
            // a chunk still in flight when the user hits Stop is dropped instead
            // of being adopted by whatever comes next.
            let _ = app.emit("assistant-tts-begin", epoch);
            Delivery::Local { app: app.clone() }
        } else {
            Delivery::Remote {
                tx: spawn_remote_synthesis(app.clone(), settings.clone(), epoch),
            }
        };
        Self {
            chunker: SpeechChunker::new(),
            delivery,
            epoch,
            spoke: false,
        }
    }

    /// Forward newly streamed tokens, speaking any chunk they complete.
    pub fn push(&mut self, tokens: &str) {
        if crate::tts::current_epoch() != self.epoch {
            return;
        }
        for chunk in self.chunker.push(tokens) {
            self.deliver(chunk);
        }
    }

    /// Discard text buffered but not yet spoken.
    ///
    /// Used between tool-calling rounds: a round that ends in tool calls has its
    /// text replaced by the next round, so its unspoken tail must not be voiced
    /// as though it belonged to the answer. Anything already spoken stands — it
    /// was the model's own words and the panel showed it live.
    pub fn reset(&mut self) {
        self.chunker.reset();
    }

    /// Whether any speech was actually sent to an engine.
    pub fn spoke(&self) -> bool {
        self.spoke
    }

    /// Speak the trailing text and close the reply. Idempotent.
    ///
    /// Must be called on every exit path — success, provider error, or Stop — so
    /// the synthesis task ends and the audio device is released.
    pub fn finish(&mut self) {
        if matches!(self.delivery, Delivery::Closed) {
            return;
        }
        // A superseded reply skips its tail; the epoch check inside `deliver`
        // would drop it anyway, but this also avoids pointless sanitizing.
        if crate::tts::current_epoch() == self.epoch {
            for chunk in self.chunker.finish() {
                self.deliver(chunk);
            }
        }
        if let Delivery::Local { app } = &self.delivery {
            use tauri::Emitter;
            let _ = app.emit("assistant-tts-end", ());
        }
        // Replacing the delivery drops the channel sender, which ends the
        // synthesis task; it releases the audio device once the queue has played.
        self.delivery = Delivery::Closed;
    }

    fn deliver(&mut self, chunk: String) {
        match &self.delivery {
            Delivery::Local { app } => {
                use tauri::Emitter;
                let _ = app.emit("assistant-tts-chunk", chunk);
                self.spoke = true;
            }
            Delivery::Remote { tx } => {
                // A closed channel means synthesis already stopped (error or
                // cancellation); dropping the chunk is the correct response.
                if tx.send(chunk).is_ok() {
                    self.spoke = true;
                }
            }
            Delivery::Closed => {}
        }
    }
}

impl Drop for SpeechPipeline {
    /// Safety net for exit paths that never reach an explicit
    /// [`finish`](SpeechPipeline::finish) — a turn that bails out mid-way, for
    /// instance. Without it the local engine would never receive
    /// `assistant-tts-end` and the webview's splitter would stay open until the
    /// next reply. `finish` is idempotent, so the normal path is unaffected.
    fn drop(&mut self) {
        self.finish();
    }
}

/// Spawn the task that synthesizes remote chunks in order.
///
/// Deliberately sequential. Synthesizing a sentence is far faster than speaking
/// it, so one request at a time still stays comfortably ahead of playback, while
/// avoiding parallel requests keeps chunks in order, keeps request rate low, and
/// is required for ElevenLabs Request Stitching, where each chunk is conditioned
/// on the ids of the ones before it.
fn spawn_remote_synthesis(
    app: tauri::AppHandle,
    settings: crate::settings::AppSettings,
    epoch: u64,
) -> tokio::sync::mpsc::UnboundedSender<String> {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    tauri::async_runtime::spawn(async move {
        let device = settings.selected_output_device.clone();
        let volume = settings.audio_feedback_volume;
        let mut stitch_ids: Vec<String> = Vec::new();

        while let Some(text) = rx.recv().await {
            if crate::tts::current_epoch() != epoch {
                break;
            }
            let synthesized = crate::tts::synthesize_speech(
                &settings,
                crate::tts::SpeechRequest {
                    text: &text,
                    previous_request_ids: &stitch_ids,
                },
            )
            .await;

            match synthesized {
                Ok(speech) => {
                    if let Some(id) = speech.request_id {
                        stitch_ids.push(id);
                    }
                    let app_play = app.clone();
                    let device = device.clone();
                    // Queueing can block briefly when playback is far behind, so
                    // it runs off the async runtime.
                    let queued = tauri::async_runtime::spawn_blocking(move || {
                        crate::tts::enqueue_speech_chunk(
                            &app_play,
                            speech.bytes,
                            device,
                            volume,
                            epoch,
                        )
                    })
                    .await;
                    if let Ok(Err(e)) = queued {
                        log::error!("Failed to queue speech chunk: {}", e);
                        break;
                    }
                }
                Err(e) => {
                    log::error!("Streaming TTS request failed: {}", e);
                    // Report once and stop: a configuration failure (bad key,
                    // wrong voice) would otherwise repeat for every sentence.
                    if crate::tts::current_epoch() == epoch {
                        crate::assistant::emit_error(&app, "tts", e);
                    }
                    break;
                }
            }
        }
        crate::tts::finish_speech_stream(epoch);
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Feed text one character at a time — the worst case for a streaming
    /// splitter, and close to what a fast local model actually produces.
    fn stream_chars(text: &str, policy: ChunkPolicy) -> Vec<String> {
        let mut chunker = SpeechChunker::with_policy(policy);
        let mut out = Vec::new();
        for ch in text.chars() {
            out.extend(chunker.push(&ch.to_string()));
        }
        out.extend(chunker.finish());
        out
    }

    /// Feed the whole string at once, as a batching cloud provider might.
    fn stream_whole(text: &str, policy: ChunkPolicy) -> Vec<String> {
        let mut chunker = SpeechChunker::with_policy(policy);
        let mut out = chunker.push(text);
        out.extend(chunker.finish());
        out
    }

    /// Small thresholds so tests exercise boundary logic rather than sizing.
    fn eager() -> ChunkPolicy {
        ChunkPolicy {
            first_min_chars: 1,
            first_clause_chars: usize::MAX,
            min_chars: 1,
            max_chars: 10_000,
        }
    }

    #[test]
    fn splits_on_sentence_ends() {
        let out = stream_chars("First sentence. Second one! Third?", eager());
        assert_eq!(
            out,
            vec![
                "First sentence.".to_string(),
                "Second one!".to_string(),
                "Third?".to_string()
            ]
        );
    }

    #[test]
    fn chunking_is_independent_of_token_size() {
        let text = "One thing happened. Then another thing happened! And finally a question?";
        assert_eq!(stream_chars(text, eager()), stream_whole(text, eager()));
    }

    #[test]
    fn keeps_decimals_intact() {
        let out = stream_chars("Version 2.5 is out. Upgrade now.", eager());
        assert_eq!(
            out,
            vec![
                "Version 2.5 is out.".to_string(),
                "Upgrade now.".to_string()
            ]
        );
    }

    #[test]
    fn does_not_split_on_abbreviations() {
        let out = stream_chars("Use a linter, e.g. ESLint, first. Then ship.", eager());
        assert_eq!(
            out,
            vec![
                "Use a linter, e.g. ESLint, first.".to_string(),
                "Then ship.".to_string()
            ]
        );
    }

    #[test]
    fn does_not_split_on_initials() {
        let out = stream_chars("It was J. R. R. Tolkien. He wrote it.", eager());
        assert_eq!(
            out,
            vec![
                "It was J. R. R. Tolkien.".to_string(),
                "He wrote it.".to_string()
            ]
        );
    }

    #[test]
    fn does_not_split_on_list_markers() {
        let out = stream_chars("Steps:\n1. Open it.\n2. Close it.\n", eager());
        // A colon is a clause mark, not a sentence end, so the lead-in stays
        // attached. What matters is that "1." and "2." did not split.
        assert_eq!(
            out,
            vec!["Steps: 1. Open it.".to_string(), "2. Close it.".to_string()]
        );
    }

    #[test]
    fn keeps_ellipsis_together() {
        let out = stream_chars("Well... it depends. Really.", eager());
        assert_eq!(
            out,
            vec!["Well... it depends.".to_string(), "Really.".to_string()]
        );
    }

    #[test]
    fn keeps_closing_quotes_with_the_sentence() {
        let out = stream_chars("He said \"stop.\" Then he left.", eager());
        assert_eq!(
            out,
            vec!["He said \"stop.\"".to_string(), "Then he left.".to_string()]
        );
    }

    #[test]
    fn does_not_cut_inside_a_link_url() {
        // The dots inside the URL must not become sentence ends, and the link
        // text must survive while the URL is dropped.
        let out = stream_chars("See [the docs](https://ex.co/a.b.html) now. Done.", eager());
        assert_eq!(
            out,
            vec!["See the docs now.".to_string(), "Done.".to_string()]
        );
    }

    #[test]
    fn drops_fenced_code_and_speaks_the_prose_around_it() {
        let text = "Run this. \n```rust\nlet x = 1. let y = 2.\n```\nThat is all.";
        let out = stream_chars(text, eager());
        assert_eq!(
            out,
            vec!["Run this.".to_string(), "That is all.".to_string()]
        );
    }

    #[test]
    fn drops_an_unterminated_code_fence() {
        // A cancelled or truncated reply can leave a fence open; its contents
        // must never be read aloud.
        let text = "Here you go.\n```python\nprint('hi'). more code.";
        let out = stream_chars(text, eager());
        assert_eq!(out, vec!["Here you go.".to_string()]);
    }

    #[test]
    fn never_emits_an_empty_chunk() {
        let out = stream_chars("```\ncode.\n```\n\n---\n\n:tada:\n", eager());
        assert!(
            out.iter().all(|c| !c.trim().is_empty()),
            "empty chunk in {out:?}"
        );
    }

    #[test]
    fn first_chunk_is_cut_early_and_later_ones_grow() {
        let policy = ChunkPolicy {
            first_min_chars: 10,
            first_clause_chars: 90,
            min_chars: 60,
            max_chars: 400,
        };
        let text = "Sure thing. Alpha beta gamma. Delta epsilon zeta. Eta theta iota. Kappa.";
        let out = stream_chars(text, policy);
        // The opener is released on its own for a fast start.
        assert_eq!(out[0], "Sure thing.");
        // Later chunks accumulate past the larger minimum instead of going out
        // one short sentence at a time.
        assert!(
            out[1].len() >= 60,
            "follow-up chunk should batch sentences, got {:?}",
            out[1]
        );
    }

    #[test]
    fn long_opening_sentence_falls_back_to_a_clause() {
        let policy = ChunkPolicy {
            first_min_chars: 10,
            first_clause_chars: 40,
            min_chars: 140,
            max_chars: 400,
        };
        let mut chunker = SpeechChunker::with_policy(policy);
        let out = chunker.push(
            "The build is failing because the linker cannot find the Vulkan SDK, which usually means",
        );
        assert_eq!(
            out,
            vec!["The build is failing because the linker cannot find the Vulkan SDK,".to_string()],
            "expected a clause cut so audio can start"
        );
    }

    #[test]
    fn clause_fallback_applies_only_to_the_opening_chunk() {
        let policy = ChunkPolicy {
            first_min_chars: 5,
            first_clause_chars: 20,
            min_chars: 140,
            max_chars: 400,
        };
        let mut chunker = SpeechChunker::with_policy(policy);
        let first = chunker.push("Right. Now");
        assert_eq!(first, vec!["Right.".to_string()]);
        // Now that something has been spoken, commas must not trigger cuts.
        let second = chunker.push("This clause, and this clause, keep going for a while.");
        assert!(
            second.is_empty(),
            "commas should not cut follow-up chunks: {second:?}"
        );
    }

    #[test]
    fn forces_a_cut_on_unpunctuated_prose() {
        let policy = ChunkPolicy {
            first_min_chars: 10,
            first_clause_chars: usize::MAX,
            min_chars: 10,
            max_chars: 40,
        };
        let mut chunker = SpeechChunker::with_policy(policy);
        let out = chunker.push(&"word ".repeat(20));
        assert!(
            !out.is_empty(),
            "a long run without punctuation must still be spoken"
        );
        assert!(out[0].chars().count() <= 40, "forced cut too long: {out:?}");
    }

    #[test]
    fn handles_cjk_sentence_marks() {
        let out = stream_chars("这是第一句。这是第二句！", eager());
        assert_eq!(
            out,
            vec!["这是第一句。".to_string(), "这是第二句！".to_string()]
        );
    }

    #[test]
    fn waits_until_the_following_word_is_visible() {
        // "that." could still become "that.5", and the word after it decides
        // whether this is a real sentence end, so the cut waits for both.
        let mut chunker = SpeechChunker::with_policy(eager());
        assert!(chunker.push("Let me check that.").is_empty());
        assert!(chunker.push(" ").is_empty());
        assert_eq!(chunker.push("Then"), vec!["Let me check that.".to_string()]);
    }

    #[test]
    fn an_unpaired_backtick_does_not_stall_the_stream() {
        // Regression: inline-construct tracking used to be sticky, so a single
        // unmatched backtick marked the rest of the reply unsafe to cut and the
        // whole turn was buffered and spoken as one chunk at the end — the
        // latency win silently disappearing on a stray character.
        let text = "Use the `run command. Then it builds. Then it ships. Done.";
        let out = stream_chars(text, eager());
        assert!(
            out.len() > 1,
            "expected multiple chunks despite the unpaired backtick, got {out:?}"
        );
    }

    #[test]
    fn an_unmatched_bracket_does_not_stall_the_stream() {
        let text = "See [the docs for details. Then run it. Then check output. Done.";
        let out = stream_chars(text, eager());
        assert!(
            out.len() > 1,
            "expected multiple chunks despite the unmatched bracket, got {out:?}"
        );
    }

    #[test]
    fn markdown_state_cannot_hold_text_past_the_hard_ceiling() {
        // Even with an inline construct left open on the same line, the hard
        // ceiling must still force a cut rather than buffering without limit.
        let policy = ChunkPolicy {
            first_min_chars: 10,
            first_clause_chars: usize::MAX,
            min_chars: 10,
            max_chars: 40,
        };
        let mut chunker = SpeechChunker::with_policy(policy);
        let out = chunker.push(&format!("`{}", "word ".repeat(30)));
        assert!(
            !out.is_empty(),
            "hard ceiling must still fire inside an open inline construct"
        );
    }

    #[test]
    fn does_not_cut_inside_a_url_containing_parentheses() {
        let out = stream_chars(
            "Read [the page](https://en.wikipedia.org/wiki/Foo_(bar)) first. Done.",
            eager(),
        );
        assert_eq!(
            out,
            vec!["Read the page first.".to_string(), "Done.".to_string()]
        );
    }

    #[test]
    fn a_long_unbroken_token_after_a_sentence_cannot_hang() {
        // Regression: whitespace cut points were recorded *at* the space, so a
        // remainder beginning with a space produced a zero-length cut and
        // `push`'s drain loop spun forever — hanging the turn and leaving the
        // assistant permanently busy. A hex digest triggers it.
        let text = format!(
            "Here is the file digest for you. {}",
            "0123456789abcdef".repeat(30)
        );
        let mut chunker = SpeechChunker::new();
        // Would never return before the fix.
        let first = chunker.push(&text);
        let rest = chunker.finish();
        assert!(!first.is_empty());
        let joined = [first, rest].concat().join(" ");
        assert!(joined.contains("Here is the file digest for you."));
        assert!(joined.contains("0123456789abcdef"), "digest text was lost");
    }

    #[test]
    fn prose_mentioning_a_fence_marker_is_not_treated_as_code() {
        // Regression: every chunk boundary was assumed to start a new line, so a
        // mid-sentence "```" opened a phantom fence that never closed. That
        // suppressed all further cuts and made `finish` truncate at the marker —
        // silently discarding the whole rest of the reply.
        //
        // The text *between* the two markers is still dropped, because the
        // sanitizer has always read same-line triple backticks as a code span.
        // What matters here is that everything after them survives.
        let text = "You wrap code in triple backticks. ``` starts a block and ``` ends it.";
        let out = stream_chars(text, ChunkPolicy::default());
        let joined = out.join(" ");
        assert!(
            joined.contains("You wrap code in triple backticks."),
            "opening sentence lost: {out:?}"
        );
        assert!(
            joined.contains("ends it"),
            "text after a mid-line fence marker was discarded: {out:?}"
        );
    }

    #[test]
    fn a_fence_is_only_closed_by_its_own_delimiter() {
        // The sanitizer pairs ``` with ``` and ~~~ with ~~~, so treating a
        // mismatched marker as a closer would offer a cut past text the
        // sanitizer cannot recognise as code — and the code would be spoken.
        let text = "Here you go.\n```python\nprint('secret')\n~~~\nAll done now.";
        let joined = stream_chars(text, eager()).join(" ");
        assert!(
            !joined.contains("print"),
            "code leaked into speech: {joined:?}"
        );
    }

    #[test]
    fn reset_discards_buffered_text_and_rearms_the_opening() {
        let mut chunker = SpeechChunker::with_policy(eager());
        assert_eq!(
            chunker.push("Let me check that. Then"),
            vec!["Let me check that."]
        );
        chunker.push(" more text");
        chunker.reset();
        // The buffered tail is gone rather than spoken later.
        assert!(chunker.finish().is_empty());
    }

    #[test]
    fn reassembles_the_full_reply_across_chunks() {
        // Nothing spoken should go missing: every word of the prose (bar the
        // deliberately dropped code block) must appear in some chunk.
        let text = "First point here. Second point follows, with detail. \
                    Finally a closing thought about version 1.2 and Dr. Smith.";
        let joined = stream_chars(text, eager()).join(" ");
        for word in ["First", "Second", "detail", "1.2", "Dr.", "Smith"] {
            assert!(joined.contains(word), "{word:?} missing from {joined:?}");
        }
    }
}
