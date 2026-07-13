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

/**
 * Build the ordered list of docx paragraphs from a Summary.
 * The Summary is a map of sectionKey -> { title, blocks[] }; a special "title"
 * key (if present) is treated as the document title, matching convertToMarkdown.
 */
function buildBody(summary: Summary): Paragraph[] {
  const paragraphs: Paragraph[] = [];
  let sectionIndex = 0;

  for (const [key, section] of Object.entries(summary)) {
    if (!section) continue;
    if (key === 'title') continue; // document title handled separately
    const blocks = section.blocks ?? [];
    if (blocks.length === 0) continue;

    sectionIndex += 1;
    let h1Index = 0; // 二级 （一）
    let h2Index = 0; // 三级 1.

    // 一级标题: "一、<section title>" 黑体
    const sectionTitle = cleanText(section.title || key).replace(/[:：]$/, '');
    paragraphs.push(
      headingParagraph(`${toChineseNumber(sectionIndex)}、${sectionTitle}`, FONT_HEI)
    );

    for (const block of blocks) {
      const content = cleanText(block.content);
      if (!content) continue;

      switch (block.type) {
        case 'heading1': {
          h1Index += 1;
          h2Index = 0; // reset 三级 numbering under a new 二级 heading
          paragraphs.push(
            headingParagraph(`${toParenChineseNumber(h1Index)}${content}`, FONT_KAI)
          );
          break;
        }
        case 'heading2': {
          h2Index += 1;
          // 三级标题用仿宋（与正文同字体），形如 "1."
          paragraphs.push(headingParagraph(`${h2Index}.${content}`, FONT_FANGSONG));
          break;
        }
        case 'bullet':
        case 'text':
        default:
          paragraphs.push(bodyParagraph(content));
          break;
      }
    }
  }

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

  const bodyParagraphs = buildBody(summary);

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
