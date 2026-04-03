# ig brain — Persistent AI Memory via brain.dev

Connect ig to brain.dev for persistent memory across Claude Code sessions. Memories are injected **before** tokens are spent, saving 10-20x per prompt.

---

## Table of Contents

1. [Installation](#installation)
2. [Quick Start](#quick-start)
3. [Commands](#commands)
4. [Hooks](#hooks)
5. [Configuration](#configuration)
6. [How It Works](#how-it-works)
7. [Usage Examples](#usage-examples)
8. [Debugging](#debugging)
9. [Uninstall / Reset](#uninstall--reset)
10. [FAQ](#faq)

---

## Installation

### Prerequisites

- ig v1.6.2+ installed (`ig --version`)
- A brain.dev account (https://brain.dev)
- Claude Code installed (`claude --version`)

### Step 1: Update ig

```bash
ig update
# or reinstall:
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash
```

### Step 2: Login to brain.dev

```bash
ig brain login
```

This will prompt you to paste your API token:

```
brain.dev — API Token Setup

1. Go to https://brain.dev/settings/tokens
2. Create a new token
3. Paste it here:

Token: brn_a1b2c3d4e5f6...

✓ Connected to brain.dev
```

### Step 3: Install hooks (automatic)

After login, run `ig setup` to register the brain hooks in Claude Code:

```bash
ig setup
```

This installs two hooks:
- `brain-inject.sh` — injects memories before each prompt (UserPromptSubmit)
- `brain-capture.sh` — captures observations after edits (PostToolUse)

### Step 4: Verify

```bash
ig brain status
```

Expected output:
```
brain.dev — Connected
  User: kev@brain.dev
  Plan: Pro
  Last sync: just now
```

---

## Quick Start

```bash
# 1. Login
ig brain login

# 2. Install hooks
ig setup

# 3. Sync your existing knowledge
ig brain sync

# 4. Use Claude Code normally — brain.dev injects memories automatically
claude "fix the auth bug"
# → [brain.dev] Injecting 3 relevant memories...
#   • Auth uses JWT + refresh tokens (TokenService)
#   • Last fix: rate limiting on /api/auth/refresh
#   • Auth middleware: src/middleware/auth.ts
```

---

## Commands

### `ig brain login`

Connect ig to your brain.dev account.

```bash
ig brain login
```

**What it does:**
- Prompts for your API token (get one at brain.dev/settings/tokens)
- Saves the token to `~/.config/brain/config.json`
- Registers brain hooks in Claude Code (via `ig setup`)

**Token format:** `brn_` followed by 32 hex characters.

---

### `ig brain sync`

Push local knowledge to brain.dev.

```bash
ig brain sync
```

**What it syncs:**
- `~/.claude/MEMORY.md` entries → memories on brain.dev
- Token savings history (`ig gain` data) → project analytics
- Current project info → project registration

**Output:**
```
Syncing to brain.dev...
  Memories: 12 synced
  Stats: 2,835 commands, 5.9MB saved
✓ Sync complete
```

---

### `ig brain pull`

Download skills and rules from brain.dev to your local project.

```bash
ig brain pull
```

**What it pulls:**
- Skills assigned to your org → `.brain/skills/`
- Rules for the current project → `.claude/rules/`

**Output:**
```
Pulling from brain.dev...
  Skills: 5 downloaded → .brain/skills/
  Rules: 3 downloaded → .claude/rules/
✓ Pull complete
```

---

### `ig brain status`

Check connection status and account info.

```bash
ig brain status
```

**Output:**
```
brain.dev — Connected
  User: kev@brain.dev
  Plan: Pro
  Org: Brain Dev
  Last sync: 2h ago
  Memories: 47
  Skills: 12
```

**If not connected:**
```
Error: not logged in — run `ig brain login` first
```

---

## Hooks

### brain-inject.sh (UserPromptSubmit)

**Location:** `~/.claude/hooks/brain-inject.sh`

**When it fires:** Before every prompt you send to Claude Code.

**What it does:**
1. Reads your prompt from stdin
2. Sends it to `brain.dev/api/v1/brain/search` (semantic search)
3. Gets the top 3 most relevant memories
4. Returns them as `additionalContext` — Claude sees them before spending tokens

**Timeout:** 2 seconds. If brain.dev is slow or offline, the hook exits silently and Claude works normally.

**Example injection:**
```
[brain.dev] Relevant project memory:
  • Auth uses JWT + refresh tokens. TokenService in src/auth/service.ts
  • Last auth fix (2026-03-28): rate limiting on /api/auth/refresh
  • Deployment uses Vercel (frontend) + Cloudflare Workers (API)
```

### brain-capture.sh (PostToolUse)

**Location:** `~/.claude/hooks/brain-capture.sh`

**When it fires:** After every successful Edit or Write operation by Claude.

**What it does:**
1. Detects the file that was modified
2. Sends a fire-and-forget POST to brain.dev with the file path + project name
3. brain.dev stores it as an "auto-capture" memory

**Non-blocking:** Runs in the background. Never slows down Claude.

---

## Configuration

### Config file

```
~/.config/brain/config.json
```

```json
{
  "token": "brn_a1b2c3d4e5f6...",
  "api_url": "https://brain.dev/api/v1",
  "auto_sync": true
}
```

| Field | Description | Default |
|-------|-------------|---------|
| `token` | API token from brain.dev | — (required) |
| `api_url` | brain.dev API base URL | `https://brain.dev/api/v1` |
| `auto_sync` | Auto-sync on `ig brain` commands | `true` |

### Hook files

| File | Hook Event | Purpose |
|------|-----------|---------|
| `~/.claude/hooks/brain-inject.sh` | UserPromptSubmit | Inject memories before prompts |
| `~/.claude/hooks/brain-capture.sh` | PostToolUse (Edit\|Write) | Capture observations |

### Environment variables (optional)

| Variable | Description |
|----------|-------------|
| `BRAIN_TOKEN` | Override token (for CI/CD) |
| `BRAIN_API_URL` | Override API URL |
| `BRAIN_DISABLE` | Set to `1` to disable hooks |

---

## How It Works

```
You type a prompt
       │
       ▼
┌─────────────────────────────┐
│ Claude Code receives prompt │
│                             │
│ BEFORE sending to API:      │
│ UserPromptSubmit hook fires │
│       │                     │
│       ▼                     │
│ brain-inject.sh             │
│  → curl brain.dev/search    │
│  → semantic match on prompt │
│  → returns top-3 memories   │
│       │                     │
│       ▼                     │
│ additionalContext injected  │
│ into Claude's system prompt │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│ Claude starts with context  │
│ → knows the codebase        │
│ → knows past decisions      │
│ → codes directly            │
│                             │
│ Instead of ~50K tokens      │
│ exploring, uses ~5K tokens  │
│ = 90% savings               │
└─────────────────────────────┘
```

### Memory sources

| Source | How | Example |
|--------|-----|---------|
| **Manual** | Create on brain.dev dashboard | "Auth uses JWT + refresh tokens" |
| **Auto-capture** | brain-capture.sh hook | "Modified src/auth/service.ts" |
| **Import** | `ig brain sync` from MEMORY.md | Existing Claude Code memories |
| **API** | POST /api/v1/memories | Programmatic from scripts |

### Semantic search

brain.dev uses embeddings (CF Workers AI, bge-base-en-v1.5, 768 dims) for semantic search. When you type "fix the auth bug", it finds memories about auth even if they don't contain the word "fix" or "bug".

---

## Usage Examples

### Example 1: First-time setup

```bash
# Install ig
curl -fsSL https://raw.githubusercontent.com/MakFly/instant-grep/main/install.sh | bash

# Login to brain.dev
ig brain login

# Sync existing knowledge
ig brain sync

# Verify
ig brain status
```

### Example 2: Import team skills

```bash
# Pull shared skills from your team
ig brain pull

# Check what was downloaded
ls .brain/skills/
# auth-jwt-setup.md  code-review.md  commit-convention.md
```

### Example 3: Manual memory creation

On brain.dev dashboard (`/memories`):
- Click "New Memory"
- Content: "The payment service uses Stripe webhooks. Endpoint: /api/webhooks/stripe. Secret in STRIPE_WEBHOOK_SECRET env var."
- Tags: payment, stripe, webhooks
- Save

Next time you ask Claude about payments, this memory is automatically injected.

### Example 4: Check token savings

```bash
ig gain
# Shows how many tokens brain.dev saved you
```

---

## Debugging

### Hook not firing

```bash
# Check hooks are registered
cat ~/.claude/settings.json | python3 -m json.tool | grep brain

# Expected: brain-inject.sh in UserPromptSubmit hooks
# Expected: brain-capture.sh in PostToolUse hooks
```

If not registered:
```bash
ig setup  # re-registers all hooks
```

### brain-inject.sh not returning memories

```bash
# Test manually
echo '{"prompt":"fix auth bug"}' | bash ~/.claude/hooks/brain-inject.sh
# Should output: {"additionalContext": "..."}
```

If empty, check:
```bash
# Is the config valid?
cat ~/.config/brain/config.json

# Is the API reachable?
curl -s https://brain.dev/api/v1/brain/search \
  -H "Authorization: Bearer $(cat ~/.config/brain/config.json | python3 -c 'import sys,json;print(json.load(sys.stdin)["token"])')" \
  -H "Content-Type: application/json" \
  -d '{"query":"test"}' | python3 -m json.tool
```

### brain-capture.sh not capturing

```bash
# Test manually
echo '{"tool_name":"Edit","tool_input":{"file_path":"test.ts"}}' | bash ~/.claude/hooks/brain-capture.sh
# Should exit 0 silently (fire-and-forget)
```

### Timeout issues

The inject hook has a 2-second timeout. If brain.dev is slow:
```bash
# Check latency
time curl -s https://brain.dev/api/v1/brain/search \
  -H "Authorization: Bearer YOUR_TOKEN" \
  -d '{"query":"test"}'
```

If consistently >2s, check your network or brain.dev status.

### Config issues

```bash
# View current config
cat ~/.config/brain/config.json

# Reset config
rm ~/.config/brain/config.json
ig brain login  # re-login
```

### Logs

brain-inject.sh writes to stderr on errors (visible in Claude Code's hook output):
```bash
# Run Claude with verbose hook output
claude --verbose "test prompt"
```

---

## Uninstall / Reset

### Remove brain.dev connection (keep ig)

```bash
# Delete config
rm -f ~/.config/brain/config.json

# Remove hooks
rm -f ~/.claude/hooks/brain-inject.sh
rm -f ~/.claude/hooks/brain-capture.sh

# Clean hooks from settings.json (ig setup will do this on next run)
ig setup
```

### Remove hooks only (keep config)

```bash
rm -f ~/.claude/hooks/brain-inject.sh
rm -f ~/.claude/hooks/brain-capture.sh
```

### Delete local brain data

```bash
# Remove local .brain/ directory (skills, artifacts)
rm -rf .brain/

# Remove config
rm -rf ~/.config/brain/
```

### Delete brain.dev account data

1. Go to brain.dev/settings
2. Click "Delete Account"
3. This removes all memories, skills, rules, and projects from the cloud

### Complete uninstall (ig + brain)

```bash
# Remove brain config
rm -rf ~/.config/brain/

# Remove brain hooks
rm -f ~/.claude/hooks/brain-inject.sh
rm -f ~/.claude/hooks/brain-capture.sh

# Uninstall ig entirely
ig uninstall
```

---

## FAQ

**Q: Does brain.dev store my code?**
A: No. brain.dev stores only memory summaries (text), skills (markdown), and rules. No raw source code is uploaded. Auto-capture only stores file paths, not file contents.

**Q: What happens if brain.dev is down?**
A: The inject hook has a 2-second timeout. If unreachable, it exits silently and Claude works normally — just without memory injection.

**Q: Can I use brain.dev without ig?**
A: Yes. brain.dev has a web dashboard where you can manage memories, skills, and rules manually. ig is the CLI bridge that automates injection.

**Q: How much does it cost?**
A: Free plan: 100 memories, 10 skills, 1 project. Pro: $8/month for unlimited. Team: $20/user/month.

**Q: Does it work with Cursor / other AI tools?**
A: brain.dev stores knowledge accessible via API. Currently hooks are built for Claude Code. Cursor/Copilot integration is planned.

**Q: Can I self-host brain.dev?**
A: Not yet. Self-hosting is planned for v2.

**Q: Where is my data stored?**
A: brain.dev uses Turso (SQLite edge) for structured data and Cloudflare R2 for assets. All data is in your account, exportable anytime.
