import React, { useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Cpu } from 'lucide-react';
import { toast } from 'sonner';
import { useConfig } from '@/contexts/ConfigContext';
import { useTranscriptionModels, ModelOption } from '@/hooks/useTranscriptionModels';
import type { TranscriptModelProps } from '@/components/TranscriptSettings';

/**
 * Transcription model picker for the pre-recording bar.
 *
 * Lets the engine be chosen where the recording actually starts, instead of only in
 * Settings — the model matters most at that moment (e.g. switching to Fun-ASR for a
 * Chinese meeting).
 *
 * The selection is *persisted* to `transcript_settings` rather than applied as a
 * one-shot override: the recording backend resolves its engine from that table
 * (`validate_transcription_model_ready` / `get_or_init_transcription_engine`), so
 * writing there is what actually takes effect, and it keeps this control and the
 * Settings pane showing the same thing.
 */

/** Model-list providers are spelled differently from the stored config provider. */
function toConfigProvider(provider: ModelOption['provider']): TranscriptModelProps['provider'] {
  return provider === 'whisper' ? 'localWhisper' : provider;
}

interface TranscriptionModelPickerProps {
  /** Disabled while a recording is starting or a model is being validated. */
  disabled?: boolean;
  /** Opens the full model manager when no models are installed. */
  onRequestModelSettings?: () => void;
}

export function TranscriptionModelPicker({
  disabled = false,
  onRequestModelSettings,
}: TranscriptionModelPickerProps) {
  const {
    transcriptModelConfig,
    setTranscriptModelConfig,
    selectedLanguage,
    setSelectedLanguage,
  } = useConfig();

  const { availableModels, selectedModelKey, setSelectedModelKey, loadingModels, fetchModels } =
    useTranscriptionModels(transcriptModelConfig);

  useEffect(() => {
    fetchModels();
  }, [fetchModels]);

  const handleChange = useCallback(
    async (key: string) => {
      const model = availableModels.find((m) => `${m.provider}:${m.name}` === key);
      if (!model) return;

      const provider = toConfigProvider(model.provider);
      setSelectedModelKey(key);

      try {
        await invoke('api_save_transcript_config', {
          provider,
          model: model.name,
          apiKey: null,
        });
        setTranscriptModelConfig({ ...transcriptModelConfig, provider, model: model.name });

        // Fun-ASR transcribes Chinese only (the sidecar runs a fixed Chinese prompt and
        // FunAsrProvider rejects other languages), so switching to it with e.g. English
        // selected would fail on every chunk. Move the language across with it.
        if (provider === 'funasr' && selectedLanguage !== 'zh' && selectedLanguage !== 'auto') {
          setSelectedLanguage('zh');
          toast.info('Language set to Chinese', {
            description: 'Fun-ASR transcribes Chinese audio.',
            duration: 4000,
          });
        }
      } catch (err) {
        console.error('Failed to save transcript model selection:', err);
        toast.error('Could not switch transcription model', {
          description: err instanceof Error ? err.message : String(err),
        });
      }
    },
    [
      availableModels,
      selectedLanguage,
      setSelectedLanguage,
      setSelectedModelKey,
      setTranscriptModelConfig,
      transcriptModelConfig,
    ]
  );

  // No installed models: point at the manager rather than showing an empty dropdown.
  if (!loadingModels && availableModels.length === 0) {
    return (
      <div className="flex items-center gap-2 bg-white rounded-full shadow-lg px-3 py-1.5 text-xs">
        <Cpu size={14} className="text-gray-500 flex-shrink-0" />
        <button
          type="button"
          onClick={onRequestModelSettings}
          className="text-blue-600 hover:underline"
        >
          Download a transcription model
        </button>
      </div>
    );
  }

  return (
    <div className="flex items-center gap-2 bg-white rounded-full shadow-lg px-3 py-1.5 text-xs">
      <Cpu size={14} className="text-gray-500 flex-shrink-0" />
      <select
        value={selectedModelKey}
        onChange={(e) => handleChange(e.target.value)}
        disabled={disabled || loadingModels}
        className="bg-transparent text-gray-700 focus:outline-none max-w-[220px] cursor-pointer disabled:opacity-50"
        title="Transcription model"
      >
        {loadingModels && <option value="">Loading models…</option>}
        {availableModels.map((model) => (
          <option key={`${model.provider}:${model.name}`} value={`${model.provider}:${model.name}`}>
            {model.displayName}
          </option>
        ))}
      </select>
    </div>
  );
}
