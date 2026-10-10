#!/usr/bin/env node
/**
 * prompt-report — write a Markdown report of the results of the last prompt
 * executed by Claude Code in this project.
 *
 * Reads the session transcript Claude Code keeps at
 *   ~/.claude/projects/<slugified-cwd>/<session-id>.jsonl
 * finds the most recent human-typed prompt, and renders everything that
 * happened after it: the answer, every tool call, the files touched, token
 * usage and timings.
 *
 * Usage:
 *   npm run review:package
 *   npm run review:package -- --stdout
 *   npm run review:package -- --list
 *   npm run review:package -- --prev 1 --thinking --out docs/run.md
 *
 * Flags:
 *   --out <path>        output file (default docs/prompt-runs/<ts>-<slug>.md)
 *   --stdout            print to stdout instead of writing a file
 *   --list              list the prompts in the session and exit
 *   --prev <n>          report the nth prompt back (0 = last, default 0)
 *   --session <id>      use a specific session id instead of the newest
 *   --dir <path>        transcript directory override
 *   --project <path>    project dir whose transcripts to read (default cwd)
 *   --thinking          include the model's thinking blocks
 *   --max-result <n>    chars of each tool result to keep (default 1200)
 *   --no-git            skip the git status/diffstat section
 */

import { readFileSync, readdirSync, statSync, writeFileSync, mkdirSync } from 'node:fs';
import { join, resolve, dirname, isAbsolute, relative } from 'node:path';
import { homedir } from 'node:os';
import { execFileSync } from 'node:child_process';

// ---------------------------------------------------------------- args

function parseArgs(argv) {
  const opts = {
    out: null, stdout: false, list: false, prev: 0, session: null,
    dir: null, project: process.cwd(), thinking: false, maxResult: 1200, git: true,
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    const next = () => {
      const v = argv[++i];
      if (v === undefined) die(`${a} needs a value`);
      return v;
    };
    switch (a) {
      case '--out': opts.out = next(); break;
      case '--stdout': opts.stdout = true; break;
      case '--list': opts.list = true; break;
      case '--prev': opts.prev = Number(next()); break;
      case '--session': opts.session = next(); break;
      case '--dir': opts.dir = next(); break;
      case '--project': opts.project = next(); break;
      case '--thinking': opts.thinking = true; break;
      case '--max-result': opts.maxResult = Number(next()); break;
      case '--no-git': opts.git = false; break;
      case '-h': case '--help': usage(); process.exit(0);
      default: die(`unknown flag: ${a}`);
    }
  }
  if (!Number.isInteger(opts.prev) || opts.prev < 0) die('--prev must be a non-negative integer');
  if (!Number.isFinite(opts.maxResult) || opts.maxResult < 0) die('--max-result must be a number');
  return opts;
}

function usage() {
  const src = readFileSync(new URL(import.meta.url), 'utf8');
  const header = src.slice(src.indexOf('/**'), src.indexOf('*/'));
  console.log(header.split('\n').map((l) => l.replace(/^\s*(\/\*\*|\*\/?)\s?/, '')).join('\n').trim());
}

function die(msg) {
  console.error(`prompt-report: ${msg}`);
  process.exit(1);
}

// ---------------------------------------------------- transcript loading

/** Claude Code slugifies the project path into the transcript directory name. */
function slugifyProjectPath(p) {
  return resolve(p).replace(/[^a-zA-Z0-9]/g, '-');
}

function transcriptDir(opts) {
  if (opts.dir) return resolve(opts.dir);
  const base = process.env.CLAUDE_CONFIG_DIR
    ? resolve(process.env.CLAUDE_CONFIG_DIR)
    : join(homedir(), '.claude');
  return join(base, 'projects', slugifyProjectPath(opts.project));
}

function pickTranscript(dir, sessionId) {
  let entries;
  try {
    entries = readdirSync(dir).filter((f) => f.endsWith('.jsonl'));
  } catch {
    die(`no transcripts found at ${dir}\n  (pass --dir or --project if the project lives elsewhere)`);
  }
  if (!entries.length) die(`no .jsonl transcripts in ${dir}`);
  if (sessionId) {
    const hit = entries.find((f) => f === `${sessionId}.jsonl` || f.startsWith(sessionId));
    if (!hit) die(`no transcript matching session "${sessionId}" in ${dir}`);
    return join(dir, hit);
  }
  return entries
    .map((f) => join(dir, f))
    .sort((a, b) => statSync(b).mtimeMs - statSync(a).mtimeMs)[0];
}

function readTranscript(file) {
  return readFileSync(file, 'utf8')
    .split('\n')
    .filter((l) => l.trim())
    .map((l) => { try { return JSON.parse(l); } catch { return null; } })
    .filter(Boolean);
}

// ------------------------------------------------------------- analysis

