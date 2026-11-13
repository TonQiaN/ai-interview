use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use futures_util::{StreamExt, TryStreamExt, stream};
use interview_audio::{Audio, decode_wav, encode_wav};

use crate::backend::{AudioStream, EngineStatus, TtsBackend, TtsError};
use crate::cache::AudioCache;
use crate::request::SynthesisRequest;
use crate::segment::split_text;

/// 一段音频最少要多响才算有声音。真人声的峰值远高于这个数。
const MIN_PEAK: f32 = 1e-3;
/// 每个字最少多少毫秒。再快的语速也到不了这个数，低于它基本是被截断了。
const MIN_MS_PER_CHAR: u64 = 25;

/// 合成好的一段话。
#[derive(Debug, Clone, PartialEq)]
pub struct Synthesized {
    /// 请求的内容哈希，也是缓存的键。
    pub content_hash: String,
    pub audio: Audio,
    /// 16 位单声道 WAV，可以直接存进 BlobStore 或发给浏览器。
    pub wav: Vec<u8>,
    /// 这次是不是直接从缓存里拿的。
    pub from_cache: bool,
}

impl Synthesized {
    pub fn duration_ms(&self) -> u64 {
        self.audio.duration_ms()
    }

    pub fn sample_rate(&self) -> u32 {
        self.audio.sample_rate()
    }
}

/// 批量合成里的一条。`key` 由调用方定（比如题目的编号），结果里原样带回。
#[derive(Debug, Clone, PartialEq)]
pub struct BatchItem {
    pub key: String,
    pub request: SynthesisRequest,
}

/// 批量合成里一条的结果。一条失败不影响其他条。
#[derive(Debug, Clone, PartialEq)]
pub struct BatchOutcome {
    pub key: String,
    pub result: Result<Synthesized, TtsError>,
}

/// 语音合成的入口：检查请求、查缓存、长文分段、调后端、检查结果、写缓存。
///
/// 面试准备阶段用 [`Synthesizer::synthesize_batch`] 把所有题目一次合成好；
/// 实时对话用 [`Synthesizer::synthesize_stream`] 边合成边播。
#[derive(Clone)]
pub struct Synthesizer {
    backend: Arc<dyn TtsBackend>,
    cache: Option<Arc<dyn AudioCache>>,
    max_attempts: u32,
}

impl Synthesizer {
    pub fn new(backend: Arc<dyn TtsBackend>) -> Self {
        Self {
            backend,
            cache: None,
            max_attempts: 2,
        }
    }

    /// 接上缓存。不接也能用，只是每次都重新合成。
    pub fn with_cache(mut self, cache: Arc<dyn AudioCache>) -> Self {
        self.cache = Some(cache);
        self
    }

