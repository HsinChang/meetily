import React, { useState, useEffect, useRef, useCallback } from 'react';
import { listen } from '@tauri-apps/api/event';
import { invoke } from '@tauri-apps/api/core';
import { motion, AnimatePresence } from 'framer-motion';
import { toast } from 'sonner';
import { Check, Download, Trash2, X, AlertCircle, Loader2 } from 'lucide-react';
import {
  FunAsrAPI,
  FunAsrModelInfo,
  FunAsrModelStatus,
  getFunAsrModelDisplayInfo,
  getFunAsrModelDisplayName,
  formatModelSize,
  isModelAvailable,
  isModelDownloading,
  getDownloadProgress,
  getModelError,
} from '../lib/funasr';

interface FunAsrModelManagerProps {
  selectedModel?: string;
  onModelSelect?: (modelName: string) => void;
  className?: string;
  autoSave?: boolean;
}

export function FunAsrModelManager({
  selectedModel,
  onModelSelect,
  className = '',
  autoSave = false,
}: FunAsrModelManagerProps) {
  const [models, setModels] = useState<FunAsrModelInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [initialized, setInitialized] = useState(false);

  const onModelSelectRef = useRef(onModelSelect);
  const autoSaveRef = useRef(autoSave);
  // Throttles progress events so a fast download doesn't re-render on every chunk.
  const progressThrottleRef = useRef<Map<string, { progress: number; timestamp: number }>>(new Map());

  useEffect(() => {
    onModelSelectRef.current = onModelSelect;
    autoSaveRef.current = autoSave;
  }, [onModelSelect, autoSave]);

  useEffect(() => {
    if (initialized) return;

    const initializeModels = async () => {
      try {
        setLoading(true);
        await FunAsrAPI.init();
        setModels(await FunAsrAPI.getAvailableModels());
        setInitialized(true);
      } catch (err) {
        console.error('Failed to initialize Fun-ASR:', err);
        setError(err instanceof Error ? err.message : 'Failed to load models');
        toast.error('Failed to load Fun-ASR models', {
          description: err instanceof Error ? err.message : 'Unknown error',
          duration: 5000,
        });
      } finally {
        setLoading(false);
      }
    };

    initializeModels();
  }, [initialized]);

  const downloadModel = useCallback(async (modelName: string) => {
    const displayName = getFunAsrModelDisplayName(modelName);
    try {
      setModels((prev) =>
        prev.map((m) =>
          m.name === modelName ? { ...m, status: { Downloading: { progress: 0 } } as FunAsrModelStatus } : m
        )
      );
      await FunAsrAPI.downloadModel(modelName);
    } catch (err) {
      console.error('Failed to download Fun-ASR model:', err);
      toast.error(`Failed to download ${displayName}`, {
        description: err instanceof Error ? err.message : String(err),
        duration: 6000,
      });
    }
  }, []);

  useEffect(() => {
    let unlistenProgress: (() => void) | null = null;
    let unlistenComplete: (() => void) | null = null;
    let unlistenError: (() => void) | null = null;

    const setupListeners = async () => {
      unlistenProgress = await listen<{ modelName: string; progress: number }>(
        'funasr-model-download-progress',
        (event) => {
          const { modelName, progress } = event.payload;
          const now = Date.now();
          const throttleData = progressThrottleRef.current.get(modelName);
          const shouldUpdate =
            !throttleData || now - throttleData.timestamp > 300 || Math.abs(progress - throttleData.progress) >= 5;

          if (shouldUpdate) {
            progressThrottleRef.current.set(modelName, { progress, timestamp: now });
            setModels((prev) =>
              prev.map((m) =>
                m.name === modelName ? { ...m, status: { Downloading: { progress } } as FunAsrModelStatus } : m
              )
            );
          }
        }
      );

      unlistenComplete = await listen<{ modelName: string }>('funasr-model-download-complete', (event) => {
        const { modelName } = event.payload;
        progressThrottleRef.current.delete(modelName);
        setModels((prev) =>
          prev.map((m) => (m.name === modelName ? { ...m, status: 'Available' as FunAsrModelStatus } : m))
        );
        toast.success(`${getFunAsrModelDisplayName(modelName)} ready`, { duration: 4000 });
      });

      unlistenError = await listen<{ modelName: string; error: string }>(
        'funasr-model-download-error',
        (event) => {
          const { modelName, error: downloadError } = event.payload;
          progressThrottleRef.current.delete(modelName);
          setModels((prev) =>
            prev.map((m) => (m.name === modelName ? { ...m, status: 'Missing' as FunAsrModelStatus } : m))
          );
          toast.error(`Failed to download ${getFunAsrModelDisplayName(modelName)}`, {
            description: downloadError,
            duration: 6000,
            action: { label: 'Retry', onClick: () => downloadModel(modelName) },
          });
        }
      );
    };

    setupListeners();
    return () => {
      unlistenProgress?.();
      unlistenComplete?.();
      unlistenError?.();
    };
  }, [downloadModel]);

  const saveModelSelection = async (modelName: string) => {
    try {
      await invoke('api_save_transcript_config', {
        provider: 'funasr',
        model: modelName,
        apiKey: null,
      });
    } catch (err) {
      console.error('Failed to save Fun-ASR model selection:', err);
    }
  };

  const handleSelect = async (model: FunAsrModelInfo) => {
    if (!isModelAvailable(model.status)) return;
    if (autoSaveRef.current) await saveModelSelection(model.name);
    onModelSelectRef.current?.(model.name);
  };

  const cancelDownload = async (modelName: string) => {
    try {
      await FunAsrAPI.cancelDownload(modelName);
      setModels((prev) =>
        prev.map((m) => (m.name === modelName ? { ...m, status: 'Missing' as FunAsrModelStatus } : m))
      );
      toast.info('Download cancelled');
    } catch (err) {
      console.error('Failed to cancel Fun-ASR download:', err);
    }
  };

  const deleteModel = async (modelName: string) => {
    try {
      await FunAsrAPI.deleteModel(modelName);
      setModels(await FunAsrAPI.getAvailableModels());
      toast.success(`Deleted ${getFunAsrModelDisplayName(modelName)}`);
    } catch (err) {
      toast.error('Failed to delete model', {
        description: err instanceof Error ? err.message : String(err),
      });
    }
  };

  if (loading) {
    return (
      <div className={`flex items-center justify-center py-8 ${className}`}>
        <Loader2 className="w-5 h-5 animate-spin text-gray-400" />
        <span className="ml-2 text-sm text-gray-500">Loading Fun-ASR models…</span>
      </div>
    );
  }

  if (error) {
    return (
      <div className={`flex items-start gap-2 p-3 rounded-md bg-red-50 text-red-700 text-sm ${className}`}>
        <AlertCircle className="w-4 h-4 mt-0.5 shrink-0" />
        <span>{error}</span>
      </div>
    );
  }

  return (
    <div className={className}>
      <p className="text-xs text-gray-500 mb-3">
        Chinese-first speech recognition. Noticeably more accurate than Whisper on noisy or
        accented Mandarin. Downloads once (~1.2 GB) and runs entirely on your machine.
      </p>

      <div className="space-y-2">
        <AnimatePresence initial={false}>
          {models.map((model) => {
            const display = getFunAsrModelDisplayInfo(model.name);
            const available = isModelAvailable(model.status);
            const downloading = isModelDownloading(model.status);
            const progress = getDownloadProgress(model.status);
            const modelError = getModelError(model.status);
            const isSelected = selectedModel === model.name;

            return (
              <motion.div
                key={model.name}
                initial={{ opacity: 0, y: 4 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0 }}
                onClick={() => handleSelect(model)}
                className={`rounded-lg border p-3 transition-colors ${
                  isSelected ? 'border-blue-500 bg-blue-50' : 'border-gray-200 bg-white'
                } ${available ? 'cursor-pointer hover:border-blue-400' : 'cursor-default'}`}
              >
                <div className="flex items-center justify-between gap-3">
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <span>{display?.icon ?? '🇨🇳'}</span>
                      <span className="font-medium text-sm text-gray-900 truncate">
                        {display?.friendlyName ?? model.name}
                      </span>
                      {display?.recommended && (
                        <span className="text-[10px] px-1.5 py-0.5 rounded bg-blue-100 text-blue-700">
                          Recommended
                        </span>
                      )}
                      {isSelected && <Check className="w-4 h-4 text-blue-600 shrink-0" />}
                    </div>
                    <p className="text-xs text-gray-500 mt-0.5 truncate">
                      {display?.tagline ?? model.description}
                    </p>
                    <p className="text-[11px] text-gray-400 mt-0.5">{formatModelSize(model.size_mb)}</p>
                  </div>

                  <div className="shrink-0 flex items-center gap-2">
                    {downloading ? (
                      <>
                        <span className="text-xs text-gray-600 tabular-nums">{progress}%</span>
                        <button
                          onClick={(e) => {
                            e.stopPropagation();
                            cancelDownload(model.name);
                          }}
                          className="p-1.5 rounded hover:bg-gray-100 text-gray-500"
                          title="Cancel download"
                        >
                          <X className="w-4 h-4" />
                        </button>
                      </>
                    ) : available ? (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          deleteModel(model.name);
                        }}
                        className="p-1.5 rounded hover:bg-red-50 text-gray-400 hover:text-red-600"
                        title="Delete model"
                      >
                        <Trash2 className="w-4 h-4" />
                      </button>
                    ) : (
                      <button
                        onClick={(e) => {
                          e.stopPropagation();
                          downloadModel(model.name);
                        }}
                        className="flex items-center gap-1.5 px-2.5 py-1.5 rounded text-xs font-medium bg-blue-600 text-white hover:bg-blue-700"
                      >
                        <Download className="w-3.5 h-3.5" />
                        Download
                      </button>
                    )}
                  </div>
                </div>

                {downloading && (
                  <div className="mt-2 h-1 w-full rounded bg-gray-200 overflow-hidden">
                    <div
                      className="h-full bg-blue-600 transition-all duration-300"
                      style={{ width: `${progress}%` }}
                    />
                  </div>
                )}

                {modelError && (
                  <div className="mt-2 flex items-start gap-1.5 text-xs text-red-600">
                    <AlertCircle className="w-3.5 h-3.5 mt-px shrink-0" />
                    <span>{modelError}</span>
                  </div>
                )}
              </motion.div>
            );
          })}
        </AnimatePresence>
      </div>
    </div>
  );
}
