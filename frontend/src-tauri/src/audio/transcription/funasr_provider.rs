// audio/transcription/funasr_provider.rs
//
// Fun-ASR-Nano transcription provider.
//
// This is the first implementation of `TranscriptionProvider` to actually be constructed —
// the trait and the `TranscriptionEngine::Provider` variant existed but were dead code.
// The worker loop needs no changes to support it.

use super::provider::{TranscriptionError, TranscriptionProvider, TranscriptResult};
use async_trait::async_trait;
use log::debug;
use std::sync::Arc;

pub struct FunAsrProvider {
    engine: Arc<crate::funasr_engine::FunAsrEngine>,
}

impl FunAsrProvider {
    pub fn new(engine: Arc<crate::funasr_engine::FunAsrEngine>) -> Self {
        Self { engine }
    }
}

#[async_trait]
impl TranscriptionProvider for FunAsrProvider {
    async fn transcribe(
        &self,
        audio: Vec<f32>,
        language: Option<String>,
    ) -> std::result::Result<TranscriptResult, TranscriptionError> {
        // Fun-ASR is driven by a fixed Chinese transcription prompt in the sidecar, so
        // unlike Whisper there is no per-request language switch. "zh", "auto" and unset
        // are all served correctly; anything else would silently return Chinese-biased
        // output, so reject it rather than mislead the caller.
        match language.as_deref() {
            None | Some("zh") | Some("auto") => {}
            Some("auto-translate") => {
                // Whisper's translate-to-English mode has no Fun-ASR equivalent. The
                // separate LLM translation hook in worker.rs covers the Chinese case and
                // deliberately skips when language == "zh".
                return Err(TranscriptionError::UnsupportedLanguage(
                    "auto-translate (Fun-ASR transcribes only; use the translation toggle)".to_string(),
                ));
            }
            Some(other) => {
                return Err(TranscriptionError::UnsupportedLanguage(format!(
                    "{} (Fun-ASR-Nano is configured for Chinese transcription)",
                    other
                )));
            }
        }

        if !self.engine.is_model_loaded().await {
            return Err(TranscriptionError::ModelNotLoaded);
        }

        debug!("Fun-ASR transcribing {} samples", audio.len());

        match self.engine.transcribe_samples(audio).await {
            Ok(text) => Ok(TranscriptResult {
                text: text.trim().to_string(),
                // No confidence score is produced. worker.rs treats `None` as passing the
                // threshold (`confidence_opt.map_or(true, ...)`), which is what we want.
                confidence: None,
                is_partial: false,
            }),
            Err(e) => Err(TranscriptionError::EngineFailed(e.to_string())),
        }
    }

    async fn is_model_loaded(&self) -> bool {
        self.engine.is_model_loaded().await
    }

    async fn get_current_model(&self) -> Option<String> {
        self.engine.get_current_model().await
    }

    fn provider_name(&self) -> &'static str {
        "Fun-ASR"
    }
}
