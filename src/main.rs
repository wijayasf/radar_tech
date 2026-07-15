use chrono::{Datelike, Local, Weekday};
use dotenv::dotenv;
use reqwest::Client as HttpClient;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    env, fs,
    io::{self, Write},
    process,
};
use tokio::time::{Duration, sleep};

const PROCESSED_URLS_FILE: &str = "processed_urls.txt";
const ARTICLE_TEXT_LIMIT: usize = 5_000;
const MAX_ARTICLES_PER_SESSION: usize = 30;
const MIN_ARTICLES_PER_CATEGORY: usize = 3;
const MAX_ARTICLES_PER_CATEGORY: usize = 5;
const SOURCE_DELAY_SECONDS: u64 = 2;
const OPENAI_RESPONSES_URL: &str = "https://api.openai.com/v1/responses";
const OPENAI_SYSTEM_PROMPT: &str = r#"Anda adalah analis teknologi untuk Innovation Engineer. Analisis semua artikel yang diberikan sebagai satu batch, lalu kembalikan JSON saja.

Kategori JSON wajib: "agentic_ai", "architecture_ai", "programming", "tech_update", "creator_insights", dan "certifications".

Catatan kategori: "agentic_ai" adalah key JSON internal untuk display category "AI Agents, Skills & Tools".

Setiap kategori harus berupa objek:
{
  "summary": "ringkasan eksekutif singkat yang mensintesis artikel teratas",
  "key_themes": ["tema 1", "tema 2", "tema 3"],
  "articles": [
    {
      "title": "...",
      "source": "...",
      "url": "...",
      "why_this_matters": "1 kalimat kenapa ini penting",
      "strategic_implication": "opsional: implikasi untuk Innovation Engineer"
    }
  ]
}

Definisi kategori "agentic_ai" / "AI Agents, Skills & Tools":
- Masukkan artikel jika topik utamanya membahas autonomous atau semi-autonomous AI agents, AI coding agents, agent products, agent frameworks, agent orchestration, multi-agent systems, agent skills, plugins, MCP servers dan MCP tooling, tool use dan function calling untuk agents, agent memory dan context management, planning dan reflection, browser use dan computer use, human-in-the-loop agent workflows, agent evaluation dan observability, agent security dan governance, atau agent deployment dan infrastructure khusus agent workloads.
- Known entity hints bukan whitelist. Gunakan nama berikut sebagai sinyal pendukung saja: Ponytail, Caveman, Hermes, .paul, Claude Code, Codex CLI, Cursor Agent, Replit Agent, Devin, GitHub Copilot Agents, LangGraph, CrewAI, AutoGen, OpenAI Agents SDK, Semantic Kernel, PydanticAI, smolagents, Agno, Mastra, MCP, Model Context Protocol, MCP Server, agent skills, agent plugins, tool-use frameworks, agent memory, dan agent evaluation.
- Classifier harus tetap mengenali agent, framework, tool, skill, atau protocol baru berdasarkan capability dan konteks artikel meskipun nama entity tidak ada dalam daftar.

Inclusion rules untuk "agentic_ai":
- Topik utama adalah agent yang dapat merencanakan atau menjalankan task.
- AI coding agent atau autonomous developer tool.
- Framework untuk membangun atau mengorkestrasi agent.
- Agent skill, plugin, MCP server, atau agent tool.
- Memory, planning, reflection, tool selection, browser use, atau computer use untuk agent.
- Multi-agent system.
- Evaluation, observability, security, governance, atau deployment khusus AI agents.

Exclusion rules untuk "agentic_ai":
- Jangan masukkan model LLM baru tanpa agent capability.
- Jangan masukkan chatbot sederhana tanpa planning atau tool use.
- Jangan masukkan text generation atau summarization biasa.
- Jangan masukkan programming language release.
- Jangan masukkan developer tool tanpa autonomous atau semi-autonomous behavior.
- Jangan masukkan cloud outage biasa atau cloud infrastructure umum.
- Jangan masukkan generic AI funding atau acquisition news, kecuali agent technology adalah topik utama.
- Jangan masukkan RAG framework yang hanya menyebut agent sebagai fitur opsional.
- Jangan masukkan MCP yang hanya disebut sekilas dan bukan topik utama.

Classification examples:
- New AI coding agent that plans and executes repository changes -> agentic_ai.
- New MCP server that allows agents to interact with databases -> agentic_ai.
- Framework for multi-agent orchestration -> agentic_ai.
- Marketplace for reusable agent skills -> agentic_ai.
- Security platform specifically designed for autonomous agents -> agentic_ai.
- Newly released product not present in known entity hints, but capable of planning, tool use, and autonomous execution -> agentic_ai.
- New foundation model without agent capability -> not agentic_ai.
- Cloud provider outage -> architecture_ai or tech_update.
- Programming language version release -> programming.
- Simple chatbot without planning or tool use -> not agentic_ai.
- Generic AI startup funding -> tech_update unless agent technology is the primary subject.
- IDE with autocomplete only -> programming.
- IDE that autonomously plans, edits, tests, and iterates over a repository -> agentic_ai.
- Cloud platform where an agent integration is only mentioned briefly -> architecture_ai.
- Cloud platform launching infrastructure specifically for agent workloads -> agentic_ai or architecture_ai based on the primary subject.

Ranking dan filtering:
- Gunakan prinsip "Primary subject wins": artikel harus dimasukkan berdasarkan topik utamanya, bukan hanya karena terdapat satu keyword terkait agent.
- Prioritaskan relevansi terhadap AI Agents, Skills & Tools, MCP, AI engineering, Enterprise AI, LLM tooling, RAG, evaluation, AI infrastructure, cloud architecture, dan developer productivity.
- Urutkan berdasarkan relevansi AI/engineering, freshness, kualitas sumber, dan strategic importance untuk Innovation Engineer.
- Targetkan 5 artikel per kategori.
- Jika hanya tersedia 3 atau 4 artikel relevan, kembalikan 3 atau 4.
- Jika kategori memiliki kurang dari 3 artikel relevan, kembalikan "articles": [], "summary": "", dan "key_themes": [].
- Jangan duplikasi artikel yang sama di lebih dari satu kategori jika URL sama; pilih kategori paling sesuai dengan topik utama.
- Jangan dump judul RSS mentah; sintetis dan ranking konten.
- Untuk certifications, isi hanya jika ada ujian, pelatihan, voucher, cohort, program belajar, atau sertifikasi yang relevan dengan ASEAN/Indonesia.
- Gunakan URL dan source asli dari input.
- Jangan sertakan teks lain selain JSON."#;

