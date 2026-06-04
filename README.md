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

## Limitations

- RSS parsing is intentionally lightweight and dependency-free. It handles common RSS and Atom shapes but is not a full XML parser.
- GitHub Trending is left as a TODO placeholder because scraping `github.com/trending` is brittle.
- Product Hunt is left as a TODO placeholder until a token and stable GraphQL query are configured.
- Every.to RSS is left as a TODO placeholder because Every documents RSS as a personal subscriber feed rather than a stable public feed.
- The bot caps processing with `MAX_ARTICLES_PER_SESSION` to keep OpenAI and Discord usage predictable.

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
