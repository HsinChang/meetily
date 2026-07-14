"use client";

import { Transcript, TranscriptSegmentData } from '@/types';
import { TranscriptView } from '@/components/TranscriptView';
import { VirtualizedTranscriptView } from '@/components/VirtualizedTranscriptView';
import { TranscriptButtonGroup } from './TranscriptButtonGroup';
import { useConfig } from '@/contexts/ConfigContext';
import { Popover, PopoverTrigger, PopoverContent } from '@/components/ui/popover';
import { Download } from 'lucide-react';
import { exportTranscriptDocx, exportTranscriptTxt, type TranscriptExportFormat } from '@/lib/transcriptExport';
import { toast } from 'sonner';
import { useMemo, useState } from 'react';

interface TranscriptPanelProps {
  transcripts: Transcript[];
  customPrompt: string;
  onPromptChange: (value: string) => void;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  isRecording: boolean;
  disableAutoScroll?: boolean;

  // Optional pagination props (when using virtualization)
  usePagination?: boolean;
  segments?: TranscriptSegmentData[];
  hasMore?: boolean;
  isLoadingMore?: boolean;
  totalCount?: number;
  loadedCount?: number;
  onLoadMore?: () => void;

  // Retranscription props
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;

  // Meeting title (used for export filenames)
  meetingTitle?: string;
}

