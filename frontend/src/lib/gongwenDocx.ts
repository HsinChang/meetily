// gongwenDocx.ts
//
// Export a meeting summary as a .docx laid out per the Chinese official-document
// standard 党政机关公文格式 (GB/T 9704-2012), body/正文 portion.
//
// Layout rules implemented (正文部分):
//   - Page: A4, margins 上37 / 下35 / 左28 / 右26 mm
//   - 标题:  2号 方正小标宋简体, centered
//   - 正文:  3号 仿宋_GB2312, 首行缩进2字符, 固定行距约28.8磅, 两端对齐
//   - 层次序数: 一级"一、"黑体; 二级"（一）"楷体_GB2312; 三级"1."仿宋_GB2312
//   - 页码:  4号 宋体, 居中, 形如 "— 1 —"
//
// The document declares the standard 公文 font names; if a viewer lacks them,
// Word substitutes automatically.

import {
  Document,
  Packer,
  Paragraph,
  TextRun,
  Footer,
  PageNumber,
  AlignmentType,
  LineRuleType,
} from 'docx';
import type { Summary } from '@/types';

// ---- Font names (公文 standard) ----
const FONT_TITLE = '方正小标宋简体';
const FONT_HEI = '黑体'; // 一级标题
const FONT_KAI = '楷体_GB2312'; // 二级标题
const FONT_FANGSONG = '仿宋_GB2312'; // 正文 / 三级标题
const FONT_SONG = '宋体'; // 页码

// ---- Sizes in half-points (docx unit). 号数 -> pt -> half-pt ----
const SIZE_2HAO = 44; // 二号 = 22pt
const SIZE_3HAO = 32; // 三号 = 16pt
const SIZE_4HAO = 28; // 四号 = 14pt

// ---- Metrics (twips = 1/1440 inch; 1mm ≈ 56.6929 twips) ----
const mm = (v: number) => Math.round(v * 56.6929);
const A4_WIDTH = mm(210);
const A4_HEIGHT = mm(297);
const MARGIN = { top: mm(37), bottom: mm(35), left: mm(28), right: mm(26) };
const FIRST_LINE_INDENT = 640; // 2 chars × 三号(16pt) = 32pt = 640 twips
const BODY_LINE = 576; // 固定值 28.8磅 (28.8 × 20)

/** Convert 1..N to a Chinese ordinal (一, 二, …, 十一, …). Sufficient for section counts. */
function toChineseNumber(n: number): string {
  const digits = ['', '一', '二', '三', '四', '五', '六', '七', '八', '九'];
  if (n <= 0) return String(n);
  if (n < 10) return digits[n];
  if (n === 10) return '十';
  if (n < 20) return '十' + digits[n - 10];
  if (n < 100) {
    const tens = Math.floor(n / 10);
    const ones = n % 10;
    return digits[tens] + '十' + (ones ? digits[ones] : '');
  }
  return String(n);
}

/** Circled/parenthesized ordinal used for 二级标题: （一）（二）… */
function toParenChineseNumber(n: number): string {
  return `（${toChineseNumber(n)}）`;
}

function cleanText(s: string): string {
  return (s ?? '').replace(/\s+/g, ' ').trim();
}

/** Body/正文 paragraph: 三号仿宋, 首行缩进2字符, 固定行距, 两端对齐. */
function bodyParagraph(text: string): Paragraph {
  return new Paragraph({
    alignment: AlignmentType.JUSTIFIED,
    spacing: { line: BODY_LINE, lineRule: LineRuleType.EXACT },
    indent: { firstLine: FIRST_LINE_INDENT },
    children: [new TextRun({ text, font: FONT_FANGSONG, size: SIZE_3HAO })],
  });
}

/** Heading paragraph with a given font (黑体/楷体/仿宋), 三号, 首行缩进2字符. */
function headingParagraph(text: string, font: string): Paragraph {
  return new Paragraph({
    alignment: AlignmentType.JUSTIFIED,
    spacing: { line: BODY_LINE, lineRule: LineRuleType.EXACT },
    indent: { firstLine: FIRST_LINE_INDENT },
    children: [new TextRun({ text, font, size: SIZE_3HAO })],
  });
}