const INTERRUPT_MARKER = '[Request interrupted by user';

function textOf(content) {
  if (typeof content === 'string') return content;
  if (!Array.isArray(content)) return '';
  return content.filter((b) => b.type === 'text').map((b) => b.text).join('\n');
}

/** A human-typed prompt: a user entry on the main chain with plain string content. */
function isHumanPrompt(e) {
  if (e.type !== 'user' || e.isSidechain) return false;
  if (typeof e.message?.content !== 'string') return false;
  if (e.origin && e.origin.kind !== 'human') return false;
  if (!e.origin && e.promptSource !== 'typed') return false;
  const t = e.message.content.trim();
  if (!t || t.startsWith('<local-command-stdout>')) return false;
  return true;
}

/** Slash-command invocations arrive wrapped in <command-name> tags. */
function promptLabel(raw) {
  const cmd = raw.match(/<command-name>([^<]+)<\/command-name>/);
  if (cmd) {
    const args = raw.match(/<command-args>([^<]*)<\/command-args>/);
    return `${cmd[1].trim()}${args && args[1].trim() ? ` ${args[1].trim()}` : ''}`.trim();
  }
  return raw.trim();
}

function findPrompts(entries) {
  return entries.flatMap((e, i) => (isHumanPrompt(e)
    ? [{
        index: i,
        uuid: e.uuid,
        promptId: e.promptId,
        timestamp: e.timestamp,
        raw: e.message.content,
        label: promptLabel(e.message.content),
      }]
    : []));
}

/** Everything from the chosen prompt up to (not including) the next one. */
function sliceTurn(entries, prompts, prev) {
  if (!prompts.length) die('no human prompts found in this transcript');
  const pos = prompts.length - 1 - prev;
  if (pos < 0) die(`--prev ${prev} is out of range (${prompts.length} prompt(s) in session)`);
  const start = prompts[pos].index;
  const end = pos + 1 < prompts.length ? prompts[pos + 1].index : entries.length;
  return {
    prompt: prompts[pos],
    entries: entries.slice(start + 1, end),
    ordinal: pos + 1,
    total: prompts.length,
  };
}

function analyze(turn) {
  const toolResults = new Map();   // tool_use_id -> result text
  const toolErrors = new Set();
  for (const e of turn.entries) {
    if (e.type !== 'user' || !Array.isArray(e.message?.content)) continue;
    for (const b of e.message.content) {
      if (b.type !== 'tool_result') continue;
      toolResults.set(b.tool_use_id, typeof b.content === 'string' ? b.content : textOf(b.content));
      if (b.is_error) toolErrors.add(b.tool_use_id);
    }
  }

  const answer = [];
  const thinking = [];
  const tools = [];
  const models = new Set();
  const usage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, thinking: 0 };
  let assistantTurns = 0;
  let sidechainEntries = 0;
  let interrupted = false;
  let lastTimestamp = turn.prompt.timestamp;

  for (const e of turn.entries) {
    if (e.timestamp && e.timestamp > lastTimestamp) lastTimestamp = e.timestamp;
    if (e.isSidechain) { sidechainEntries++; continue; }

    if (e.type === 'user' && typeof e.message?.content === 'string'
        && e.message.content.includes(INTERRUPT_MARKER)) interrupted = true;

    if (e.type !== 'assistant' || !Array.isArray(e.message?.content)) continue;
    assistantTurns++;
    if (e.message.model) models.add(e.message.model);
    const u = e.message.usage || {};
    usage.input += u.input_tokens || 0;
    usage.output += u.output_tokens || 0;
    usage.cacheRead += u.cache_read_input_tokens || 0;
    usage.cacheWrite += u.cache_creation_input_tokens || 0;
    usage.thinking += u.output_tokens_details?.thinking_tokens || 0;

    for (const b of e.message.content) {
      if (b.type === 'text' && b.text.trim()) answer.push(b.text.trim());
      else if (b.type === 'thinking' && b.thinking?.trim()) thinking.push(b.thinking.trim());
      else if (b.type === 'tool_use') {
        tools.push({
          id: b.id,
          name: b.name,
          input: b.input || {},
          result: toolResults.get(b.id) ?? null,
          isError: toolErrors.has(b.id),
        });
      }
    }
  }

  return {
    answer, thinking, tools, models: [...models], usage,
    assistantTurns, sidechainEntries, interrupted, lastTimestamp,
  };
}

/** File-mutating tools, in the order they first touched each path. */
function filesTouched(tools) {
  const byPath = new Map();
  const pathKeys = ['file_path', 'notebook_path', 'path'];
  for (const t of tools) {
    if (!/^(Edit|Write|NotebookEdit|MultiEdit)$/.test(t.name)) continue;
    const key = pathKeys.map((k) => t.input[k]).find(Boolean);
    if (!key) continue;
    if (!byPath.has(key)) byPath.set(key, new Set());
    byPath.get(key).add(t.name);
  }
  return [...byPath].map(([path, ops]) => ({ path, ops: [...ops] }));
}

