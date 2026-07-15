# Tech Radar Bot

Rust bot that collects AI and technology updates, asks OpenAI to classify/summarize them, and sends a Discord morning digest.

## Sources

| Source | Type | URL | Status |
| --- | --- | --- | --- |
| Hacker News | API | `https://hacker-news.firebaseio.com/v0/topstories.json` | Active |
| TLDR AI | RSS | `https://ai.tldr.tech/rss` | Active |
| The Rundown AI | RSS | `https://www.therundown.ai/feed` | Active |
| The New Stack | RSS | `https://thenewstack.io/feed/` | Active |
| InfoQ | RSS | `https://www.infoq.com/feed/` | Active |
| GitHub Trending | Placeholder | `https://github.com/trending` | TODO: no stable official RSS/API |
| Product Hunt | Placeholder API | Product Hunt GraphQL API | TODO: requires `PRODUCT_HUNT_TOKEN` and a stable query |
| Lenny's Newsletter | RSS/Substack | `https://www.lennysnewsletter.com/feed` | Active |
| Every.to AI | Placeholder RSS | User-specific RSS feed | TODO: requires `EVERY_AI_RSS_URL` if using a private subscriber feed |

## Required Environment Variables

- `OPENAI_API_KEY`: OpenAI API key used by the Responses API analysis step.
- `DISCORD_WEBHOOK_URL`: Discord webhook URL used for digest delivery.

Optional future variables:

- `PRODUCT_HUNT_TOKEN`: Required before implementing the Product Hunt API collector.
- `EVERY_AI_RSS_URL`: Required before implementing a private Every.to RSS collector.

## Digest Categories

The OpenAI analysis step returns a fixed category structure that the Discord formatter renders as separate digest embeds:

- AI Agents, Skills & Tools
- Cloud Architecture
- Programming
- Tech Updates
- Creator Insights
- Certifications

`Certifications` is only rendered on Mondays. Categories with fewer than `MIN_ARTICLES_PER_CATEGORY` relevant articles are skipped.

## AI Agents, Skills & Tools

The display category `AI Agents, Skills & Tools` uses the legacy internal JSON key `agentic_ai` for backward compatibility with `CategorizedSummary`, OpenAI response parsing, and Discord formatting.

This category covers articles whose primary subject is autonomous or semi-autonomous AI agents, AI coding agents, agent products, agent frameworks, orchestration, multi-agent systems, agent skills, plugins, MCP servers and tooling, tool use, function calling for agents, memory and context management, planning, reflection, browser or computer use, human-in-the-loop workflows, evaluation, observability, security, governance, deployment, or infrastructure specifically built for agent workloads.

Include articles when the main topic is:

- An agent that can plan or execute tasks.
- An AI coding agent or autonomous developer tool.
- A framework for building or orchestrating agents.
- An agent skill, plugin, MCP server, or agent tool.
- Memory, planning, reflection, tool selection, browser use, or computer use for agents.
- A multi-agent system.
- Evaluation, observability, security, governance, or deployment specifically for AI agents.

Exclude articles when they only discuss:

- A foundation model without agent capability.
- A simple chatbot without planning or tool use.
- Basic text generation or summarization.
- A programming language release.
- A developer tool without autonomous or semi-autonomous behavior.
- A normal cloud outage or generic cloud infrastructure update.
- Generic AI funding or acquisition news where agent technology is not the primary subject.
- A RAG framework where agents are only an optional or briefly mentioned feature.
- MCP mentioned in passing rather than as the article's main topic.

Known entity hints such as Claude Code, Codex CLI, Cursor Agent, Replit Agent, Devin, GitHub Copilot Agents, LangGraph, CrewAI, AutoGen, OpenAI Agents SDK, Semantic Kernel, PydanticAI, smolagents, Agno, Mastra, MCP, and Model Context Protocol are hints only, not a whitelist. New agents and tools should still be recognized semantically from their capabilities and article context.

Before delivery to Discord, articles are deduplicated across categories by normalized URL. Category priority is `AI Agents, Skills & Tools`, `Cloud Architecture`, `Programming`, `Tech Updates`, `Creator Insights`, then `Certifications`.

## Limitations

- RSS parsing is intentionally lightweight and dependency-free. It handles common RSS and Atom shapes but is not a full XML parser.
- GitHub Trending is left as a TODO placeholder because scraping `github.com/trending` is brittle.
- Product Hunt is left as a TODO placeholder until a token and stable GraphQL query are configured.
- Every.to RSS is left as a TODO placeholder because Every documents RSS as a personal subscriber feed rather than a stable public feed.
- The bot caps processing with `MAX_ARTICLES_PER_SESSION` to keep OpenAI and Discord usage predictable.
- AI category classification still depends on the OpenAI model following the prompt, and URL deduplication does not detect duplicate stories with different URLs.

## Add a New Source

1. Add a collector function that returns `Vec<Article>`.
2. Normalize every item into `Article` with:
   - `source_name`
   - `source_feed`
   - `title`
   - `url`
   - `summary`
3. Call the collector from `collect_source_articles`.
4. Add a `log_source_count("Source Name", articles.len())` debug log.
5. Prefer RSS/API sources over scraping.
6. If a source has no reliable API/RSS, add a placeholder collector with a clear TODO instead of fragile scraping.

## Local Testing

```bash
cargo fmt
cargo check --locked
cargo build --locked
cargo run --locked
```

The app reads local secrets from `.env` via `dotenv`.

For safe runtime validation of the AI agent category without sending Discord messages, run the local fixture in dry-run mode:

```bash
DRY_RUN=true TECH_RADAR_FIXTURE=phase3 cargo run --locked
```

This uses 12 local fixture articles, calls the OpenAI analysis step, builds the Discord payload, prints safe category/embed counts, and skips Discord delivery and `processed_urls.txt` updates.
