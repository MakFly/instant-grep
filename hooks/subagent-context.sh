#!/usr/bin/env bash
# SubagentStart hook — inject ig context into every subagent/teammate
cat << 'JSON'
{"hookSpecificOutput":{"hookEventName":"SubagentStart","additionalContext":"Use ig for ALL code search (trigram-indexed, sub-ms). Never use grep, rg, or find. Never use the Grep tool. Start concept searches with: ig symbols | grep KEYWORD + ig -l KEYWORD. Examples: ig \"pattern\" src/, ig read file.rs, ig git status, ig -l \"auth\"."}}
JSON