// -------------------------------------------------------------- render

/** Fence that survives bodies which themselves contain backtick fences. */
function fence(body, lang = '') {
  const runs = [...String(body).matchAll(/`{3,}/g)].map((m) => m[0].length + 1);
  const ticks = '`'.repeat(Math.max(3, ...runs, 3));
  return `${ticks}${lang}\n${body}\n${ticks}`;
}

function truncate(s, max) {
  const str = String(s ?? '');
  if (max === 0 || str.length <= max) return { text: str, clipped: false };
  return { text: `${str.slice(0, max)}\n…`, clipped: true };
}

const num = (n) => n.toLocaleString('en-US');

function humanDuration(ms) {
  if (!Number.isFinite(ms) || ms < 0) return 'n/a';
  const s = Math.round(ms / 1000);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${s % 60}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

/** One-line summary of a tool call, per tool shape. */
function toolHeadline(t) {
  const i = t.input;
  switch (t.name) {
    case 'Bash': case 'PowerShell': return i.description || (i.command || '').split('\n')[0];
    case 'Read': case 'Write': case 'Edit': case 'NotebookEdit': return i.file_path || i.notebook_path || '';
    case 'Grep': return `/${i.pattern}/${i.path ? ` in ${i.path}` : ''}`;
    case 'Glob': return i.pattern || '';
    case 'Agent': case 'Task': return i.description || '';
    case 'WebFetch': case 'WebSearch': return i.url || i.query || '';
    case 'Skill': return i.skill || '';
    default: return '';
  }
}

/** Body of a tool call: the interesting inputs, as code where it reads better. */
function toolBody(t, maxResult) {
  const i = t.input;
  const parts = [];
  if (t.name === 'Bash' || t.name === 'PowerShell') {
    parts.push(fence(i.command ?? '', 'sh'));
  } else if (t.name === 'Edit') {
    if (i.old_string !== undefined) parts.push(`**Replaced:**\n${fence(truncate(i.old_string, maxResult).text)}`);
    if (i.new_string !== undefined) parts.push(`**With:**\n${fence(truncate(i.new_string, maxResult).text)}`);
  } else if (t.name === 'Write') {
    parts.push(`**Contents:**\n${fence(truncate(i.content, maxResult).text)}`);
  } else {
    const shown = Object.fromEntries(Object.entries(i).filter(([, v]) => v !== undefined));
    if (Object.keys(shown).length) {
      parts.push(fence(truncate(JSON.stringify(shown, null, 2), maxResult).text, 'json'));
    }
  }
  if (t.result !== null) {
    const { text, clipped } = truncate(t.result, maxResult);
    const label = `${t.isError ? 'Error' : 'Result'}${clipped ? ' (truncated)' : ''}`;
    parts.push(`<details>\n<summary>${label}</summary>\n\n${fence(text)}\n\n</details>`);
  }
  return parts.join('\n\n');
}

function gitSection(cwd) {
  const git = (args) => {
    try {
      return execFileSync('git', args, { cwd, encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] }).trim();
    } catch { return null; }
  };
  if (git(['rev-parse', '--is-inside-work-tree']) !== 'true') return null;
  const status = git(['status', '--short']);
  const diffstat = git(['diff', '--stat']);
  const head = git(['log', '-1', '--pretty=%h %s']);
  const out = [];
  if (head) out.push(`**HEAD:** \`${head}\``);
  out.push(status ? `**Working tree:**\n${fence(status)}` : '**Working tree:** clean');
  if (diffstat) out.push(`**Tracked changes:**\n${fence(diffstat)}`);
  return out.join('\n\n');
}

function render({ turn, info, sessionFile, sessionId, opts }) {
  const started = new Date(turn.prompt.timestamp);
  const ended = new Date(info.lastTimestamp);
  // Not every entry carries cwd/gitBranch (snapshots, attachments) — take the latest that does.
  const last = [...turn.entries].reverse().find((e) => e.cwd || e.gitBranch) || {};
  const cwd = last.cwd || opts.project;
  const files = filesTouched(info.tools);
  const failed = info.tools.filter((t) => t.isError).length;
  /** Paths inside the project read better relative to it. */
  const rel = (p) => {
    const r = relative(cwd, p);
    return !r || r.startsWith('..') || isAbsolute(r) ? p : r.split('\\').join('/');
  };

  const md = [];
  md.push(`# Prompt run — ${started.toISOString().replace('T', ' ').slice(0, 16)} UTC`);
  md.push('');
  md.push('| | |');
  md.push('|---|---|');
  md.push(`| Prompt | ${turn.ordinal} of ${turn.total} in session |`);
  md.push(`| Session | \`${sessionId}\` |`);
  md.push(`| Branch | ${last.gitBranch || 'n/a'} |`);
  md.push(`| Working dir | \`${cwd}\` |`);
  md.push(`| Model | ${info.models.join(', ') || 'n/a'} |`);
  md.push(`| Started | ${started.toISOString()} |`);
  md.push(`| Duration | ${humanDuration(ended - started)} |`);
  md.push(`| Assistant turns | ${info.assistantTurns} |`);
  md.push(`| Tool calls | ${info.tools.length}${failed ? ` (${failed} failed)` : ''} |`);
  if (info.sidechainEntries) md.push(`| Subagent messages | ${info.sidechainEntries} |`);
  md.push(`| Tokens | out ${num(info.usage.output)} · in ${num(info.usage.input)} · cache read ${num(info.usage.cacheRead)} · cache write ${num(info.usage.cacheWrite)} |`);
  md.push(`| Status | ${info.interrupted ? 'interrupted by user' : 'completed'} |`);
  md.push('');

  md.push('## Prompt');
  md.push('');
  md.push(turn.prompt.label.split('\n').map((l) => `> ${l}`).join('\n'));
  md.push('');

  md.push('## Result');
  md.push('');
  md.push(info.answer.length ? info.answer.join('\n\n') : '_No assistant text in this turn._');
  md.push('');

  if (files.length) {
    md.push('## Files touched');
    md.push('');
    for (const f of files) md.push(`- \`${rel(f.path)}\` — ${f.ops.join(', ')}`);
    md.push('');
  }

  if (info.tools.length) {
    md.push('## Tool calls');
    md.push('');
    info.tools.forEach((t, n) => {
      const head = rel(toolHeadline(t));
      md.push(`### ${n + 1}. ${t.name}${head ? ` — ${head}` : ''}${t.isError ? ' ⚠️' : ''}`);
      md.push('');
      const body = toolBody(t, opts.maxResult);
      if (body) { md.push(body); md.push(''); }
    });
  }

  if (opts.thinking && info.thinking.length) {
    md.push('## Thinking');
    md.push('');
    info.thinking.forEach((t, n) => {
      md.push(`<details>\n<summary>Block ${n + 1}</summary>\n\n${fence(t)}\n\n</details>`);
      md.push('');
    });
  }

  if (opts.git) {
    const g = gitSection(cwd);
    if (g) {
      md.push('## Repository state after the run');
      md.push('');
      md.push(g);
      md.push('');
    }
  }

  md.push('---');
  md.push('');
  md.push(`_Generated by \`scripts/prompt-report.mjs\` from \`${sessionFile}\`._`);
  md.push('');
  return md.join('\n');
}

// ---------------------------------------------------------------- main

function defaultOutPath(turn, project) {
  const stamp = new Date(turn.prompt.timestamp).toISOString().replace(/[:.]/g, '-').slice(0, 19);
  const slug = turn.prompt.label.toLowerCase()
    .replace(/[^a-z0-9]+/g, '-').replace(/^-|-$/g, '').slice(0, 50) || 'prompt';
  return join(project, 'docs', 'prompt-runs', `${stamp}-${slug}.md`);
}

function main() {
  const opts = parseArgs(process.argv.slice(2));
  const dir = transcriptDir(opts);
  const sessionFile = pickTranscript(dir, opts.session);
  const entries = readTranscript(sessionFile);
  const prompts = findPrompts(entries);

  if (opts.list) {
    if (!prompts.length) die('no human prompts found in this transcript');
    console.log(`${sessionFile}\n`);
    prompts.forEach((p, i) => {
      const prev = prompts.length - 1 - i;
      const when = p.timestamp ? new Date(p.timestamp).toISOString().slice(0, 19).replace('T', ' ') : '?';
      console.log(`  --prev ${String(prev).padEnd(3)} ${when}  ${p.label.replace(/\s+/g, ' ').slice(0, 90)}`);
    });
    return;
  }

  const turn = sliceTurn(entries, prompts, opts.prev);
  const info = analyze(turn);
  const sessionId = sessionFile.replace(/^.*[\\/]/, '').replace(/\.jsonl$/, '');
  const md = render({ turn, info, sessionFile, sessionId, opts });

  if (opts.stdout) { process.stdout.write(md); return; }
  const out = opts.out
    ? (isAbsolute(opts.out) ? opts.out : resolve(process.cwd(), opts.out))
    : defaultOutPath(turn, opts.project);
  mkdirSync(dirname(out), { recursive: true });
  writeFileSync(out, md, 'utf8');
  console.error(`prompt-report: wrote ${out} (${info.tools.length} tool call(s), ${md.length} bytes)`);
}

main();
