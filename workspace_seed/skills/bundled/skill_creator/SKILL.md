# skill_creator

## Description
Meta-skill: how to create, update and optimize skills. Follow this guide whenever the user asks to add a new skill or improve an existing one.

## Parameters
- name: skill name — lowercase letters, digits and underscores only (e.g. api_tester)
- action: "create" or "update"

## Example
run_skills("skill_creator")

## Skill storage locations

FastClaw has two tiers of skills, both under the workspace `skills/` directory:

1. **Bundled skills** (read-only): `<workspace>/skills/bundled/<skill_name>/SKILL.md`
   — Shipped with the framework. Do NOT modify them.
2. **User skills**: `<workspace>/skills/user/<skill_name>/SKILL.md`
   — Created by the user. This is where new skills go by default.

Resolve `<workspace>` first via `run_shell`: `echo "${FASTCLAW_WORKSPACE:-$HOME/.fastclaw-rs/workspace}"` (debug builds may set `FASTCLAW_DEV_WORKSPACE` instead). Directories whose name starts with `_` are ignored by the registry.

## Two kinds of skills

- **Meta skill** (document skill): the directory contains only `SKILL.md`. Listing and execution both return the instructional content; the model reads it and follows the steps itself using `run_shell` and other tools. Start here — it's always safe, and changes take effect immediately (no restart).
- **Native skill**: a Rust struct implementing the `Skill` trait (`name()` / `description()` / `execute()`), registered in host code such as `src/api.rs`. It requires recompilation, so the agent cannot create one at runtime; only propose one when the logic genuinely needs code (external API calls, heavy computation, etc.) and let the developer implement it.

## Required SKILL.md structure

```markdown
## Description
One sentence describing what this skill does.

## Parameters
- param1: description of param1
- param2: description of param2

## Example
run_skills("<skill_name>", {"param1": "value1", "param2": "value2"})
```

Optional sections (append AFTER the three required ones when the skill connects to a remote service):

```markdown
## Remote Endpoint
https://example.com/api

## Secret
<the secret or API key required by the endpoint>
```

## How to create a skill

1. Check for name conflicts: `run_skills("__list__")` — if the name already exists (either bundled or user), DO NOT overwrite it unless the user explicitly asked to modify that exact skill. Otherwise pick a different name.
2. Write the file using heredoc (keeps formatting intact), with `<workspace>` resolved above:

```
WS=$(echo "${FASTCLAW_WORKSPACE:-$HOME/.fastclaw-rs/workspace}")
mkdir -p "$WS/skills/user/my_skill"

cat > "$WS/skills/user/my_skill/SKILL.md" <<'EOF'
## Description
...

## Parameters
- ...

## Example
run_skills("my_skill", {...})
EOF
```

3. Verify: `run_skills("__info__", {"skill_name": "my_skill"})` — confirm the content is complete and well-formed. Changes take effect immediately, no restart needed.

## How to update / optimize a skill

1. Read the current version: `run_skills("__info__", {"skill_name": "x"})`.
2. Rewrite the full SKILL.md (same heredoc pattern). Keep the required section structure; preserve existing `## Remote Endpoint` and `## Secret` sections unless the user asked to change them.

## Rules

- Skill names: `^[a-z][a-z0-9_]*$` — no spaces, no dashes, no uppercase.
- Never copy a `## Secret` value into any other file, command output, or message; it may only exist inside its own SKILL.md.
- Keep Description to one line — it appears in every system prompt; details belong in the body (progressive disclosure keeps context small).
- Use `<<'EOF'` (quoted) with heredocs to prevent shell variable expansion.
- ALL skill files (SKILL.md, resources, etc.) must be written inside the skill's own directory (`<workspace>/skills/user/<skill_name>/`). Never write code outside the skill directory.
