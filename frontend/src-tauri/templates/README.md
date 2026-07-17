# Meeting Summary Templates

This directory contains template definitions for meeting summary generation.

## Available Templates

These templates target governmental / administrative use and are written to produce
formal Chinese public-document (公文) style output. Section titles become the headings
used by the 公文格式 .docx export.

### 1. `gov_brief.json` — 会议纪要·简要版
Short minutes for routine administrative meetings; fits within 1–2 pages.

**Sections:**
- 会议背景 (background, ≤150 chars)
- 会议要点 (5–8 key points / decisions)

### 2. `gov_detailed.json` — 会议纪要·详细版
Full minutes for formal meetings that must be filed for the record.

**Sections:**
- 会议概况 (time, place, format, chair)
- 参会人员 (attendees table)
- 会议背景 (background)
- 会议经过 (flow of the meeting — the focus of this template)
- 议定事项 (decisions with owners and deadlines)

### 3. `gov_report.json` — 会议报告·呈报版
Detailed report with analysis and recommendations, for submission to supervisors.

**Sections:**
- 会议概况
- 参会人员
- 背景与形势
- 主要内容与讨论
- 议定事项与责任分工
- 分析研判 (analysis / insight)
- 问题与风险
- 下一步工作建议

> Instructions in these templates explicitly forbid inventing names, owners or deadlines
> that are not present in the transcript — unknowns are recorded as “待明确” / “未提及”.

## Template Structure

Each template JSON file follows this schema:

```json
{
  "name": "Template Name",
  "description": "Brief description of the template's purpose",
  "sections": [
    {
      "title": "Section Title",
      "instruction": "Instructions for the LLM on what to extract/include",
      "format": "paragraph|list|string",
      "item_format": "Optional: Markdown table format for list items"
    }
  ]
}
```

## Custom Templates

Users can add custom templates to the application data directory:

- **macOS**: `~/Library/Application Support/Meetily/templates/`
- **Windows**: `%APPDATA%\Meetily\templates\`
- **Linux**: `~/.config/Meetily/templates/`

Custom templates override built-in templates with the same filename.

## Template Fields

### Root Level
- `name` (required): Display name for the template
- `description` (required): Brief explanation of the template's use case
- `sections` (required): Array of section definitions

### Section Object
- `title` (required): Section heading text
- `instruction` (required): LLM guidance for this section
- `format` (required): One of `"paragraph"`, `"list"`, or `"string"`
- `item_format` (optional): Markdown formatting hint for list items (e.g., table structure)
- `example_item_format` (optional): Alternative formatting hint

## Usage in Code

Templates are loaded using the `templates` module:

```rust
use crate::summary::templates;

// Get a specific template
let template = templates::get_template("gov_brief")?;

// List available templates
let available = templates::list_templates();

// Validate custom template JSON
let custom_json = std::fs::read_to_string("custom.json")?;
let validated = templates::validate_template(&custom_json)?;
```