export function TranscriptPanel({
  transcripts,
  customPrompt,
  onPromptChange,
  onCopyTranscript,
  onOpenMeetingFolder,
  isRecording,
  disableAutoScroll = false,
  usePagination = false,
  segments,
  hasMore,
  isLoadingMore,
  totalCount,
  loadedCount,
  onLoadMore,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
  meetingTitle,
}: TranscriptPanelProps) {
  const { showTranslation, toggleShowTranslation } = useConfig();
  const [includeTranslation, setIncludeTranslation] = useState(true);

  const handleExportTranscript = (format: TranscriptExportFormat) => {
    const title = meetingTitle?.trim() || 'transcript';
    const withTranslation = hasTranslations && includeTranslation;
    try {
      if (format === 'docx') {
        exportTranscriptDocx(convertedSegments, title, withTranslation)
          .then(() => toast.success('Transcript exported (.docx)'))
          .catch((e) => toast.error('Export failed: ' + (e instanceof Error ? e.message : String(e))));
      } else {
        exportTranscriptTxt(convertedSegments, title, withTranslation);
        toast.success('Transcript exported (.txt)');
      }
    } catch (e) {
      toast.error('Export failed: ' + (e instanceof Error ? e.message : String(e)));
    }
  };

  // Convert transcripts to segments if pagination is not used but we want virtualization
  const convertedSegments = useMemo(() => {
    if (usePagination && segments) {
      return segments;
    }
    // Convert transcripts to segments for virtualization
    return transcripts.map(t => ({
      id: t.id,
      timestamp: t.audio_start_time ?? 0,
      endTime: t.audio_end_time,
      text: t.text,
      confidence: t.confidence,
      translation: t.translation,
    }));
  }, [transcripts, usePagination, segments]);

  const hasTranslations = useMemo(
    () => convertedSegments.some((s) => !!s.translation),
    [convertedSegments]
  );

  return (
    <div className="hidden md:flex md:w-1/4 lg:w-1/3 min-w-0 border-r border-gray-200 bg-white flex-col relative shrink-0">
      {/* Title area */}
      <div className="p-4 border-b border-gray-200">
        <TranscriptButtonGroup
          transcriptCount={usePagination ? (totalCount ?? convertedSegments.length) : (transcripts?.length || 0)}
          onCopyTranscript={onCopyTranscript}
          onOpenMeetingFolder={onOpenMeetingFolder}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onRefetchTranscripts={onRefetchTranscripts}
        />

        {/* Show/hide Chinese translation (only when translations exist) */}
        {hasTranslations && (
          <div className="mt-2 flex items-center justify-center">
            <button
              type="button"
              role="switch"
              aria-checked={showTranslation}
              aria-label="Show Chinese translation"
              onClick={() => toggleShowTranslation(!showTranslation)}
              className="flex items-center gap-1.5 text-xs text-gray-600 hover:text-gray-800"
              title="Show or hide the Chinese translation"
            >
              <span className={showTranslation ? 'text-blue-600 font-medium' : 'text-gray-500'}>译中</span>
              <span
                className={`relative inline-flex h-4 w-7 items-center rounded-full transition-colors ${
                  showTranslation ? 'bg-blue-500' : 'bg-gray-300'
                }`}
              >
                <span
                  className={`inline-block h-3 w-3 transform rounded-full bg-white transition-transform ${
                    showTranslation ? 'translate-x-3.5' : 'translate-x-0.5'
                  }`}
                />
              </span>
              <span>Show translation</span>
            </button>
          </div>
        )}

        {/* Export transcript to .docx / .txt (optionally with translation) */}
        {convertedSegments.length > 0 && (
          <div className="mt-2 flex items-center justify-center">
            <Popover>
              <PopoverTrigger asChild>
                <button
                  type="button"
                  className="flex items-center gap-1.5 text-xs text-gray-600 hover:text-gray-800 px-2 py-1 rounded-md hover:bg-gray-100"
                  title="Export transcript"
                >
                  <Download className="w-3.5 h-3.5" />
                  <span>Export transcript</span>
                </button>
              </PopoverTrigger>
              <PopoverContent align="center" className="w-60 p-3 text-sm">
                <div className="font-medium text-gray-900 mb-2">Export transcript</div>
                {hasTranslations && (
                  <label className="flex items-center gap-2 mb-3 cursor-pointer text-gray-700">
                    <input
                      type="checkbox"
                      checked={includeTranslation}
                      onChange={(e) => setIncludeTranslation(e.target.checked)}
                      className="h-4 w-4 rounded border-gray-300 text-blue-600 focus:ring-blue-500"
                    />
                    <span>Include Chinese translation (译中)</span>
                  </label>
                )}
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => handleExportTranscript('docx')}
                    className="flex-1 px-3 py-1.5 text-sm bg-blue-600 text-white rounded-md hover:bg-blue-700"
                  >
                    .docx
                  </button>
                  <button
                    type="button"
                    onClick={() => handleExportTranscript('txt')}
                    className="flex-1 px-3 py-1.5 text-sm bg-gray-100 text-gray-800 rounded-md hover:bg-gray-200"
                  >
                    .txt
                  </button>
                </div>
              </PopoverContent>
            </Popover>
          </div>
        )}
      </div>

      {/* Transcript content - use virtualized view for better performance */}
      <div className="flex-1 overflow-hidden pb-4">
        <VirtualizedTranscriptView
          segments={convertedSegments}
          isRecording={isRecording}
          isPaused={false}
          isProcessing={false}
          isStopping={false}
          enableStreaming={false}
          showConfidence={true}
          showTranslation={showTranslation}
          disableAutoScroll={disableAutoScroll}
          hasMore={hasMore}
          isLoadingMore={isLoadingMore}
          totalCount={totalCount}
          loadedCount={loadedCount}
          onLoadMore={onLoadMore}
        />
      </div>

      {/* Custom prompt input at bottom of transcript section */}
      {!isRecording && convertedSegments.length > 0 && (
        <div className="p-1 border-t border-gray-200">
          <textarea
            placeholder="Add context for AI summary. For example people involved, meeting overview, objective etc..."
            className="w-full px-3 py-2 border border-gray-200 rounded-md text-sm focus:outline-none focus:ring-1 focus:ring-blue-500 focus:border-blue-500 bg-white shadow-sm min-h-[80px] resize-y"
            value={customPrompt}
            onChange={(e) => onPromptChange(e.target.value)}
          />
        </div>
      )}
    </div>
  );
}