    /// 后端返回的音频明显不对时，一段最多合成几次（每次换一个种子）。默认 2 次。
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    pub fn backend_name(&self) -> &'static str {
        self.backend.name()
    }

    /// 三个引擎现在各自能不能用。
    pub async fn status(&self) -> Vec<EngineStatus> {
        self.backend.status().await
    }

    /// 合成一段话，返回整段音频。
    pub async fn synthesize(&self, request: &SynthesisRequest) -> Result<Synthesized, TtsError> {
        request.check()?;
        let content_hash = request.content_hash();
        if let Some(hit) = self.read_cache(&content_hash).await {
            return Ok(hit);
        }

        let mut joined: Option<Audio> = None;
        for segment in segments_of(request) {
            let audio = self.synthesize_segment(request, segment).await?;
            match &mut joined {
                None => joined = Some(audio),
                Some(whole) => {
                    let pause =
                        Audio::silence(whole.sample_rate(), request.options.segment_pause_ms)
                            .map_err(bad_audio)?;
                    whole.append(&pause).map_err(bad_audio)?;
                    whole.append(&audio).map_err(bad_audio)?;
                }
            }
        }
        // 检查过的请求文本不是空的，至少有一段
        let audio = joined.ok_or(TtsError::InvalidRequest(crate::RequestError::EmptyText))?;
        let wav = encode_wav(&audio);
        self.write_cache(&content_hash, &wav).await;
        Ok(Synthesized {
            content_hash,
            audio,
            wav,
            from_cache: false,
        })
    }

    /// 按顺序合成一批，结果的顺序和输入一致。
    ///
    /// 一条失败只影响它自己。内容完全相同的条目只合成一次。
    /// 一条一条地做，不并发：一台 Mac 上的模型同一时间只能合成一段。
    pub async fn synthesize_batch(&self, items: Vec<BatchItem>) -> Vec<BatchOutcome> {
        let mut done: HashMap<String, Synthesized> = HashMap::new();
        let mut outcomes = Vec::with_capacity(items.len());
        for BatchItem { key, request } in items {
            let content_hash = request.content_hash();
            let result = match done.get(&content_hash) {
                Some(same) => Ok(Synthesized {
                    from_cache: true,
                    ..same.clone()
                }),
                None => {
                    let result = self.synthesize(&request).await;
                    if let Ok(synthesized) = &result {
                        done.insert(content_hash, synthesized.clone());
                    }
                    result
                }
            };
            outcomes.push(BatchOutcome { key, result });
        }
        outcomes
    }

    /// 边合成边返回，给实时对话用。
    ///
    /// 长文按句子分段，一段一段地交给后端，所以第一块音频不用等整段话合成完。
    /// 命中缓存时整段作为一块返回。流式的结果不写缓存（实时对话里的话几乎不会重复）。
    pub async fn synthesize_stream(
        &self,
        request: &SynthesisRequest,
    ) -> Result<AudioStream, TtsError> {
        request.check()?;
        let content_hash = request.content_hash();
        if let Some(hit) = self.read_cache(&content_hash).await {
            return Ok(Box::pin(stream::once(async move { Ok(hit.audio) })));
        }

        let segments: Vec<SynthesisRequest> = segments_of(request)
            .into_iter()
            .map(|text| SynthesisRequest {
                text,
                ..request.clone()
            })
            .collect();
        let last = segments.len().saturating_sub(1);
        let pause_ms = request.options.segment_pause_ms;
        let backend = Arc::clone(&self.backend);

        let chunks = stream::iter(segments.into_iter().enumerate())
            .then(move |(index, segment)| {
                let backend = Arc::clone(&backend);
                async move {
                    let inner = backend.synthesize_stream(&segment).await?;
                    if index == last || pause_ms == 0 {
                        return Ok(inner);
                    }
                    // 段和段之间补一小段静音，采样率跟这一段最后一块一致
                    let rate = Arc::new(AtomicU32::new(0));
                    let seen = Arc::clone(&rate);
                    let inner = inner.inspect(move |chunk| {
                        if let Ok(audio) = chunk {
                            seen.store(audio.sample_rate(), Ordering::Relaxed);
                        }
                    });
                    let pause = stream::once(async move {
                        Audio::silence(rate.load(Ordering::Relaxed), pause_ms)
                    })
                    // 这一段一块都没出来时采样率是 0，造不出静音，直接跳过
                    .filter_map(|silence| async move { silence.ok().map(Ok) });
                    Ok::<AudioStream, TtsError>(Box::pin(inner.chain(pause)))
                }
            })
            .try_flatten();
        Ok(Box::pin(chunks))
    }

    async fn synthesize_segment(
        &self,
        request: &SynthesisRequest,
        text: String,
    ) -> Result<Audio, TtsError> {
        let mut segment = SynthesisRequest {
            text,
            ..request.clone()
        };
        let mut attempt = 1;
        loop {
            let audio = self.backend.synthesize(&segment).await?;
            match check_audio(&audio, &segment.text) {
                Ok(()) => return Ok(audio),
                Err(reason) if attempt < self.max_attempts => {
                    tracing::warn!(
                        engine = %segment.engine,
                        attempt,
                        %reason,
                        "合成出来的音频不对，换个种子重试"
                    );
                    attempt += 1;
                    segment.options.seed = segment.options.seed.wrapping_add(1);
                }
                Err(reason) => return Err(TtsError::BadAudio(reason)),
            }
        }
    }

    async fn read_cache(&self, content_hash: &str) -> Option<Synthesized> {
        let cache = self.cache.as_ref()?;
        let wav = match cache.get(content_hash).await {
            Ok(Some(wav)) => wav,
            Ok(None) => return None,
            Err(error) => {
                tracing::warn!(%error, "读合成缓存失败，这次重新合成");
                return None;
            }
        };
        match decode_wav(&wav) {
            Ok(audio) if !audio.is_empty() => Some(Synthesized {
                content_hash: content_hash.to_owned(),
                audio,
                wav,
                from_cache: true,
            }),
            // 缓存里的文件坏了：当没有，重新合成后会覆盖它
            _ => {
                tracing::warn!(content_hash, "合成缓存里的音频读不出来，这次重新合成");
                None
            }
        }
    }

    async fn write_cache(&self, content_hash: &str, wav: &[u8]) {
        let Some(cache) = &self.cache else { return };
        if let Err(error) = cache.put(content_hash, wav).await {
            tracing::warn!(%error, "写合成缓存失败，结果照常返回");
        }
    }
}

