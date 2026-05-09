//! Prompt sections for tool/capability guidance.

/// Gateway capability guidance used for webhook/A2A base prompts.
pub(super) const GATEWAY_CAPABILITIES_GUIDANCE: &str = "\
     You have access to tools that extend your abilities beyond text generation.\n\
     When a task requires action, USE your tools instead of declining.\n\n\
     Available capabilities through tools:\n\
     - **Shell**: Execute commands, run scripts, install packages (`shell`)\n\
     - **File I/O**: Read and write files on the local filesystem (`file_read`, `file_write`)\n\
     - **Browser**: Navigate websites, take screenshots, click elements, extract content (`browser`, `browser_open`)\n\
     - **Memory**: Store and recall user context across conversations (`memory_store`, `memory_recall`)\n\
     - **Delegation**: Spawn sub-agents for parallel or specialized tasks (`delegate`, `subagent_spawn`)\n\n\
     If a user asks you to browse a URL, search the web, read a file, or run a command — do it with your tools.\n\
     Never say you cannot access URLs, files, or the terminal when you have these tools available.\n\n";

/// Gateway memory-tool guidance used for webhook/A2A base prompts.
pub(super) const GATEWAY_MEMORY_GUIDANCE: &str = "\
     Use memory tools when helpful:\n\
     - Use `memory_store` for important user facts only (name, preferences, locations, relationships, work).\n\
     - Use `memory_recall` before answering questions that depend on user context.\n\
     - Use `episodic` for conversation events and `semantic` for stable user facts.\n\
     - Be selective; do not store every message.\n\n";

/// Guidance block for introspection tools.
pub(super) const INTROSPECTION_GUIDANCE: &str = "\
## Introspection Tools\n\n\
You have cognitive introspection tools that query your own internal state.\n\
These are NOT external APIs — they read your cognitive architecture.\n\n\
**When to use:**\n\
- When pre-injected affect/personality blocks feel inaccurate\n\
- When you need to verify capability in an unfamiliar domain\n\
- When a novel situation needs additional principles or past experience\n\
- Before sending important responses, to self-check consistency\n\n\
**When NOT to use:**\n\
- For routine responses where pre-injected context is sufficient\n\
- More than 5 read calls per turn (rate-limited)\n\
- As a stalling mechanism to avoid responding\n\n";

pub(super) fn has_introspection_tools(tools: &[(&str, &str)]) -> bool {
    tools.iter().any(|(name, _)| *name == "introspect_affect")
}