#[derive(Debug, Clone)]
struct Article {
    source_name: String,
    source_feed: String,
    title: String,
    url: String,
    summary: String,
}

#[derive(Debug, Deserialize)]
struct CategorizedSummary {
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    // Legacy/internal key for the "AI Agents, Skills & Tools" display category.
    agentic_ai: CategoryDigest,
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    architecture_ai: CategoryDigest,
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    programming: CategoryDigest,
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    tech_update: CategoryDigest,
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    creator_insights: CategoryDigest,
    #[serde(default, deserialize_with = "deserialize_null_object_as_default")]
    certifications: CategoryDigest,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct CategoryDigest {
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    summary: String,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    key_themes: Vec<String>,
    #[serde(default, deserialize_with = "deserialize_null_as_default")]
    articles: Vec<ArticleInfo>,
}

#[derive(Debug, Clone, Deserialize)]
struct ArticleInfo {
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    title: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    source: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    url: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    why_this_matters: String,
    #[serde(default, deserialize_with = "deserialize_null_string_as_default")]
    strategic_implication: String,
}

fn deserialize_null_as_default<'de, D, T>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_null_string_as_default<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Ok(Option::<String>::deserialize(deserializer)?.unwrap_or_default())
}

fn deserialize_null_object_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

#[tokio::main]
async fn main() {
    dotenv().ok();

    if let Err(error) = run().await {
        eprintln!("Error: {error}");
        process::exit(0);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    println!("Workflow started");

    let openai_api_key = read_env("OPENAI_API_KEY")?;
    let dry_run = env_flag_enabled("DRY_RUN");
    let discord_webhook_url = if dry_run {
        String::new()
    } else {
        read_env("DISCORD_WEBHOOK_URL")?
    };
    let runtime_fixture_articles = runtime_validation_fixture_articles();
    let processed_urls = read_processed_urls(PROCESSED_URLS_FILE)?;
    let http_client = HttpClient::new();

    let collected_articles = if let Some(articles) = runtime_fixture_articles {
        println!(
            "Runtime validation fixture enabled with {} articles.",
            articles.len()
        );
        articles
    } else {
        collect_source_articles(&http_client).await?
    };
    let collected_articles_count = collected_articles.len();
    let mut new_articles = Vec::new();

    for article in collected_articles {
        if !dry_run && processed_urls.contains(&article.url) {
            println!(
                "Lewati artikel yang sudah diproses dari {}: {}",
                article.source_name, article.url
            );
            continue;
        }

        new_articles.push(article);

        if new_articles.len() >= MAX_ARTICLES_PER_SESSION {
            println!(
                "Batas {} artikel baru per sesi tercapai.",
                MAX_ARTICLES_PER_SESSION
            );
            break;
        }
    }

    println!("Number of collected articles: {collected_articles_count}");
    println!("Number of filtered articles: {}", new_articles.len());

    if new_articles.is_empty() {
        println!("Tidak ada artikel baru untuk diproses.");
        if dry_run {
            println!(
                "DRY_RUN enabled: Discord fallback message was built but not sent, and processed_urls.txt was not updated."
            );
            log_final_summary(collected_articles_count, new_articles.len(), 0, "dry-run");
            return Ok(());
        }

        match send_discord_text(
            &http_client,
            &discord_webhook_url,
            "✅ Tech Radar test successful — workflow executed, but no new relevant updates were found.",
        )
        .await
        {
            Ok(()) => {
                println!("Discord delivery success");
                log_final_summary(collected_articles_count, new_articles.len(), 0, "success");
            }
            Err(error) => {
                eprintln!("Discord delivery failed: {error}");
                log_final_summary(collected_articles_count, new_articles.len(), 0, "failed");
                return Err(error);
            }
        }
        return Ok(());
    }

    let summary = match summarize_articles(&http_client, &openai_api_key, &new_articles).await {
        Ok(summary) => {
            println!("OpenAI analysis success");
            summary
        }
        Err(error) => {
            eprintln!("OpenAI analysis failure: {error}");
            return Err(error);
        }
    };
    let discord_embeds = format_discord_message(&summary);

    if discord_embeds.is_empty() {
        println!("No category reached minimum relevance threshold.");
        if dry_run {
            println!(
                "DRY_RUN enabled: Discord fallback message was built but not sent, and processed_urls.txt was not updated."
            );
            log_runtime_validation_summary(&summary, &discord_embeds);
            log_final_summary(
                collected_articles_count,
                new_articles.len(),
                new_articles.len(),
                "dry-run",
            );
            return Ok(());
        }

        match send_discord_text(
            &http_client,
            &discord_webhook_url,
            "✅ Tech Radar test successful — workflow executed, but no category had at least 3 relevant updates.",
        )
        .await
        {
            Ok(()) => {
                println!("Discord delivery success");
                log_final_summary(
                    collected_articles_count,
                    new_articles.len(),
                    new_articles.len(),
                    "success",
                );
            }
            Err(error) => {
                eprintln!("Discord delivery failed: {error}");
                log_final_summary(
                    collected_articles_count,
                    new_articles.len(),
                    new_articles.len(),
                    "failed",
                );
                return Err(error);
            }
        }
        append_processed_urls(PROCESSED_URLS_FILE, &new_articles)?;
        return Ok(());
    }

    if dry_run {
        println!(
            "DRY_RUN enabled: Discord payload was built but not sent, and processed_urls.txt was not updated."
        );
        log_runtime_validation_summary(&summary, &discord_embeds);
        log_final_summary(
            collected_articles_count,
            new_articles.len(),
            new_articles.len(),
            "dry-run",
        );
        return Ok(());
    }

    match send_to_discord(&http_client, &discord_webhook_url, &discord_embeds).await {
        Ok(()) => {
            println!("Discord delivery success");
            log_final_summary(
                collected_articles_count,
                new_articles.len(),
                new_articles.len(),
                "success",
            );
        }
        Err(error) => {
            eprintln!("Discord delivery failed: {error}");
            log_final_summary(
                collected_articles_count,
                new_articles.len(),
                new_articles.len(),
                "failed",
            );
            return Err(error);
        }
    }
    append_processed_urls(PROCESSED_URLS_FILE, &new_articles)?;

    println!(
        "Berhasil mengirim ringkasan dan menyimpan {} URL artikel baru.",
        new_articles.len()
    );

    Ok(())
}

async fn collect_source_articles(
    http_client: &HttpClient,
) -> Result<Vec<Article>, Box<dyn std::error::Error>> {
    let mut articles = Vec::new();

    articles.extend(collect_hacker_news(http_client).await);

    let rss_sources = rss_sources();

    for (index, source) in rss_sources.iter().enumerate() {
        let source_articles = collect_rss_source(http_client, source).await;
        log_source_count(source.name, source_articles.len());
        articles.extend(source_articles);

        if index + 1 < rss_sources.len() {
            sleep(Duration::from_secs(SOURCE_DELAY_SECONDS)).await;
        }
    }

    articles.extend(collect_github_trending_placeholder());
    articles.extend(collect_product_hunt_placeholder());
    articles.extend(collect_every_ai_placeholder());

    Ok(articles)
}

struct RssSource {
    name: &'static str,
    url: &'static str,
}

fn rss_sources() -> Vec<RssSource> {
    vec![
        RssSource {
            name: "TLDR AI",
            url: "https://ai.tldr.tech/rss",
        },
        RssSource {
            name: "The Rundown AI",
            url: "https://www.therundown.ai/feed",
        },
        RssSource {
            name: "The New Stack",
            url: "https://thenewstack.io/feed/",
        },
        RssSource {
            name: "InfoQ",
            url: "https://www.infoq.com/feed/",
        },
        RssSource {
            name: "Lenny's Newsletter",
            url: "https://www.lennysnewsletter.com/feed",
        },
    ]
}

async fn collect_hacker_news(http_client: &HttpClient) -> Vec<Article> {
    const SOURCE_NAME: &str = "Hacker News";
    const TOP_STORIES_URL: &str = "https://hacker-news.firebaseio.com/v0/topstories.json";
    const ITEM_LIMIT: usize = 10;

    let result = async {
        let story_ids = http_client
            .get(TOP_STORIES_URL)
            .send()
            .await?
            .error_for_status()?
            .json::<Vec<u64>>()
            .await?;
        let mut articles = Vec::new();

        for story_id in story_ids.into_iter().take(ITEM_LIMIT) {
            let item_url = format!("https://hacker-news.firebaseio.com/v0/item/{story_id}.json");
            let item = http_client
                .get(&item_url)
                .send()
                .await?
                .error_for_status()?
                .json::<HackerNewsItem>()
                .await?;

            let title = item
                .title
                .unwrap_or_else(|| "Untitled Hacker News item".to_string());
            let url = item
                .url
                .unwrap_or_else(|| format!("https://news.ycombinator.com/item?id={story_id}"));

            articles.push(Article {
                source_name: SOURCE_NAME.to_string(),
                source_feed: TOP_STORIES_URL.to_string(),
                title: clean_text(&title),
                url: clean_text(&url),
                summary: format!(
                    "Hacker News top story with score {}.",
                    item.score.unwrap_or(0)
                ),
            });
        }

        Ok::<Vec<Article>, Box<dyn std::error::Error>>(articles)
    }
    .await;

    match result {
        Ok(articles) => {
            log_source_count(SOURCE_NAME, articles.len());
            articles
        }
        Err(error) => {
            log_source_failure(SOURCE_NAME, &error.to_string());
            Vec::new()
        }
    }
}

#[derive(Debug, Deserialize)]
struct HackerNewsItem {
    title: Option<String>,
    url: Option<String>,
    score: Option<u64>,
}

async fn collect_rss_source(http_client: &HttpClient, source: &RssSource) -> Vec<Article> {
    match fetch_rss_feed(http_client, source.url).await {
        Ok(feed_content) => parse_feed_articles(source.name, source.url, &feed_content),
        Err(error) => {
            log_source_failure(source.name, &error.to_string());
            Vec::new()
        }
    }
}

fn collect_github_trending_placeholder() -> Vec<Article> {
    // TODO: GitHub Trending does not expose a stable official RSS/API endpoint.
    // Add a small, tested HTML collector only if we accept scraping maintenance risk,
    // or replace this with a stable third-party feed we explicitly trust.
    log_source_count("GitHub Trending", 0);
    Vec::new()
}

fn collect_product_hunt_placeholder() -> Vec<Article> {
    // TODO: Product Hunt requires an API token and GraphQL query configuration.
    // Implement this collector once PRODUCT_HUNT_TOKEN is configured in GitHub
    // Actions secrets and the selected query shape is locked down.
    match env::var("PRODUCT_HUNT_TOKEN") {
        Ok(_) => eprintln!(
            "Product Hunt collector TODO: PRODUCT_HUNT_TOKEN is set, but API collector is not implemented yet."
        ),
        Err(_) => eprintln!("Product Hunt collector skipped: PRODUCT_HUNT_TOKEN is not set."),
    }
    log_source_count("Product Hunt", 0);
    Vec::new()
}

fn collect_every_ai_placeholder() -> Vec<Article> {
    // TODO: Every.to documents RSS as a personal subscriber feed, not a stable
    // public source URL. Add EVERY_AI_RSS_URL support if a user-specific feed is
    // provided via environment variable.
    match env::var("EVERY_AI_RSS_URL") {
        Ok(_) => eprintln!(
            "Every.to AI collector TODO: EVERY_AI_RSS_URL is set, but private feed support is not implemented yet."
        ),
        Err(_) => eprintln!("Every.to AI collector skipped: EVERY_AI_RSS_URL is not set."),
    }
    log_source_count("Every.to AI", 0);
    Vec::new()
}

fn log_source_count(source_name: &str, count: usize) {
    println!("{source_name} collected {count} items");
}

fn log_source_failure(source_name: &str, error: &str) {
    eprintln!("{source_name} failed: {error}");
}

fn log_final_summary(
    total_collected: usize,
    total_filtered: usize,
    total_analyzed: usize,
    discord_status: &str,
) {
    println!("Final summary: total collected = {total_collected}");
    println!("Final summary: total filtered = {total_filtered}");
    println!("Final summary: total analyzed = {total_analyzed}");
    println!("Final summary: discord delivery status = {discord_status}");
}

fn log_runtime_validation_summary(summary: &CategorizedSummary, embeds: &[Value]) {
    println!("Runtime validation category counts:");
    println!("agentic_ai = {}", summary.agentic_ai.articles.len());
    println!(
        "architecture_ai = {}",
        summary.architecture_ai.articles.len()
    );
    println!("programming = {}", summary.programming.articles.len());
    println!("tech_update = {}", summary.tech_update.articles.len());
    println!(
        "creator_insights = {}",
        summary.creator_insights.articles.len()
    );
    println!("certifications = {}", summary.certifications.articles.len());
    println!("Discord embeds built = {}", embeds.len());

    for embed in embeds {
        if let Some(title) = embed.get("title").and_then(Value::as_str) {
            println!("Discord embed title: {title}");
        }
    }
}

fn env_flag_enabled(key: &str) -> bool {
    env::var(key)
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn runtime_validation_fixture_articles() -> Option<Vec<Article>> {
    match env::var("TECH_RADAR_FIXTURE") {
        Ok(value) if value.trim().eq_ignore_ascii_case("phase3") => {
            Some(phase3_runtime_validation_articles())
        }
        _ => None,
    }
}

fn phase3_runtime_validation_articles() -> Vec<Article> {
    vec![
        fixture_article(
            "Ponytail introduces reusable skills for coding agents",
            "https://example.com/phase3/ponytail-skills",
            "Ponytail launched reusable skills that coding agents can select, execute, and share across repository workflows.",
        ),
        fixture_article(
            "Caveman launches autonomous repository planning",
            "https://example.com/phase3/caveman-planning",
            "Caveman can inspect a repository, create an implementation plan, edit files, run checks, and iterate on failures.",
        ),
        fixture_article(
            "A new MCP server connects agents to PostgreSQL",
            "https://example.com/phase3/postgres-mcp",
            "The MCP server exposes PostgreSQL database tools so agents can query schema, inspect data, and call database operations safely.",
        ),
        fixture_article(
            "LangGraph introduces multi-agent orchestration",
            "https://example.com/phase3/langgraph-orchestration",
            "LangGraph added orchestration primitives for coordinating planner, executor, reviewer, and evaluator agents.",
        ),
        fixture_article(
            "A marketplace launches reusable agent skills",
            "https://example.com/phase3/agent-skill-marketplace",
            "The marketplace distributes reusable agent skills and plugins for browser use, code editing, memory, and tool selection.",
        ),
        fixture_article(
            "Unnamed product plans tasks, calls tools, executes changes, and evaluates output",
            "https://example.com/phase3/unnamed-agent-tool",
            "A newly released product can break down tasks, call tools, execute changes in external systems, and evaluate its own results.",
        ),
        fixture_article(
            "A new foundation model is released without agent capability",
            "https://example.com/phase3/foundation-model",
            "The model improves benchmark scores for text generation and summarization, but does not include planning, tools, memory, or autonomous execution.",
        ),
        fixture_article(
            "AWS experiences a regional outage",
            "https://example.com/phase3/aws-outage",
            "A regional cloud outage affected compute and storage services for several hours before recovery.",
        ),
        fixture_article(
            "Rust releases a new compiler version",
            "https://example.com/phase3/rust-compiler",
            "The Rust compiler release includes diagnostics improvements, stabilizations, and performance updates.",
        ),
        fixture_article(
            "A startup raises funding for a generic AI chatbot",
            "https://example.com/phase3/chatbot-funding",
            "The startup raised funding for a customer-support chatbot focused on answering questions without planning or tool use.",
        ),
        fixture_article(
            "An IDE adds autocomplete only",
            "https://example.com/phase3/ide-autocomplete",
            "The IDE added faster AI autocomplete suggestions, but it does not plan tasks, edit multiple files autonomously, or run tools.",
        ),
        fixture_article(
            "A RAG framework briefly mentions optional agent support",
            "https://example.com/phase3/rag-framework",
            "The framework focuses on retrieval pipelines and mentions optional agent support as a future integration.",
        ),
    ]
}

fn fixture_article(title: &str, url: &str, summary: &str) -> Article {
    Article {
        source_name: "Phase 3 Fixture".to_string(),
        source_feed: "local-runtime-validation".to_string(),
        title: title.to_string(),
        url: url.to_string(),
        summary: summary.to_string(),
    }
}

fn read_env(key: &str) -> Result<String, Box<dyn std::error::Error>> {
    match env::var(key) {
        Ok(value) if !value.trim().is_empty() => Ok(value),
        _ => Err(format!("Variabel environment {key} belum diisi.").into()),
    }
}

fn read_processed_urls(path: &str) -> io::Result<HashSet<String>> {
    match fs::read_to_string(path) {
        Ok(content) => Ok(content
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(ToOwned::to_owned)
            .collect()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(HashSet::new()),
        Err(error) => Err(error),
    }
}

fn append_processed_urls(path: &str, articles: &[Article]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;

    for article in articles {
        writeln!(file, "{}", article.url)?;
    }

    Ok(())
}

async fn fetch_rss_feed(
    http_client: &HttpClient,
    url: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let response = http_client.get(url).send().await?;

    if !response.status().is_success() {
        return Err(format!("Status HTTP {}", response.status()).into());
    }

    Ok(response.text().await?)
}

fn parse_feed_articles(source_name: &str, feed_url: &str, feed_content: &str) -> Vec<Article> {
    let mut articles = parse_rss_items(source_name, feed_url, feed_content);

    if articles.is_empty() {
        articles = parse_atom_entries(source_name, feed_url, feed_content);
    }

    articles
}

fn parse_rss_items(source_name: &str, feed_url: &str, feed_content: &str) -> Vec<Article> {
    find_blocks(feed_content, "<item", "</item>")
        .into_iter()
        .filter_map(|item| {
            let title =
                extract_tag_text(&item, "title").unwrap_or_else(|| "Tanpa judul".to_string());
            let url = extract_tag_text(&item, "link")?;
            let summary = extract_tag_text(&item, "description")
                .or_else(|| extract_tag_text(&item, "content:encoded"))
                .unwrap_or_default();

            Some(Article {
                source_name: source_name.to_string(),
                source_feed: feed_url.to_string(),
                title: clean_text(&title),
                url: clean_text(&url),
                summary: clean_text(&summary),
            })
        })
        .collect()
}

fn parse_atom_entries(source_name: &str, feed_url: &str, feed_content: &str) -> Vec<Article> {
    find_blocks(feed_content, "<entry", "</entry>")
        .into_iter()
        .filter_map(|entry| {
            let title =
                extract_tag_text(&entry, "title").unwrap_or_else(|| "Tanpa judul".to_string());
            let url = extract_atom_link(&entry)?;
            let summary = extract_tag_text(&entry, "summary")
                .or_else(|| extract_tag_text(&entry, "content"))
                .unwrap_or_default();

            Some(Article {
                source_name: source_name.to_string(),
                source_feed: feed_url.to_string(),
                title: clean_text(&title),
                url: clean_text(&url),
                summary: clean_text(&summary),
            })
        })
        .collect()
}

fn find_blocks(content: &str, start_marker: &str, end_marker: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut remaining = content;

    while let Some(start_index) = remaining.find(start_marker) {
        let after_start = &remaining[start_index..];

        if let Some(end_index) = after_start.find(end_marker) {
            let block_end = end_index + end_marker.len();
            blocks.push(after_start[..block_end].to_string());
            remaining = &after_start[block_end..];
        } else {
            break;
        }
    }

    blocks
}

fn extract_tag_text(content: &str, tag: &str) -> Option<String> {
    let start_marker = format!("<{tag}");
    let end_marker = format!("</{tag}>");
    let start_index = content.find(&start_marker)?;
    let after_start = &content[start_index..];
    let close_index = after_start.find('>')?;
    let text_start = close_index + 1;
    let text_end = after_start[text_start..].find(&end_marker)? + text_start;

    Some(after_start[text_start..text_end].to_string())
}

fn extract_atom_link(entry: &str) -> Option<String> {
    let mut remaining = entry;

    while let Some(link_index) = remaining.find("<link") {
        let after_link = &remaining[link_index..];
        let tag_end = after_link.find('>')?;
        let link_tag = &after_link[..tag_end];

        if let Some(href) = extract_attribute(link_tag, "href") {
            if link_tag.contains("rel=\"alternate\"")
                || link_tag.contains("rel='alternate'")
                || !link_tag.contains("rel=")
            {
                return Some(href);
            }
        }

        remaining = &after_link[tag_end..];
    }

    None
}

fn extract_attribute(tag: &str, attribute: &str) -> Option<String> {
    let double_quote_marker = format!("{attribute}=\"");
    let single_quote_marker = format!("{attribute}='");

    if let Some(start_index) = tag.find(&double_quote_marker) {
        let value_start = start_index + double_quote_marker.len();
        let value_end = tag[value_start..].find('"')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    if let Some(start_index) = tag.find(&single_quote_marker) {
        let value_start = start_index + single_quote_marker.len();
        let value_end = tag[value_start..].find('\'')? + value_start;
        return Some(tag[value_start..value_end].to_string());
    }

    None
}

fn clean_text(value: &str) -> String {
    value
        .replace("<![CDATA[", "")
        .replace("]]>", "")
        .replace("&amp;", "&")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

async fn summarize_articles(
    http_client: &HttpClient,
    openai_api_key: &str,
    articles: &[Article],
) -> Result<CategorizedSummary, Box<dyn std::error::Error>> {
    let prompt = build_articles_prompt(articles);
    let request = json!({
        "model": "gpt-4o",
        "instructions": OPENAI_SYSTEM_PROMPT,
        "input": prompt,
        "text": {
            "format": {
                "type": "json_object"
            }
        }
    });

    let response = http_client
        .post(OPENAI_RESPONSES_URL)
        .bearer_auth(openai_api_key)
        .json(&request)
        .send()
        .await?;
    let status = response.status();
    let response_body = response.text().await?;

    if !status.is_success() {
        return Err(format!("OpenAI Responses API error {status}: {response_body}").into());
    }

    let response_json: Value = serde_json::from_str(&response_body)?;
    let raw_summary_json = extract_openai_response_text(&response_json)
        .ok_or("OpenAI Responses API tidak mengembalikan output teks.")?;

    println!("Raw JSON dari AI: {}", &raw_summary_json);

    let cleaned_json = strip_json_code_fence(&raw_summary_json);
    let summary: CategorizedSummary = serde_json::from_str(&cleaned_json)?;

    Ok(summary)
}

fn extract_openai_response_text(response: &Value) -> Option<String> {
    if let Some(output_text) = response.get("output_text").and_then(Value::as_str) {
        return Some(output_text.to_string());
    }

    let mut text_parts = Vec::new();

    for output_item in response.get("output")?.as_array()? {
        let Some(content_items) = output_item.get("content").and_then(Value::as_array) else {
            continue;
        };

        for content_item in content_items {
            if let Some(text) = content_item.get("text").and_then(Value::as_str) {
                text_parts.push(text);
            }
        }
    }

    if text_parts.is_empty() {
        None
    } else {
        Some(text_parts.join(""))
    }
}

fn build_articles_prompt(articles: &[Article]) -> String {
    let mut prompt = String::from(
        "Artikel baru yang perlu dianalisis dan diranking. Kembalikan hasil sebagai JSON sesuai instructions:\n\n",
    );

    for (index, article) in articles.iter().enumerate() {
        let truncated_summary = truncate_text(&article.summary, ARTICLE_TEXT_LIMIT);

        prompt.push_str(&format!(
            "{}. Judul: {}\nURL: {}\nSumber: {}\nSumber RSS/API: {}\nCuplikan: {}\n\n",
            index + 1,
            article.title,
            article.url,
            article.source_name,
            article.source_feed,
            truncated_summary
        ));
    }

    prompt
}

fn truncate_text(value: &str, max_chars: usize) -> String {
    let mut chars = value.chars();
    let mut truncated = chars.by_ref().take(max_chars).collect::<String>();

    if chars.next().is_some() {
        truncated.push_str("...");
    }

    truncated
}

fn format_discord_message(summary: &CategorizedSummary) -> Vec<Value> {
    let mut categories = vec![
        ("AI Agents, Skills & Tools", &summary.agentic_ai, false),
        ("Cloud Architecture", &summary.architecture_ai, false),
        ("Programming", &summary.programming, false),
        ("Tech Updates", &summary.tech_update, false),
        ("Creator Insights", &summary.creator_insights, true),
    ];

    if Local::now().weekday() == Weekday::Mon {
        categories.push(("Certifications", &summary.certifications, false));
    }

    let mut embeds = Vec::new();
    let mut seen_urls = HashSet::new();

    for (category_name, digest, is_creator_insights) in categories {
        let deduplicated_digest = filter_unique_category_articles(digest, &mut seen_urls);

        if deduplicated_digest.articles.len() < MIN_ARTICLES_PER_CATEGORY {
            println!(
                "Skipping {category_name}: only {} relevant articles",
                deduplicated_digest.articles.len()
            );
            continue;
        }

        let description =
            format_category_digest_description(&deduplicated_digest, is_creator_insights);
        let footer_text = if is_creator_insights {
            "Tech Radar • Creator Insights Digest"
        } else {
            "Tech Radar • Morning Digest"
        };

        embeds.push(json!({
            "title": format!("📰 LINE TECH NEWS | {category_name}"),
            "color": category_color(category_name),
            "description": description,
            "footer": {
                "text": footer_text
            }
        }));
    }

    embeds
}

fn category_color(category_name: &str) -> u32 {
    match category_name {
        "AI Agents, Skills & Tools" => 46_714,
        "Agentic AI" => 46_714,
        "Cloud Architecture" => 39_423,
        "Programming" => 10_182_117,
        "Tech Updates" => 15_817_653,
        "Creator Insights" => 16_763_904,
        _ => 16_777_215,
    }
}

fn filter_unique_category_articles(
    digest: &CategoryDigest,
    seen_urls: &mut HashSet<String>,
) -> CategoryDigest {
    let mut unique_digest = digest.clone();
    unique_digest.articles = digest
        .articles
        .iter()
        .filter(|article| insert_unique_article_url(article, seen_urls))
        .cloned()
        .collect();

    unique_digest
}

fn insert_unique_article_url(article: &ArticleInfo, seen_urls: &mut HashSet<String>) -> bool {
    let normalized_url = normalize_url_for_deduplication(&article.url);

    if normalized_url.is_empty() {
        return true;
    }

    seen_urls.insert(normalized_url)
}

fn normalize_url_for_deduplication(url: &str) -> String {
    let without_fragment = url.trim().split('#').next().unwrap_or_default().trim();

    if without_fragment.len() <= "https://".len() {
        without_fragment.to_string()
    } else {
        without_fragment.trim_end_matches('/').to_string()
    }
}

fn strip_json_code_fence(value: &str) -> String {
    value
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim()
        .to_string()
}

fn format_category_digest_description(
    digest: &CategoryDigest,
    is_creator_insights: bool,
) -> String {
    let summary = if digest.summary.trim().is_empty() {
        "Belum ada ringkasan kategori yang cukup kuat."
    } else {
        digest.summary.trim()
    };
    let themes = format_key_themes(&digest.key_themes);
    let articles = digest
        .articles
        .iter()
        .take(MAX_ARTICLES_PER_CATEGORY)
        .map(|article| format_digest_article_item(article, is_creator_insights))
        .collect::<Vec<_>>()
        .join("\n\n");

    format!(
        "🧠 **Summary**\n{summary}\n\n🔥 **Key Themes**\n{themes}\n\n📰 **Top Updates**\n\n{articles}"
    )
}

fn format_key_themes(themes: &[String]) -> String {
    let selected_themes = themes
        .iter()
        .map(|theme| theme.trim())
        .filter(|theme| !theme.is_empty())
        .take(3)
        .map(|theme| format!("• {theme}"))
        .collect::<Vec<_>>();

    if selected_themes.is_empty() {
        "• No dominant themes identified".to_string()
    } else {
        selected_themes.join("\n")
    }
}

fn format_digest_article_item(article: &ArticleInfo, is_creator_insights: bool) -> String {
    let title = article_title(article, is_creator_insights);
    let source = if article.source.trim().is_empty() {
        "Unknown source"
    } else {
        article.source.trim()
    };
    let why_this_matters = if article.why_this_matters.trim().is_empty() {
        "Relevant to AI and engineering strategy."
    } else {
        article.why_this_matters.trim()
    };
    let strategic_implication = article.strategic_implication.trim();
    let link_label = if is_creator_insights {
        "Read Thread"
    } else {
        "Read Article"
    };
    let title_line = if article.url.trim().is_empty() {
        format!("**{title}**")
    } else {
        format!(
            "**{title}** - [{link_label}]({})",
            sanitize_markdown_url(&article.url)
        )
    };

    if strategic_implication.is_empty() {
        format!("{title_line}\nSource: {source}\nWhy this matters: {why_this_matters}")
    } else {
        format!(
            "{title_line}\nSource: {source}\nWhy this matters: {why_this_matters}\nStrategic implication: {strategic_implication}"
        )
    }
}

fn article_title(article: &ArticleInfo, is_creator_insights: bool) -> String {
    if is_creator_insights {
        let hook_source = first_line_or_sentence(article.why_this_matters.trim());
        let hook = hook_source.chars().take(60).collect::<String>();

        if !hook.trim().is_empty() {
            return format!("{}...", hook.trim());
        }
    }

    if article.title.trim().is_empty() {
        "Untitled update".to_string()
    } else {
        article.title.trim().to_string()
    }
}

fn first_line_or_sentence(value: &str) -> &str {
    let trimmed = value.trim();
    let first_line = trimmed.lines().next().unwrap_or(trimmed).trim();

    match first_line.find('.') {
        Some(sentence_end) if sentence_end > 0 => first_line[..sentence_end].trim(),
        _ => first_line,
    }
}

fn sanitize_markdown_url(url: &str) -> String {
    url.trim().replace('(', "%28").replace(')', "%29")
}

async fn send_to_discord(
    http_client: &HttpClient,
    webhook_url: &str,
    embeds: &[Value],
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = serde_json::to_string(&json!({
        "embeds": embeds
    }))?;

    let response = http_client
        .post(webhook_url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!(
            "Gagal mengirim ringkasan ke Discord. Status HTTP: {}",
            response.status()
        )
        .into());
    }

    Ok(())
}

async fn send_discord_text(
    http_client: &HttpClient,
    webhook_url: &str,
    content: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let payload = serde_json::to_string(&json!({
        "content": content
    }))?;

    let response = http_client
        .post(webhook_url)
        .header("Content-Type", "application/json")
        .body(payload)
        .send()
        .await?;

    if !response.status().is_success() {
        return Err(format!(
            "Gagal mengirim pesan fallback ke Discord. Status HTTP: {}",
            response.status()
        )
        .into());
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_article(title: &str, url: &str) -> ArticleInfo {
        ArticleInfo {
            title: title.to_string(),
            source: "Test Source".to_string(),
            url: url.to_string(),
            why_this_matters: "Test relevance.".to_string(),
            strategic_implication: "Test implication.".to_string(),
        }
    }

    fn test_digest(summary: &str, articles: Vec<ArticleInfo>) -> CategoryDigest {
        CategoryDigest {
            summary: summary.to_string(),
            key_themes: vec![
                "Agent Development".to_string(),
                "MCP & Tool Use".to_string(),
                "Agent Security".to_string(),
            ],
            articles,
        }
    }

    fn default_summary() -> CategorizedSummary {
        CategorizedSummary {
            agentic_ai: CategoryDigest::default(),
            architecture_ai: CategoryDigest::default(),
            programming: CategoryDigest::default(),
            tech_update: CategoryDigest::default(),
            creator_insights: CategoryDigest::default(),
            certifications: CategoryDigest::default(),
        }
    }

    fn embeds_as_text(embeds: &[Value]) -> String {
        serde_json::to_string(embeds).expect("embeds should serialize")
    }

    #[test]
    fn deserializes_legacy_agentic_ai_key() {
        let payload = r#"{
            "agentic_ai": {
                "summary": "Agent tools are accelerating.",
                "key_themes": ["Agent Development"],
                "articles": [
                    {
                        "title": "New coding agent",
                        "source": "Test Source",
                        "url": "https://example.com/agent",
                        "why_this_matters": "It can plan and execute changes.",
                        "strategic_implication": "Teams need agent evaluation."
                    }
                ]
            }
        }"#;

        let summary: CategorizedSummary =
            serde_json::from_str(payload).expect("legacy key should deserialize");

        assert_eq!(summary.agentic_ai.summary, "Agent tools are accelerating.");
        assert_eq!(summary.agentic_ai.articles.len(), 1);
        assert!(summary.programming.articles.is_empty());
    }

    #[test]
    fn discord_embed_uses_ai_agents_display_name() {
        let mut summary = default_summary();
        summary.agentic_ai = test_digest(
            "Agent products, frameworks, and skills are moving quickly.",
            vec![
                test_article("Agent story 1", "https://example.com/agent-1"),
                test_article("Agent story 2", "https://example.com/agent-2"),
                test_article("Agent story 3", "https://example.com/agent-3"),
            ],
        );

        let embeds = format_discord_message(&summary);
        let first_title = embeds[0]["title"].as_str().expect("title should exist");

        assert!(first_title.contains("AI Agents, Skills & Tools"));
        assert!(!first_title.contains("Agentic AI"));
    }

    #[test]
    fn ai_agents_category_color_is_not_fallback() {
        assert_ne!(
            category_color("AI Agents, Skills & Tools"),
            category_color("Unknown Category")
        );
        assert_eq!(
            category_color("AI Agents, Skills & Tools"),
            category_color("Agentic AI")
        );
    }

    #[test]
    fn cross_category_deduplication_keeps_highest_priority_category() {
        let mut summary = default_summary();
        summary.agentic_ai = test_digest(
            "Agents first.",
            vec![
                test_article("Agent Deep Dive", "https://example.com/shared-agent"),
                test_article("Agent unique 1", "https://example.com/agent-unique-1"),
                test_article("Agent unique 2", "https://example.com/agent-unique-2"),
            ],
        );
        summary.programming = test_digest(
            "Programming duplicate.",
            vec![
                test_article(
                    "Programming Duplicate",
                    " https://example.com/shared-agent/ ",
                ),
                test_article("Programming unique 1", "https://example.com/programming-1"),
                test_article("Programming unique 2", "https://example.com/programming-2"),
            ],
        );
        summary.tech_update = test_digest(
            "Tech duplicate.",
            vec![
                test_article("Tech Duplicate", "https://example.com/shared-agent#section"),
                test_article("Tech unique 1", "https://example.com/tech-1"),
                test_article("Tech unique 2", "https://example.com/tech-2"),
            ],
        );

        let rendered = embeds_as_text(&format_discord_message(&summary));

        assert!(rendered.contains("AI Agents, Skills & Tools"));
        assert!(rendered.contains("Agent Deep Dive"));
        assert!(!rendered.contains("Programming Duplicate"));
        assert!(!rendered.contains("Tech Duplicate"));
        assert_eq!(rendered.matches("shared-agent").count(), 1);
    }

    #[test]
    fn unique_articles_remain_in_their_categories() {
        let mut summary = default_summary();
        summary.agentic_ai = test_digest(
            "Agents.",
            vec![
                test_article("Agent unique 1", "https://example.com/agent-a"),
                test_article("Agent unique 2", "https://example.com/agent-b"),
                test_article("Agent unique 3", "https://example.com/agent-c"),
            ],
        );
        summary.programming = test_digest(
            "Programming.",
            vec![
                test_article("Programming unique 1", "https://example.com/programming-a"),
                test_article("Programming unique 2", "https://example.com/programming-b"),
                test_article("Programming unique 3", "https://example.com/programming-c"),
            ],
        );

        let rendered = embeds_as_text(&format_discord_message(&summary));

        assert!(rendered.contains("AI Agents, Skills & Tools"));
        assert!(rendered.contains("Programming"));
        assert!(rendered.contains("Agent unique 1"));
        assert!(rendered.contains("Programming unique 1"));
    }

    #[test]
    fn normalizes_urls_for_safe_deduplication() {
        assert_eq!(
            normalize_url_for_deduplication(" https://example.com/path/ "),
            "https://example.com/path"
        );
        assert_eq!(
            normalize_url_for_deduplication("https://example.com/path#comments"),
            "https://example.com/path"
        );

        let mut seen_urls = HashSet::new();
        assert!(insert_unique_article_url(
            &test_article("Original", "https://example.com/path"),
            &mut seen_urls
        ));
        assert!(!insert_unique_article_url(
            &test_article("Duplicate", " https://example.com/path/#comments "),
            &mut seen_urls
        ));
    }

    #[test]
    fn phase3_runtime_fixture_contains_expected_article_inputs() {
        let articles = phase3_runtime_validation_articles();
        let titles = articles
            .iter()
            .map(|article| article.title.as_str())
            .collect::<Vec<_>>();

        assert_eq!(articles.len(), 12);
        assert!(titles.contains(&"Ponytail introduces reusable skills for coding agents"));
        assert!(titles.contains(&"Caveman launches autonomous repository planning"));
        assert!(titles.contains(&"A new MCP server connects agents to PostgreSQL"));
        assert!(titles.contains(&"LangGraph introduces multi-agent orchestration"));
        assert!(titles.contains(&"A marketplace launches reusable agent skills"));
        assert!(titles.contains(
            &"Unnamed product plans tasks, calls tools, executes changes, and evaluates output"
        ));
        assert!(titles.contains(&"A new foundation model is released without agent capability"));
        assert!(titles.contains(&"AWS experiences a regional outage"));
        assert!(titles.contains(&"Rust releases a new compiler version"));
        assert!(titles.contains(&"A startup raises funding for a generic AI chatbot"));
        assert!(titles.contains(&"An IDE adds autocomplete only"));
        assert!(titles.contains(&"A RAG framework briefly mentions optional agent support"));
    }

    #[test]
    fn phase3_fixture_payload_validates_display_deduplication_and_limits() {
        let mut summary = default_summary();
        summary.agentic_ai = test_digest(
            "Agent products, frameworks, skills, MCP, and tooling are expanding.",
            vec![
                test_article(
                    "Ponytail introduces reusable skills for coding agents",
                    "https://example.com/phase3/ponytail-skills",
                ),
                test_article(
                    "Caveman launches autonomous repository planning",
                    "https://example.com/phase3/caveman-planning",
                ),
                test_article(
                    "A new MCP server connects agents to PostgreSQL",
                    "https://example.com/phase3/postgres-mcp",
                ),
                test_article(
                    "LangGraph introduces multi-agent orchestration",
                    "https://example.com/phase3/langgraph-orchestration",
                ),
                test_article(
                    "A marketplace launches reusable agent skills",
                    "https://example.com/phase3/agent-skill-marketplace",
                ),
                test_article(
                    "Unnamed product plans tasks, calls tools, executes changes, and evaluates output",
                    "https://example.com/phase3/unnamed-agent-tool",
                ),
            ],
        );
        summary.programming = test_digest(
            "Programming and IDE updates.",
            vec![
                test_article(
                    "Rust releases a new compiler version",
                    "https://example.com/phase3/rust-compiler",
                ),
                test_article(
                    "An IDE adds autocomplete only",
                    "https://example.com/phase3/ide-autocomplete",
                ),
                test_article(
                    "Duplicate MCP story in programming",
                    "https://example.com/phase3/postgres-mcp#discussion",
                ),
            ],
        );
        summary.tech_update = test_digest(
            "General tech updates.",
            vec![
                test_article(
                    "A new foundation model is released without agent capability",
                    "https://example.com/phase3/foundation-model",
                ),
                test_article(
                    "A startup raises funding for a generic AI chatbot",
                    "https://example.com/phase3/chatbot-funding",
                ),
                test_article(
                    "Duplicate agent marketplace story",
                    " https://example.com/phase3/agent-skill-marketplace/ ",
                ),
            ],
        );

        let embeds = format_discord_message(&summary);
        let rendered = embeds_as_text(&embeds);
        let agent_embed = embeds
            .iter()
            .find(|embed| {
                embed["title"]
                    .as_str()
                    .unwrap_or_default()
                    .contains("AI Agents, Skills & Tools")
            })
            .expect("agent embed should render with at least three articles");
        let description = agent_embed["description"]
            .as_str()
            .expect("description should exist");

        assert_eq!(
            agent_embed["title"].as_str(),
            Some("📰 LINE TECH NEWS | AI Agents, Skills & Tools")
        );
        assert!(description.contains("🧠 **Summary**"));
        assert!(description.contains("🔥 **Key Themes**"));
        assert!(description.contains("📰 **Top Updates**"));
        assert!(description.contains("Source: Test Source"));
        assert!(description.contains("Why this matters: Test relevance."));
        assert!(description.contains("Strategic implication: Test implication."));
        assert_eq!(
            agent_embed["footer"]["text"].as_str(),
            Some("Tech Radar • Morning Digest")
        );
        assert!(!rendered.contains("LINE TECH NEWS | Agentic AI"));
        assert!(!rendered.contains("Duplicate MCP story in programming"));
        assert!(!rendered.contains("Duplicate agent marketplace story"));
        assert!(!description.contains("Unnamed product plans tasks"));
        assert_eq!(
            description.matches("Read Article").count(),
            MAX_ARTICLES_PER_CATEGORY
        );
    }

    #[test]
    fn existing_categories_keep_unique_articles_after_deduplication() {
        let mut summary = default_summary();
        summary.agentic_ai = test_digest(
            "Agents.",
            vec![
                test_article("Agent 1", "https://example.com/regression/agent-1"),
                test_article("Agent 2", "https://example.com/regression/agent-2"),
                test_article("Agent 3", "https://example.com/regression/agent-3"),
            ],
        );
        summary.architecture_ai = test_digest(
            "Cloud architecture.",
            vec![
                test_article("Cloud 1", "https://example.com/regression/cloud-1"),
                test_article("Cloud 2", "https://example.com/regression/cloud-2"),
                test_article("Cloud 3", "https://example.com/regression/cloud-3"),
            ],
        );
        summary.programming = test_digest(
            "Programming.",
            vec![
                test_article(
                    "Programming 1",
                    "https://example.com/regression/programming-1",
                ),
                test_article(
                    "Programming 2",
                    "https://example.com/regression/programming-2",
                ),
                test_article(
                    "Programming 3",
                    "https://example.com/regression/programming-3",
                ),
            ],
        );
        summary.tech_update = test_digest(
            "Tech.",
            vec![
                test_article("Tech 1", "https://example.com/regression/tech-1"),
                test_article("Tech 2", "https://example.com/regression/tech-2"),
                test_article("Tech 3", "https://example.com/regression/tech-3"),
            ],
        );
        summary.creator_insights = test_digest(
            "Creators.",
            vec![
                test_article("Creator 1", "https://example.com/regression/creator-1"),
                test_article("Creator 2", "https://example.com/regression/creator-2"),
                test_article("Creator 3", "https://example.com/regression/creator-3"),
            ],
        );
        summary.certifications = test_digest(
            "Certifications.",
            vec![
                test_article("Certification 1", "https://example.com/regression/cert-1"),
                test_article("Certification 2", "https://example.com/regression/cert-2"),
                test_article("Certification 3", "https://example.com/regression/cert-3"),
            ],
        );

        let rendered = embeds_as_text(&format_discord_message(&summary));

        assert!(rendered.contains("Cloud Architecture"));
        assert!(rendered.contains("Programming"));
        assert!(rendered.contains("Tech Updates"));
        assert!(rendered.contains("Creator Insights"));
        assert!(rendered.contains("Cloud 1"));
        assert!(rendered.contains("Programming 1"));
        assert!(rendered.contains("Tech 1"));
        assert!(rendered.contains("Creator"));

        if Local::now().weekday() == Weekday::Mon {
            assert!(rendered.contains("Certifications"));
        } else {
            assert!(!rendered.contains("Certifications"));
        }
    }
}
