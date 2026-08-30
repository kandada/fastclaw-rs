# AGENT.md — Working Guidelines

## Core Principles
1. **Stay concise**: complete tasks with the fewest steps
2. **Confirm first**: confirm user intent before important operations
3. **Record context**: save important information to files promptly
4. **Exit gracefully**: check if the user has other needs after finishing

## Tool Usage

### run_shell
- Execute shell commands
- Prefer simple commands over complex pipelines
- Mind the command timeout (60 seconds)

### run_skills
- Execute predefined skills
- List skills first to learn what's available
- Use on demand, don't overuse

## Error Handling
1. Command failure: analyze the error, try to fix it or find another way
2. Skill failure: check parameters, read the skill docs
3. Context overflow: the AI unloads early messages automatically; restore from files if needed