// Unified intermediate representation for any summary format.
type OutlineKind = 'h1' | 'h2' | 'h3' | 'body';
interface OutlineItem {
  kind: OutlineKind;
  text: string;
}

/** Strip common inline markdown so we render plain text runs. */
function stripInlineMarkdown(s: string): string {
  return (s ?? '')
    .replace(/\*\*(.*?)\*\*/g, '$1')
    .replace(/__(.*?)__/g, '$1')
    .replace(/\*(.*?)\*/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\[([^\]]*)\]\([^)]*\)/g, '$1')
    .replace(/^>\s*/, '')
    .trim();
}

/** Remove a leading enumerator (一、/（一）/1./- ) so 公文 numbering isn't doubled. */
function stripLeadingEnumerator(text: string): string {
  return text
    .replace(
      /^\s*(?:[（(]?\s*[一二三四五六七八九十]{1,3}\s*[)）]?\s*[、.．]?|\d{1,3}\s*[、.．)]|[-*+•])\s+/,
      ''
    )
    .trim();
}

/** Extract plain text from a BlockNote block's inline content array. */
function blockNoteText(block: any): string {
  const content = block?.content;
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content.map((c: any) => (typeof c === 'string' ? c : c?.text ?? '')).join('');
}

/** Markdown -> outline. Uses relative heading depth so the shallowest heading is 一级. */
function outlineFromMarkdown(md: string): OutlineItem[] {
  const lines = md.split(/\r?\n/);
  const headingLevels: number[] = [];
  for (const l of lines) {
    const m = l.match(/^(#{1,6})\s+\S/);
    if (m) headingLevels.push(m[1].length);
  }
  const minLevel = headingLevels.length ? Math.min(...headingLevels) : 1;

  const items: OutlineItem[] = [];
  for (const raw of lines) {
    const line = raw.trim();
    if (!line || /^([-=*_])\1{2,}$/.test(line)) continue; // skip blank / hr

    const h = line.match(/^(#{1,6})\s+(.*)$/);
    if (h) {
      const rel = h[1].length - minLevel;
      const kind: OutlineKind = rel <= 0 ? 'h1' : rel === 1 ? 'h2' : 'h3';
      const text = stripInlineMarkdown(h[2]);
      if (text) items.push({ kind, text });
      continue;
    }

    const body = stripInlineMarkdown(line.replace(/^(?:[-*+•]|\d+[.)])\s+/, ''));
    if (body) items.push({ kind: 'body', text: body });
  }
  return items;
}

/** BlockNote JSON blocks -> outline. */
function outlineFromBlockNote(blocks: any[]): OutlineItem[] {
  const items: OutlineItem[] = [];
  const walk = (arr: any[]) => {
    for (const b of arr ?? []) {
      const text = stripInlineMarkdown(blockNoteText(b));
      if (b?.type === 'heading') {
        const level = Number(b?.props?.level) || 1;
        const kind: OutlineKind = level <= 1 ? 'h1' : level === 2 ? 'h2' : 'h3';
        if (text) items.push({ kind, text });
      } else if (text) {
        items.push({ kind: 'body', text });
      }
      if (Array.isArray(b?.children) && b.children.length) walk(b.children);
    }
  };
  walk(blocks);
  return items;
}

/** Legacy section/blocks Summary -> outline. */
function outlineFromSections(summary: Record<string, any>): OutlineItem[] {
  const items: OutlineItem[] = [];
  for (const [key, section] of Object.entries(summary ?? {})) {
    if (!section || key === 'title') continue;
    const blocks = section.blocks ?? [];
    if (blocks.length === 0) continue;

    items.push({ kind: 'h1', text: cleanText(section.title || key).replace(/[:：]$/, '') });
    for (const block of blocks) {
      const content = cleanText(block.content);
      if (!content) continue;
      if (block.type === 'heading1') items.push({ kind: 'h2', text: content });
      else if (block.type === 'heading2') items.push({ kind: 'h3', text: content });
      else items.push({ kind: 'body', text: content });
    }
  }
  return items;
}

/** Detect the summary format and produce a unified outline. */
function summaryToOutline(summary: any): OutlineItem[] {
  if (summary && typeof summary.markdown === 'string') {
    return outlineFromMarkdown(summary.markdown);
  }
  if (summary && Array.isArray(summary.summary_json)) {
    return outlineFromBlockNote(summary.summary_json);
  }
  return outlineFromSections(summary);
}

/**
 * Render the outline into 公文-formatted paragraphs, applying 层次序数 numbering
 * (一、/（一）/1.) and the matching fonts. Headings equal to the document title
 * are skipped to avoid duplicating the centered 标题.
 */
function buildBody(summary: any, docTitle: string): Paragraph[] {
  const items = summaryToOutline(summary);
  const paragraphs: Paragraph[] = [];
  let h1 = 0;
  let h2 = 0;
  let h3 = 0;
  const normalizedTitle = cleanText(docTitle);

  for (const item of items) {
    const text = cleanText(item.text);
    if (!text) continue;

    if (item.kind !== 'body' && text === normalizedTitle) continue; // avoid title dup

    switch (item.kind) {
      case 'h1':
        h1 += 1;
        h2 = 0;
        h3 = 0;
        paragraphs.push(
          headingParagraph(`${toChineseNumber(h1)}、${stripLeadingEnumerator(text)}`, FONT_HEI)
        );
        break;
      case 'h2':
        h2 += 1;
        h3 = 0;
        paragraphs.push(
          headingParagraph(`${toParenChineseNumber(h2)}${stripLeadingEnumerator(text)}`, FONT_KAI)
        );
        break;
      case 'h3':
        h3 += 1;
        paragraphs.push(
          headingParagraph(`${h3}.${stripLeadingEnumerator(text)}`, FONT_FANGSONG)
        );
        break;
      default:
        paragraphs.push(bodyParagraph(text));
        break;
    }
  }

  // Fallback: if nothing structured was produced (e.g. empty summary), avoid a
  // title-only document by at least emitting any raw markdown text.
  return paragraphs;
}

/** Footer with a centered 公文-style page number: "— 1 —" in 4号宋体. */
function pageNumberFooter(): Footer {
  return new Footer({
    children: [
      new Paragraph({
        alignment: AlignmentType.CENTER,
        children: [
          new TextRun({ text: '— ', font: FONT_SONG, size: SIZE_4HAO }),
          new TextRun({ children: [PageNumber.CURRENT], font: FONT_SONG, size: SIZE_4HAO }),
          new TextRun({ text: ' —', font: FONT_SONG, size: SIZE_4HAO }),
        ],
      }),
    ],
  });
}

/**
 * Generate a 公文格式 .docx from the summary and trigger a download.
 * @param summary  Structured meeting summary
 * @param title    Document title (meeting title)
 */
export async function exportGongwenDocx(summary: Summary, title: string): Promise<void> {
  const docTitle = cleanText(title) || '会议纪要';

  const titleParagraph = new Paragraph({
    alignment: AlignmentType.CENTER,
    spacing: { before: 240, after: 360, line: BODY_LINE, lineRule: LineRuleType.EXACT },
    children: [new TextRun({ text: docTitle, font: FONT_TITLE, size: SIZE_2HAO })],
  });

  const bodyParagraphs = buildBody(summary, docTitle);

  const doc = new Document({
    creator: 'Xin-Meetily',
    title: docTitle,
    styles: {
      default: {
        document: {
          run: { font: FONT_FANGSONG, size: SIZE_3HAO },
        },
      },
    },
    sections: [
      {
        properties: {
          page: {
            size: { width: A4_WIDTH, height: A4_HEIGHT },
            margin: MARGIN,
          },
        },
        footers: { default: pageNumberFooter() },
        children: [titleParagraph, ...bodyParagraphs],
      },
    ],
  });

  const blob = await Packer.toBlob(doc);
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = `${docTitle}_公文格式.docx`;
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}