/// 按引擎的单段上限把文本切开。没有上限的引擎自己会分段，整段交给它。
fn segments_of(request: &SynthesisRequest) -> Vec<String> {
    match request.engine.spec().max_text_chars {
        Some(max_chars) => split_text(&request.text, max_chars),
        None => vec![request.text.trim().to_owned()],
    }
}

/// 后端返回的音频是不是明显不对。返回的是给人看的原因。
fn check_audio(audio: &Audio, text: &str) -> Result<(), String> {
    if audio.is_empty() {
        return Err("音频是空的".to_owned());
    }
    if audio.samples().iter().any(|sample| !sample.is_finite()) {
        return Err("音频里有无效的采样".to_owned());
    }
    if audio.peak() < MIN_PEAK {
        return Err("音频全是静音".to_owned());
    }
    let chars = text.chars().filter(|c| c.is_alphanumeric()).count() as u64;
    let duration_ms = audio.duration_ms();
    if duration_ms < chars * MIN_MS_PER_CHAR {
        return Err(format!("{chars} 个字只有 {duration_ms} 毫秒，可能被截断了"));
    }
    Ok(())
}

fn bad_audio(error: interview_audio::AudioError) -> TtsError {
    TtsError::BadAudio(error.to_string())
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::cache::{CacheError, MemoryCache};
    use crate::engine::EngineId;
    use crate::fake::FakeBackend;
    use crate::request::testing::request;
    use crate::{RequestError, Voice};

    fn setup(backend: FakeBackend) -> (Arc<FakeBackend>, Arc<MemoryCache>, Synthesizer) {
        let backend = Arc::new(backend);
        let cache = Arc::new(MemoryCache::new());
        let synthesizer = Synthesizer::new(backend.clone()).with_cache(cache.clone());
        (backend, cache, synthesizer)
    }

    fn item(key: &str, engine: EngineId, text: &str) -> BatchItem {
        BatchItem {
            key: key.to_owned(),
            request: request(engine, text),
        }
    }

    #[tokio::test]
    async fn the_second_identical_request_comes_from_the_cache() {
        let (backend, cache, synthesizer) = setup(FakeBackend::new());
        let req = request(EngineId::Index2, "请先做一个简单的自我介绍。");

        let first = synthesizer.synthesize(&req).await.unwrap();
        assert!(!first.from_cache);
        assert_eq!(first.content_hash, req.content_hash());
        assert_eq!(first.sample_rate(), 24_000);
        assert_eq!(first.duration_ms(), 13 * 200);
        assert_eq!(decode_wav(&first.wav).unwrap().len(), first.audio.len());
        assert_eq!(cache.len(), 1);

        let second = synthesizer.synthesize(&req).await.unwrap();
        assert!(second.from_cache);
        assert_eq!(second.wav, first.wav);
        assert_eq!(backend.calls(), 1);
    }

    #[tokio::test]
    async fn an_invalid_request_never_reaches_the_backend() {
        let (backend, _, synthesizer) = setup(FakeBackend::new());
        let mut req = request(EngineId::Index2, "你好。");
        req.voice = Voice::Design {
            description: "沉稳的女声".to_owned(),
        };
        assert_eq!(
            synthesizer.synthesize(&req).await,
            Err(TtsError::InvalidRequest(
                RequestError::VoiceDesignUnsupported {
                    engine: EngineId::Index2
                }
            ))
        );
        assert_eq!(backend.calls(), 0);
    }

    #[tokio::test]
    async fn long_text_is_split_for_breeze_but_not_for_indextts() {
        // 400 个字，句号在第 200 和第 400 个字后面；Breeze 单段最多 350 字
        let text = format!("{}。{}。", "好".repeat(199), "的".repeat(199));

        let (backend, _, synthesizer) = setup(FakeBackend::new());
        let breeze = synthesizer
            .synthesize(&request(EngineId::Breeze2, &text))
            .await
            .unwrap();
        assert_eq!(backend.calls(), 2);
        assert!(backend.texts().iter().all(|t| t.chars().count() == 200));
        // 两段各 200 字，中间有默认 200 毫秒的停顿
        assert_eq!(breeze.duration_ms(), 2 * 200 * 200 + 200);

        let (backend, _, synthesizer) = setup(FakeBackend::new());
        let index = synthesizer
            .synthesize(&request(EngineId::Index2, &text))
            .await
            .unwrap();
        assert_eq!(backend.calls(), 1);
        assert_eq!(index.duration_ms(), 400 * 200);
    }

    #[tokio::test]
    async fn silent_audio_is_retried_with_another_seed_then_reported() {
        let (backend, cache, synthesizer) = setup(FakeBackend::new().silent_on("坏"));
        let req = request(EngineId::Index2, "这句第一次会坏。");
        let result = synthesizer.synthesize(&req).await.unwrap();
        assert!(result.audio.peak() > 0.1);
        assert_eq!(backend.calls(), 2);
        assert_eq!(cache.len(), 1);

        let (backend, cache, synthesizer) = setup(FakeBackend::new().silent_on("坏"));
        let synthesizer = synthesizer.with_max_attempts(1);
        let err = synthesizer.synthesize(&req).await.unwrap_err();
        assert!(matches!(err, TtsError::BadAudio(_)), "{err}");
        assert_eq!(backend.calls(), 1);
        // 坏音频不进缓存
        assert!(cache.is_empty());
    }

    #[tokio::test]
    async fn batch_keeps_order_isolates_failures_and_synthesizes_duplicates_once() {
        let (backend, _, synthesizer) = setup(FakeBackend::new().failing_on("炸"));
        let outcomes = synthesizer
            .synthesize_batch(vec![
                item("q1", EngineId::Index2, "请先做一个简单的自我介绍。"),
                item("q2", EngineId::Index2, "这一题会炸。"),
                item("q3", EngineId::Index2, "你为什么想换工作？"),
                item("q4", EngineId::Index2, "请先做一个简单的自我介绍。"),
                item("q5", EngineId::Index2, "  "),
            ])
            .await;

        let keys: Vec<&str> = outcomes.iter().map(|o| o.key.as_str()).collect();
        assert_eq!(keys, ["q1", "q2", "q3", "q4", "q5"]);
        assert!(outcomes[0].result.is_ok());
        assert!(matches!(outcomes[1].result, Err(TtsError::Backend(_))));
        assert!(outcomes[2].result.is_ok());
        let (first, repeat) = (
            outcomes[0].result.as_ref().unwrap(),
            outcomes[3].result.as_ref().unwrap(),
        );
        assert_eq!(repeat.wav, first.wav);
        assert!(repeat.from_cache);
        assert_eq!(
            outcomes[4].result,
            Err(TtsError::InvalidRequest(RequestError::EmptyText))
        );
        // q1、q2、q3 各调一次后端；q4 和 q1 相同，q5 没通过检查
        assert_eq!(backend.calls(), 3);
    }

    #[tokio::test]
    async fn batch_deduplicates_even_without_a_cache() {
        let backend = Arc::new(FakeBackend::new());
        let synthesizer = Synthesizer::new(backend.clone());
        let outcomes = synthesizer
            .synthesize_batch(vec![
                item("a", EngineId::Index25, "同一句话。"),
                item("b", EngineId::Index25, "同一句话。"),
            ])
            .await;
        assert!(outcomes.iter().all(|o| o.result.is_ok()));
        assert_eq!(backend.calls(), 1);
    }

    #[tokio::test]
    async fn stream_gives_the_same_audio_as_the_whole_synthesis() {
        let text = format!("{}。{}。", "好".repeat(199), "的".repeat(199));
        for engine in [EngineId::Index2, EngineId::Breeze2] {
            let (_, _, synthesizer) = setup(FakeBackend::new());
            let req = request(engine, &text);
            let mut stream = synthesizer.synthesize_stream(&req).await.unwrap();
            let mut joined: Option<Audio> = None;
            let mut chunks = 0;
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.unwrap();
                chunks += 1;
                match &mut joined {
                    None => joined = Some(chunk),
                    Some(whole) => whole.append(&chunk).unwrap(),
                }
            }
            assert!(chunks > 10, "{engine}");
            let whole = synthesizer.synthesize(&req).await.unwrap();
            assert_eq!(joined.unwrap(), whole.audio, "{engine}");
        }
    }

    #[tokio::test]
    async fn stream_reports_a_failing_segment_and_uses_the_cache() {
        let (_, _, synthesizer) = setup(FakeBackend::new().failing_on("炸"));
        let text = format!("{}。这里会炸。", "好".repeat(349));
        let mut stream = synthesizer
            .synthesize_stream(&request(EngineId::Breeze2, &text))
            .await
            .unwrap();
        let mut saw_audio = false;
        let mut saw_error = false;
        while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(_) => saw_audio = true,
                Err(_) => saw_error = true,
            }
        }
        assert!(saw_audio && saw_error);

        let (backend, _, synthesizer) = setup(FakeBackend::new());
        let req = request(EngineId::Index2, "这句已经合成过了。");
        synthesizer.synthesize(&req).await.unwrap();
        let mut stream = synthesizer.synthesize_stream(&req).await.unwrap();
        assert!(stream.next().await.unwrap().is_ok());
        assert!(stream.next().await.is_none());
        assert_eq!(backend.calls(), 1);
    }

    /// 读写都失败的缓存。
    struct BrokenCache;

    #[async_trait]
    impl AudioCache for BrokenCache {
        async fn get(&self, _: &str) -> Result<Option<Vec<u8>>, CacheError> {
            Err(CacheError("磁盘坏了".to_owned()))
        }
        async fn put(&self, _: &str, _: &[u8]) -> Result<(), CacheError> {
            Err(CacheError("磁盘坏了".to_owned()))
        }
    }

    #[tokio::test]
    async fn a_broken_or_corrupt_cache_does_not_break_synthesis() {
        let backend = Arc::new(FakeBackend::new());
        let synthesizer = Synthesizer::new(backend.clone()).with_cache(Arc::new(BrokenCache));
        let req = request(EngineId::Index2, "你好。");
        assert!(synthesizer.synthesize(&req).await.is_ok());

        let (backend, cache, synthesizer) = setup(FakeBackend::new());
        cache.put(&req.content_hash(), b"not a wav").await.unwrap();
        let result = synthesizer.synthesize(&req).await.unwrap();
        assert!(!result.from_cache);
        assert_eq!(backend.calls(), 1);
        // 重新合成后坏文件被覆盖
        assert!(synthesizer.synthesize(&req).await.unwrap().from_cache);
    }

    #[test]
    fn audio_sanity_check_catches_the_obvious_failures() {
        let good = Audio::tone(24_000, 220.0, 2_000, 0.3).unwrap();
        assert_eq!(check_audio(&good, "请做一个自我介绍。"), Ok(()));
        let empty = Audio::new(24_000, vec![]).unwrap();
        assert!(check_audio(&empty, "你好").is_err());
        let silent = Audio::silence(24_000, 2_000).unwrap();
        assert!(check_audio(&silent, "你好").is_err());
        let nan = Audio::new(24_000, vec![0.1, f32::NAN]).unwrap();
        assert!(check_audio(&nan, "你好").is_err());
        // 100 个字只有 2 秒
        assert!(check_audio(&good, &"好".repeat(100)).is_err());
        // 标点不算字
        assert_eq!(check_audio(&good, &"。".repeat(100)), Ok(()));
    }
}
