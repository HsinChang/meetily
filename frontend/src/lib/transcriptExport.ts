// transcriptExport.ts
//
// Export a meeting transcript as .docx or .txt, optionally including the
// real-time Chinese translation beneath each segment.

import {
  Document,
  Packer,
  Paragraph,
  TextRun,
  AlignmentType,
} from 'docx';
import type { TranscriptSegmentData } from '@/types';

export type TranscriptExportFormat = 'docx' | 'txt';

/** Format seconds as [MM:SS] (or [HH:MM:SS] past an hour). */
function formatTime(seconds: number | undefined): string {
  if (seconds === undefined || Number.isNaN(seconds)) return '[--:--]';
  const total = Math.max(0, Math.floor(seconds));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => n.toString().padStart(2, '0');
  return h > 0 ? `[${pad(h)}:${pad(m)}:${pad(s)}]` : `[${pad(m)}:${pad(s)}]`;
}

function cleanText(s: string): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

function triggerDownload(blob: Blob, filename: string): void {
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

/** Build a plain-text transcript. */
function buildTxt(
  segments: TranscriptSegmentData[],
  title: string,
  includeTranslation: boolean
): string {
  const lines: string[] = [title, ''];
  for (const seg of segments) {
    const text = cleanText(seg.text) || '[Silence]';
    lines.push(`${formatTime(seg.timestamp)} ${text}`);
    if (includeTranslation && seg.translation && cleanText(seg.translation)) {
      lines.push(`         ${cleanText(seg.translation)}`);
    }
  }
  return lines.join('\n') + '\n';
}

/** Export the transcript as a .txt file. */
export function exportTranscriptTxt(
  segments: TranscriptSegmentData[],
  title: string,
  includeTranslation: boolean
): void {
  const docTitle = cleanText(title) || 'transcript';
  const content = buildTxt(segments, docTitle, includeTranslation);
  triggerDownload(new Blob([content], { type: 'text/plain;charset=utf-8' }), `${docTitle}_transcript.txt`);
}

/** Export the transcript as a .docx file. */
export async function exportTranscriptDocx(
  segments: TranscriptSegmentData[],
  title: string,
  includeTranslation: boolean
): Promise<void> {
  const docTitle = cleanText(title) || 'transcript';

  const titleParagraph = new Paragraph({
    alignment: AlignmentType.CENTER,
    spacing: { after: 240 },
    children: [new TextRun({ text: docTitle, bold: true, size: 32 })],
  });

  const bodyParagraphs: Paragraph[] = [];
  for (const seg of segments) {
    const text = cleanText(seg.text) || '[Silence]';
    bodyParagraphs.push(
      new Paragraph({
        spacing: { after: includeTranslation && seg.translation ? 40 : 120 },
        children: [
          new TextRun({ text: `${formatTime(seg.timestamp)} `, color: '888888', size: 20 }),
          new TextRun({ text, size: 22 }),
        ],
      })
    );
    if (includeTranslation && seg.translation && cleanText(seg.translation)) {
      bodyParagraphs.push(
        new Paragraph({
          indent: { left: 480 },
          spacing: { after: 120 },
          children: [
            new TextRun({ text: cleanText(seg.translation), italics: true, color: '2563EB', size: 22 }),
          ],
        })
      );
    }
  }

  const doc = new Document({
    creator: 'Xin-Meetily',
    title: docTitle,
    sections: [{ children: [titleParagraph, ...bodyParagraphs] }],
  });

  const blob = await Packer.toBlob(doc);
  triggerDownload(blob, `${docTitle}_transcript.docx`);
}
